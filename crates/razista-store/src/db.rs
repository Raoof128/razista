//! The store handle, opening and recovery, repositories, preparations, jobs,
//! grants and retention.

use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use razista_core::policy::ActionGrant;
use razista_core::{
    BoundedVec, CallerId, ErrorCode, GrantId, JobId, JobStage, JobState, PreparedReview, RepoGrant,
    RepoId, Result, ReviewError, ReviewJob, ReviewRequest, Timestamp, validate_request,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::migrations::{self, MIGRATIONS, Migration};

/// Source of time, injectable so expiry and retention are testable.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// What the store can honestly say about confidentiality at rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AtRestProtection {
    /// The store does not encrypt, and has not verified volume encryption.
    Unverified { reason: String },
}

/// What crash recovery found and did when the store was opened.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Half-written files removed from `tmp/`.
    pub partial_files_removed: u64,
    /// Blob files with no committed metadata row, removed (never evidence).
    pub orphan_blobs_removed: u64,
    /// Committed artefacts whose bytes are missing. Reads of them fail.
    pub missing_blobs: u64,
}

/// Local retention defaults (design §18). Configurable; not a promise of
/// forensic erasure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Terminal job metadata, events and unpinned evidence.
    pub job_metadata_ms: u64,
    /// Artefacts no evidence references (raw logs, scratch output).
    pub raw_artifacts_ms: u64,
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            job_metadata_ms: 30 * DAY_MS,
            raw_artifacts_ms: 7 * DAY_MS,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RetentionReport {
    pub jobs_removed: u64,
    pub artifacts_removed: u64,
    pub bytes_freed: u64,
    /// Blob files whose row was removed but whose deletion failed.
    pub files_not_deleted: Vec<String>,
    /// Copies the engine cannot erase, stated rather than ignored.
    pub outside_control: Vec<String>,
}

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    /// Exclusive advisory lock on `root/LOCK`, held for the Store's lifetime.
    _lock: fs::File,
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
    recovery: RecoveryReport,
}

impl std::fmt::Debug for dyn Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Clock(now={})", self.now().0)
    }
}

// ---------- error helpers ------------------------------------------------------

pub(crate) fn store_err(e: impl Display) -> ReviewError {
    ReviewError::new(ErrorCode::StoreUnavailable, format!("store: {e}"))
}

/// The one answer for "absent" and "not yours", so ids leak nothing.
pub(crate) fn not_found_or_not_owned(what: &str) -> ReviewError {
    ReviewError::new(
        ErrorCode::PolicyDenied,
        format!("{what} not found or not owned by caller"),
    )
}

pub(crate) fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(store_err)
}

pub(crate) fn from_json<T: DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| store_err(format!("corrupt record: {e}")))
}

/// The serde name of a unit enum variant, e.g. `JobState::Completed` -> "completed".
pub(crate) fn enum_name<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value).map_err(store_err)? {
        serde_json::Value::String(s) => Ok(s),
        other => Err(store_err(format!("expected a unit variant, got {other}"))),
    }
}

pub(crate) fn ms(t: Timestamp) -> i64 {
    i64::try_from(t.0).unwrap_or(i64::MAX)
}

// ---------- filesystem permissions ---------------------------------------------

#[cfg(unix)]
fn ensure_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(store_err(format!(
                    "{} exists and is not a directory",
                    path.display()
                )));
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(store_err(format!(
                    "{} is accessible to other users (mode {mode:o}); expected 700",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(store_err),
        Err(e) => Err(store_err(e)),
    }
}

#[cfg(not(unix))]
fn ensure_private_dir(path: &Path) -> Result<()> {
    // Windows owner-only ACLs are a T23 platform task; until then the store
    // does not claim them.
    fs::create_dir_all(path).map_err(store_err)
}

#[cfg(unix)]
fn ensure_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() {
                return Err(store_err(format!(
                    "{} is not a regular file",
                    path.display()
                )));
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(store_err(format!(
                    "{} is accessible to other users (mode {mode:o}); expected 600",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map(drop)
            .map_err(store_err),
        Err(e) => Err(store_err(e)),
    }
}

#[cfg(not(unix))]
fn ensure_private_file(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::File::create(path).map_err(store_err)?;
    }
    Ok(())
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    ensure_private_dir(path)
}

// ---------- opening and recovery ---------------------------------------------------

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_clock(root, Arc::new(SystemClock))
    }

    pub fn open_with_clock(root: &Path, clock: Arc<dyn Clock>) -> Result<Self> {
        Self::open_with_migrations(root, clock, MIGRATIONS)
    }

    /// Open applying only `migrations`. Used to exercise upgrades from older
    /// schema versions; production code uses [`Store::open`].
    #[doc(hidden)]
    pub fn open_with_migrations(
        root: &Path,
        clock: Arc<dyn Clock>,
        migrations: &[Migration],
    ) -> Result<Self> {
        ensure_private_dir(root)?;
        ensure_private_dir(&root.join("blobs"))?;
        ensure_private_dir(&root.join("tmp"))?;
        let lock = acquire_lock(root)?;
        let db_path = root.join("store.db");
        // Created 0600 before SQLite opens it; SQLite gives its -wal and -shm
        // files the database file's permissions.
        ensure_private_file(&db_path)?;

        let mut conn = Connection::open(&db_path).map_err(store_err)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(store_err)?;
        migrations::apply(&mut conn, migrations)?;

        let mut store = Self {
            root: root.to_path_buf(),
            _lock: lock,
            conn: Mutex::new(conn),
            clock,
            recovery: RecoveryReport::default(),
        };
        store.recovery = store.recover()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn schema_version(&self) -> Result<u32> {
        migrations::current_version(&*self.conn()?)
    }

    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.recovery
    }

    pub fn at_rest_protection(&self) -> AtRestProtection {
        AtRestProtection::Unverified {
            reason: "The store does not encrypt data. Volume encryption has not been \
                     verified by the store; `razista doctor` reports it."
                .into(),
        }
    }

    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| store_err("connection lock poisoned"))
    }

    pub(crate) fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Reconcile disk with committed metadata after a crash.
    fn recover(&self) -> Result<RecoveryReport> {
        let mut report = RecoveryReport::default();

        for entry in fs::read_dir(self.root.join("tmp")).map_err(store_err)? {
            let path = entry.map_err(store_err)?.path();
            fs::remove_file(&path).map_err(store_err)?;
            report.partial_files_removed += 1;
        }

        let conn = self.conn()?;
        let mut known = conn
            .prepare("SELECT 1 FROM artifacts WHERE digest = ?1")
            .map_err(store_err)?;
        for shard in fs::read_dir(self.root.join("blobs")).map_err(store_err)? {
            let shard = shard.map_err(store_err)?.path();
            if !shard.is_dir() {
                fs::remove_file(&shard).map_err(store_err)?;
                report.orphan_blobs_removed += 1;
                continue;
            }
            for blob in fs::read_dir(&shard).map_err(store_err)? {
                let blob = blob.map_err(store_err)?.path();
                let name = blob
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let digest = format!("sha256:{name}");
                let committed = known
                    .query_row(params![digest], |_| Ok(()))
                    .optional()
                    .map_err(store_err)?
                    .is_some();
                if !committed {
                    fs::remove_file(&blob).map_err(store_err)?;
                    report.orphan_blobs_removed += 1;
                }
            }
        }
        drop(known);

        let mut rows = conn
            .prepare("SELECT digest FROM artifacts")
            .map_err(store_err)?;
        let digests: Vec<String> = rows
            .query_map([], |r| r.get(0))
            .map_err(store_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(store_err)?;
        for digest in digests {
            if !self.blob_path_for_digest(&digest).is_file() {
                report.missing_blobs += 1;
            }
        }
        Ok(report)
    }
}

// ---------- repositories and preparations --------------------------------------------

impl Store {
    pub fn register_repo(&self, grant: &RepoGrant) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO repos (repo_id, owner, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                grant.repo_id.to_string(),
                grant.owner.to_string(),
                to_json(grant)?,
                ms(grant.created_at)
            ],
        )
        .map_err(|e| {
            if is_constraint(&e) {
                ReviewError::invalid("repository is already registered")
            } else {
                store_err(e)
            }
        })?;
        Ok(())
    }

    pub fn get_repo(&self, caller: &CallerId, repo: RepoId) -> Result<RepoGrant> {
        let conn = self.conn()?;
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT owner, body FROM repos WHERE repo_id = ?1",
                params![repo.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_err)?;
        match row {
            Some((owner, body)) if owner == caller.to_string() => from_json(&body),
            _ => Err(not_found_or_not_owned("repository")),
        }
    }

    /// Store a frozen preparation. Only the repository's owner may prepare.
    pub fn put_preparation(
        &self,
        caller: &CallerId,
        repo: RepoId,
        preparation: &PreparedReview,
    ) -> Result<()> {
        let conn = self.conn()?;
        let owner: Option<String> = conn
            .query_row(
                "SELECT owner FROM repos WHERE repo_id = ?1",
                params![repo.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_err)?;
        if owner.as_deref() != Some(caller.to_string().as_str()) {
            return Err(not_found_or_not_owned("repository"));
        }
        conn.execute(
            "INSERT INTO preparations (preparation_id, owner, repo_id, body, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                preparation.preparation_id.to_string(),
                caller.to_string(),
                repo.to_string(),
                to_json(preparation)?,
                ms(preparation.expires_at)
            ],
        )
        .map_err(|e| {
            if is_constraint(&e) {
                ReviewError::invalid("preparation id is already stored")
            } else {
                store_err(e)
            }
        })?;
        Ok(())
    }
}

// ---------- jobs ------------------------------------------------------------------------

impl Store {
    /// Durably create the job for `request`, or return the job a previous
    /// identical request created. Returns only after the record is committed.
    pub fn create_job(&self, request: ReviewRequest) -> Result<ReviewJob> {
        validate_request(&request)?;
        let now = self.now();
        let caller = request.caller.to_string();
        let mut conn = self.conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_err)?;

        let prep: Option<(String, String, String, i64)> = tx
            .query_row(
                "SELECT owner, repo_id, body, expires_at FROM preparations
                 WHERE preparation_id = ?1",
                params![request.preparation_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(store_err)?;
        let Some((owner, repo_id, prep_body, expires_at)) = prep else {
            return Err(not_found_or_not_owned("preparation"));
        };
        if owner != caller {
            return Err(not_found_or_not_owned("preparation"));
        }

        // Idempotency before expiry: a retry after a lost response must find
        // the job it already started.
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT preparation_id, body FROM jobs
                 WHERE owner = ?1 AND repo_id = ?2 AND idempotency_key = ?3",
                params![caller, repo_id, request.idempotency_key.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_err)?;
        if let Some((prep_id, body)) = existing {
            if prep_id == request.preparation_id.to_string() {
                return from_json(&body);
            }
            return Err(ReviewError::new(
                ErrorCode::IdempotencyConflict,
                "idempotency key was already used for a different preparation",
            ));
        }

        if ms(now) >= expires_at {
            return Err(ReviewError::new(
                ErrorCode::PolicyDenied,
                "preparation has expired; prepare the review again",
            ));
        }

        let preparation: PreparedReview = from_json(&prep_body)?;
        let job = ReviewJob {
            job_id: JobId::new(),
            request: request.clone(),
            stage: JobStage::Queued,
            status: JobState::Queued,
            budget: preparation.plan.budget.clone(),
            gaps: BoundedVec::new(preparation.plan.disabled_capabilities.into_inner())?,
            created_at: now,
        };
        tx.execute(
            "INSERT INTO jobs (job_id, owner, repo_id, preparation_id, snapshot_id,
                               idempotency_key, status, body, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                job.job_id.to_string(),
                caller,
                repo_id,
                request.preparation_id.to_string(),
                preparation.snapshot_id.to_string(),
                request.idempotency_key.as_str(),
                enum_name(&job.status)?,
                to_json(&job)?,
                ms(now)
            ],
        )
        .map_err(store_err)?;
        tx.commit().map_err(store_err)?;
        Ok(job)
    }

    pub fn get_job(&self, caller: &CallerId, id: JobId) -> Result<ReviewJob> {
        let conn = self.conn()?;
        let body: Option<String> = conn
            .query_row(
                "SELECT body FROM jobs WHERE job_id = ?1 AND owner = ?2",
                params![id.to_string(), caller.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_err)?;
        body.map_or_else(|| Err(not_found_or_not_owned("job")), |b| from_json(&b))
    }

    /// Move a job to a new stage/state. Controller-internal: callers reach it
    /// only through the controller, which owns job progression.
    pub fn transition(&self, id: JobId, stage: JobStage, status: JobState) -> Result<ReviewJob> {
        let now = self.now();
        let mut conn = self.conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_err)?;
        let body: Option<String> = tx
            .query_row(
                "SELECT body FROM jobs WHERE job_id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_err)?;
        let mut job: ReviewJob = match body {
            Some(b) => from_json(&b)?,
            None => return Err(ReviewError::invalid(format!("unknown job {id}"))),
        };
        if job.status.is_terminal() {
            return Err(ReviewError::new(
                ErrorCode::JobTerminal,
                format!("job {id} is {:?}; terminal jobs are immutable", job.status),
            ));
        }
        job.stage = stage;
        job.status = status;
        tx.execute(
            "UPDATE jobs SET status = ?2, body = ?3, updated_at = ?4 WHERE job_id = ?1",
            params![id.to_string(), enum_name(&status)?, to_json(&job)?, ms(now)],
        )
        .map_err(store_err)?;
        crate::events::insert_event(
            &tx,
            &razista_core::Event {
                id: razista_core::EventId::new(),
                job_id: id,
                at: now,
                kind: razista_core::EventKind::StateChanged { state: status },
            },
        )?;
        tx.commit().map_err(store_err)?;
        Ok(job)
    }
}

// ---------- grants -----------------------------------------------------------------------

impl Store {
    /// Persist a grant minted by the controller. Grant ids are write-once.
    pub fn put_grant(&self, grant: &ActionGrant) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO grants (grant_id, caller, body, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                grant.grant_id.to_string(),
                grant.caller.to_string(),
                to_json(grant)?,
                ms(grant.expires_at)
            ],
        )
        .map_err(|e| {
            if is_constraint(&e) {
                ReviewError::invalid("grant id is already stored")
            } else {
                store_err(e)
            }
        })?;
        Ok(())
    }

    /// Load a grant for `caller`. This is the only source `authorize` should
    /// be given grants from.
    pub fn load_grant(&self, caller: &CallerId, id: GrantId) -> Result<ActionGrant> {
        let conn = self.conn()?;
        let body: Option<String> = conn
            .query_row(
                "SELECT body FROM grants WHERE grant_id = ?1 AND caller = ?2",
                params![id.to_string(), caller.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_err)?;
        body.map_or_else(|| Err(not_found_or_not_owned("grant")), |b| from_json(&b))
    }
}

// ---------- retention ---------------------------------------------------------------------

impl Store {
    /// Remove expired terminal jobs (with their events and unpinned evidence)
    /// and old artefacts nothing references. Running jobs and anything tied
    /// to pinned evidence are kept.
    pub fn apply_retention(&self, policy: &RetentionPolicy) -> Result<RetentionReport> {
        let now = self.now().0;
        let job_cutoff = i64::try_from(now.saturating_sub(policy.job_metadata_ms)).unwrap_or(0);
        let blob_cutoff = i64::try_from(now.saturating_sub(policy.raw_artifacts_ms)).unwrap_or(0);
        let mut report = RetentionReport {
            outside_control: vec![
                "Reports exported outside the store directory".into(),
                "Filesystem snapshots, backups and SSD remnant blocks".into(),
                "Copies held by connected MCP clients".into(),
            ],
            ..RetentionReport::default()
        };

        // The connection lock is held until the files are unlinked too, so
        // `put_artifact` (which takes the same lock) cannot re-store bytes
        // between the row deletion and the unlink.
        let mut conn = self.conn()?;
        let doomed: Vec<(String, u64)>;
        {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(store_err)?;
            let removed_jobs = tx
                .execute(
                    "DELETE FROM jobs
                     WHERE status IN ('completed', 'partial', 'failed', 'cancelled')
                       AND updated_at <= ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM evidence e
                           WHERE e.job_id = jobs.job_id AND e.pinned = 1)",
                    params![job_cutoff],
                )
                .map_err(store_err)?;
            report.jobs_removed = removed_jobs as u64;

            {
                let mut stmt = tx
                    .prepare(
                        "SELECT digest, size FROM artifacts a
                         WHERE created_at <= ?1
                           AND NOT EXISTS (
                               SELECT 1 FROM evidence_artifacts l WHERE l.digest = a.digest)",
                    )
                    .map_err(store_err)?;
                doomed = stmt
                    .query_map(params![blob_cutoff], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
                    })
                    .map_err(store_err)?
                    .collect::<std::result::Result<_, _>>()
                    .map_err(store_err)?;
            }
            for (digest, _) in &doomed {
                tx.execute("DELETE FROM artifacts WHERE digest = ?1", params![digest])
                    .map_err(store_err)?;
            }
            tx.commit().map_err(store_err)?;
        }

        // Rows are gone first, so a failed unlink leaves an orphan that the
        // next recovery removes; it can never be read as evidence.
        for (digest, size) in doomed {
            let path = self.blob_path_for_digest(&digest);
            match fs::remove_file(&path) {
                Ok(()) => {
                    report.artifacts_removed += 1;
                    report.bytes_freed += size;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    report.artifacts_removed += 1;
                }
                Err(e) => report
                    .files_not_deleted
                    .push(format!("{}: {e}", path.display())),
            }
        }
        drop(conn);
        Ok(report)
    }
}

/// Take the store's exclusive lock. A second process (or a second `Store` in
/// this one) would otherwise run recovery against blobs still being written.
fn acquire_lock(root: &Path) -> Result<fs::File> {
    let path = root.join("LOCK");
    ensure_private_file(&path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(store_err)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err(store_err(format!(
            "{} is in use by another store instance",
            root.display()
        ))),
        Err(fs::TryLockError::Error(e)) => Err(store_err(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pragma(store: &Store, name: &str) -> i64 {
        store
            .conn()
            .expect("conn")
            .pragma_query_value(None, name, |r| r.get(0))
            .expect("pragma")
    }

    #[test]
    fn commits_are_flushed_to_stable_storage() {
        // Review finding 4: on macOS plain fsync does not flush the drive
        // cache; SQLite needs fullfsync for a commit to survive power loss.
        let dir = tempfile::tempdir().expect("tmp");
        let store = Store::open(&dir.path().join("store")).expect("open");
        assert_eq!(pragma(&store, "synchronous"), 2, "FULL");
        assert_eq!(pragma(&store, "fullfsync"), 1);
        assert_eq!(pragma(&store, "checkpoint_fullfsync"), 1);
        assert_eq!(pragma(&store, "foreign_keys"), 1);
        let mode: String = store
            .conn()
            .expect("conn")
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .expect("pragma");
        assert_eq!(mode, "wal");
    }
}

/// What a store directory looks like from outside, without touching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreProbe {
    pub exists: bool,
    /// Directory mode grants nothing to group or other users.
    pub private: bool,
    /// Another `Store` (normally the per-user daemon) holds the lock.
    pub in_use: bool,
    pub schema_version: Option<u32>,
}

impl Store {
    /// Inspect `root` read-only: never creates, migrates or recovers anything,
    /// and opens the database read-only. The lock is only *tested*; if it is
    /// free it is held for the instant of the test and released at once.
    pub fn probe(root: &Path) -> Result<StoreProbe> {
        let meta = match fs::symlink_metadata(root) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreProbe {
                    exists: false,
                    private: false,
                    in_use: false,
                    schema_version: None,
                });
            }
            Err(e) => return Err(store_err(e)),
        };
        #[cfg(unix)]
        let private = {
            use std::os::unix::fs::PermissionsExt;
            meta.is_dir() && meta.permissions().mode() & 0o077 == 0
        };
        #[cfg(not(unix))]
        let private = meta.is_dir();

        let in_use = match fs::File::open(root.join("LOCK")) {
            Ok(f) => matches!(f.try_lock(), Err(fs::TryLockError::WouldBlock)),
            Err(_) => false,
        };
        let db = root.join("store.db");
        let schema_version = if db.is_file() {
            Connection::open_with_flags(
                &db,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .ok()
            .and_then(|c| migrations::current_version(&c).ok())
        } else {
            None
        };
        Ok(StoreProbe {
            exists: true,
            private,
            in_use,
            schema_version,
        })
    }
}

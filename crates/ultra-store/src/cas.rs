//! Content-addressed artefact blobs and the evidence records that cite them.
//!
//! Publication order is: write to `tmp/`, fsync, rename into `blobs/`, fsync
//! the directory, then commit the metadata row. A blob is evidence only when
//! its row is committed *and* its bytes still hash to its name, so a crash at
//! any step leaves nothing readable that was not fully published.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use ultra_core::{
    ArtifactId, CallerId, Digest, ErrorCode, Evidence, EvidenceId, JobId, Result, ReviewError,
    TrustLabel,
};

use crate::Store;
use crate::db::{
    enum_name, from_json, is_constraint, ms, not_found_or_not_owned, private_dir, store_err,
    to_json,
};

fn hex_of(digest: &str) -> &str {
    digest.strip_prefix("sha256:").unwrap_or(digest)
}

fn parse_trust(name: &str) -> Result<TrustLabel> {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .map_err(|e| store_err(format!("corrupt trust label {name:?}: {e}")))
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(f)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<fs::File> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(f)
}

fn make_read_only(path: &std::path::Path) -> std::io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o400);
    }
    #[cfg(not(unix))]
    perms.set_readonly(true);
    fs::set_permissions(path, perms)
}

fn sync_dir(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

impl Store {
    /// Where an artefact's bytes live. Exposed for crash-simulation tests.
    #[doc(hidden)]
    pub fn blob_path(&self, id: &ArtifactId) -> PathBuf {
        self.blob_path_for_digest(id.digest().as_str())
    }

    pub(crate) fn blob_path_for_digest(&self, digest: &str) -> PathBuf {
        let hex = hex_of(digest);
        let shard = hex.get(..2).unwrap_or("xx");
        self.root().join("blobs").join(shard).join(hex)
    }

    /// Store `bytes` and return their content address. Returns only after the
    /// bytes and their metadata row are both durable.
    ///
    /// `owner` is recorded as having stored these bytes; only evidence of that
    /// owner's jobs may later cite them.
    pub fn put_artifact(
        &self,
        owner: &CallerId,
        bytes: &[u8],
        trust: TrustLabel,
    ) -> Result<ArtifactId> {
        let digest = Digest::of(bytes);
        let id = ArtifactId::from(digest.clone());
        let blob = self.blob_path(&id);
        let now = self.now();
        // Held across publication and the row upsert: retention takes the
        // same lock, so it cannot unlink bytes this call has just reported stored.
        let conn = self.conn()?;

        // Publish the bytes unless an intact copy is already in place. A missing
        // or altered blob is replaced by the bytes that match its address.
        let intact = fs::read(&blob).is_ok_and(|b| Digest::of(&b) == digest);
        if !intact {
            let tmp = self
                .root()
                .join("tmp")
                .join(format!("{}.partial", uuid::Uuid::now_v7()));
            write_private(&tmp, bytes).map_err(store_err)?;
            let shard = blob
                .parent()
                .ok_or_else(|| store_err("blob path has no parent"))?;
            private_dir(shard)?;
            fs::rename(&tmp, &blob).map_err(store_err)?;
            make_read_only(&blob).map_err(store_err)?;
            sync_dir(shard).map_err(store_err)?;
        }

        // Two different provenance claims for the same bytes collapse to
        // `untrusted`, so a re-upload can never launder a label. One statement,
        // so concurrent puts cannot race past the downgrade.
        // Re-storing bytes refreshes their age, so retention cannot expire
        // an artefact a running job has just produced again.
        conn.execute(
            "INSERT INTO artifacts (digest, size, trust, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (digest) DO UPDATE SET
                     created_at = excluded.created_at,
                     trust = CASE WHEN artifacts.trust = excluded.trust THEN artifacts.trust
                                  ELSE ?5 END",
            params![
                digest.as_str(),
                bytes.len() as i64,
                enum_name(&trust)?,
                ms(now),
                enum_name(&TrustLabel::Untrusted)?
            ],
        )
        .map_err(store_err)?;
        conn.execute(
            "INSERT OR IGNORE INTO artifact_owners (digest, owner) VALUES (?1, ?2)",
            params![digest.as_str(), owner.to_string()],
        )
        .map_err(store_err)?;
        Ok(id)
    }

    /// Read an artefact by content address, with no ownership check.
    ///
    /// Controller-internal only (e.g. handing a stored test to the sandbox).
    /// Anything serving a caller must use [`Store::read_evidence_artifact`]:
    /// artefacts are shared by content across owners, so an address grants nothing.
    pub fn read_artifact_unscoped(&self, id: &ArtifactId) -> Result<(Vec<u8>, TrustLabel)> {
        let conn = self.conn()?;
        self.read_artifact_in(&conn, id)
    }

    /// Read an artefact and prove its bytes still match its address, using a
    /// connection (or transaction) the caller already holds.
    fn read_artifact_in(
        &self,
        conn: &Connection,
        id: &ArtifactId,
    ) -> Result<(Vec<u8>, TrustLabel)> {
        let digest = id.digest().as_str();
        let trust: Option<String> = conn
            .query_row(
                "SELECT trust FROM artifacts WHERE digest = ?1",
                params![digest],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_err)?;
        let Some(trust) = trust else {
            return Err(ReviewError::evidence(format!(
                "artifact {digest} is not a committed artefact"
            )));
        };
        let bytes = fs::read(self.blob_path(id)).map_err(|e| {
            ReviewError::evidence(format!("artifact {digest} bytes are unavailable: {e}"))
        })?;
        if Digest::of(&bytes) != *id.digest() {
            return Err(ReviewError::evidence(format!(
                "artifact {digest} bytes no longer match their digest"
            )));
        }
        Ok((bytes, parse_trust(&trust)?))
    }

    /// Record evidence for `job`. Every cited artefact must be committed and
    /// intact, and the evidence must describe the job's own snapshot.
    pub fn put_evidence(&self, job: JobId, evidence: &Evidence) -> Result<()> {
        let now = self.now();
        let mut conn = self.conn()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_err)?;
        let row: Option<(String, String, String)> = tx
            .query_row(
                "SELECT snapshot_id, status, owner FROM jobs WHERE job_id = ?1",
                params![job.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(store_err)?;
        let Some((snapshot, status, owner)) = row else {
            return Err(ReviewError::invalid(format!("unknown job {job}")));
        };
        if matches!(
            status.as_str(),
            "completed" | "partial" | "failed" | "cancelled"
        ) {
            return Err(ReviewError::new(
                ErrorCode::JobTerminal,
                format!("job {job} is {status}; re-verification needs a child job"),
            ));
        }
        // Every artefact the body names, including the test behind an
        // execution, must be committed and intact, and is linked so retention
        // keeps what replay needs.
        let named: Vec<&ArtifactId> = evidence
            .artifact_refs
            .iter()
            .chain(
                evidence
                    .action
                    .as_ref()
                    .and_then(|a| a.test_artifact.as_ref()),
            )
            .collect();
        for artifact in &named {
            let stored_by_owner = tx
                .query_row(
                    "SELECT 1 FROM artifact_owners WHERE digest = ?1 AND owner = ?2",
                    params![artifact.digest().as_str(), owner],
                    |_| Ok(()),
                )
                .optional()
                .map_err(store_err)?
                .is_some();
            if !stored_by_owner {
                // Uniform with other ownership denials; also covers digests
                // that were never stored at all.
                self.read_artifact_in(&tx, artifact)?;
                return Err(not_found_or_not_owned("artifact"));
            }
            self.read_artifact_in(&tx, artifact)?;
        }
        if snapshot != evidence.subject_snapshot.to_string() {
            return Err(ReviewError::evidence(
                "evidence describes a different snapshot than its job",
            ));
        }
        tx.execute(
            "INSERT INTO evidence (evidence_id, job_id, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                evidence.id.to_string(),
                job.to_string(),
                to_json(evidence)?,
                ms(now)
            ],
        )
        .map_err(|e| {
            if is_constraint(&e) {
                ReviewError::invalid("evidence id is already recorded")
            } else {
                store_err(e)
            }
        })?;
        for artifact in named {
            tx.execute(
                "INSERT OR IGNORE INTO evidence_artifacts (evidence_id, digest) VALUES (?1, ?2)",
                params![evidence.id.to_string(), artifact.digest().as_str()],
            )
            .map_err(store_err)?;
        }
        tx.commit().map_err(store_err)?;
        Ok(())
    }

    /// The owning job's caller, or a uniform denial.
    fn evidence_owned_by(&self, caller: &CallerId, id: EvidenceId) -> Result<String> {
        let row: Option<(String, String)> = self
            .conn()?
            .query_row(
                "SELECT e.body, j.owner FROM evidence e JOIN jobs j ON j.job_id = e.job_id
                 WHERE e.evidence_id = ?1",
                params![id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_err)?;
        match row {
            Some((body, owner)) if owner == caller.to_string() => Ok(body),
            _ => Err(not_found_or_not_owned("evidence")),
        }
    }

    pub fn get_evidence(&self, caller: &CallerId, id: EvidenceId) -> Result<Evidence> {
        from_json(&self.evidence_owned_by(caller, id)?)
    }

    /// Read an artefact *through* evidence the caller owns. The artefact must
    /// be linked to that evidence; content addresses alone grant nothing.
    pub fn read_evidence_artifact(
        &self,
        caller: &CallerId,
        evidence: EvidenceId,
        artifact: &ArtifactId,
    ) -> Result<Vec<u8>> {
        self.evidence_owned_by(caller, evidence)?;
        let linked = self
            .conn()?
            .query_row(
                "SELECT 1 FROM evidence_artifacts WHERE evidence_id = ?1 AND digest = ?2",
                params![evidence.to_string(), artifact.digest().as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(store_err)?
            .is_some();
        if !linked {
            return Err(not_found_or_not_owned("artifact"));
        }
        Ok(self.read_artifact_unscoped(artifact)?.0)
    }

    pub fn pin_evidence(&self, caller: &CallerId, id: EvidenceId) -> Result<()> {
        self.set_pinned(caller, id, true)
    }

    pub fn unpin_evidence(&self, caller: &CallerId, id: EvidenceId) -> Result<()> {
        self.set_pinned(caller, id, false)
    }

    fn set_pinned(&self, caller: &CallerId, id: EvidenceId, pinned: bool) -> Result<()> {
        self.evidence_owned_by(caller, id)?;
        self.conn()?
            .execute(
                "UPDATE evidence SET pinned = ?2 WHERE evidence_id = ?1",
                params![id.to_string(), i64::from(pinned)],
            )
            .map_err(store_err)?;
        Ok(())
    }
}

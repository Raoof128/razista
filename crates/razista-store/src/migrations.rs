//! Versioned schema migrations, tracked in SQLite's `user_version`.
//!
//! Migrations are append-only: never edit a released one, add a new version.
//! A store whose schema is newer than this build is refused rather than
//! guessed at.

use razista_core::{ErrorCode, Result, ReviewError};
use rusqlite::Connection;

use crate::db::store_err;

pub struct Migration {
    pub version: u32,
    pub sql: &'static str,
}

const V1: &str = r#"
CREATE TABLE repos (
    repo_id    TEXT PRIMARY KEY,
    owner      TEXT NOT NULL,
    body       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE preparations (
    preparation_id TEXT PRIMARY KEY,
    owner          TEXT NOT NULL,
    repo_id        TEXT NOT NULL REFERENCES repos(repo_id),
    body           TEXT NOT NULL,
    expires_at     INTEGER NOT NULL
);

CREATE TABLE jobs (
    job_id          TEXT PRIMARY KEY,
    owner           TEXT NOT NULL,
    repo_id         TEXT NOT NULL REFERENCES repos(repo_id),
    preparation_id  TEXT NOT NULL REFERENCES preparations(preparation_id),
    snapshot_id     TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    status          TEXT NOT NULL,
    body            TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (owner, repo_id, idempotency_key)
);

-- Defence in depth for "terminal jobs are immutable": the API refuses first.
CREATE TRIGGER jobs_terminal_immutable BEFORE UPDATE ON jobs
WHEN OLD.status IN ('completed', 'partial', 'failed', 'cancelled')
BEGIN
    SELECT RAISE(ABORT, 'terminal jobs are immutable');
END;

CREATE TABLE events (
    seq      INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    job_id   TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
    at       INTEGER NOT NULL,
    body     TEXT NOT NULL
);

CREATE TRIGGER events_append_only BEFORE UPDATE ON events
BEGIN
    SELECT RAISE(ABORT, 'events are append-only');
END;

CREATE TABLE artifacts (
    digest     TEXT PRIMARY KEY,
    size       INTEGER NOT NULL,
    trust      TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- Who stored each artefact. Evidence may only cite artefacts its job's owner
-- stored, so knowing a digest grants nothing (no cross-owner confused deputy).
CREATE TABLE artifact_owners (
    digest TEXT NOT NULL REFERENCES artifacts(digest) ON DELETE CASCADE,
    owner  TEXT NOT NULL,
    PRIMARY KEY (digest, owner)
);

CREATE TABLE evidence (
    evidence_id TEXT PRIMARY KEY,
    job_id      TEXT NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE evidence_artifacts (
    evidence_id TEXT NOT NULL REFERENCES evidence(evidence_id) ON DELETE CASCADE,
    digest      TEXT NOT NULL REFERENCES artifacts(digest),
    PRIMARY KEY (evidence_id, digest)
);

CREATE TABLE grants (
    grant_id   TEXT PRIMARY KEY,
    caller     TEXT NOT NULL,
    body       TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
"#;

const V2: &str = r#"
-- Evidence pinned by the user survives retention until released.
ALTER TABLE evidence ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
CREATE INDEX events_by_job ON events (job_id, seq);
CREATE INDEX evidence_by_job ON evidence (job_id);
CREATE INDEX evidence_links_by_digest ON evidence_artifacts (digest);
"#;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: V1,
    },
    Migration {
        version: 2,
        sql: V2,
    },
];

pub(crate) fn current_version(conn: &Connection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(store_err)
}

/// Bring the schema up to the last migration in `migrations`.
pub(crate) fn apply(conn: &mut Connection, migrations: &[Migration]) -> Result<u32> {
    for (i, m) in migrations.iter().enumerate() {
        if m.version != i as u32 + 1 {
            return Err(ReviewError::new(
                ErrorCode::StoreUnavailable,
                "migrations must be numbered contiguously from 1",
            ));
        }
    }
    let current = current_version(conn)?;
    let target = migrations.last().map_or(0, |m| m.version);
    if current > target {
        return Err(ReviewError::new(
            ErrorCode::StoreUnavailable,
            format!("store schema v{current} is newer than this build supports (v{target})"),
        ));
    }
    for m in migrations.iter().filter(|m| m.version > current) {
        let tx = conn.transaction().map_err(store_err)?;
        tx.execute_batch(m.sql).map_err(store_err)?;
        tx.pragma_update(None, "user_version", m.version)
            .map_err(store_err)?;
        tx.commit().map_err(store_err)?;
    }
    Ok(target)
}

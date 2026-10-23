//! Shared fixtures for the store tests.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ultra_core::{
    AgentId, ArtifactId, BoundedVec, CallerId, Digest, Event, EventId, EventKind, Evidence,
    EvidenceId, EvidenceKind, IdempotencyKey, Ident, JobId, JobStage, PreparationId,
    PreparedReview, RepoGrant, RepoId, ResourceBudget, ReviewPlan, ReviewRequest, RoleKind,
    ScopeMode, SnapshotId, Text, Timestamp,
};
use ultra_store::{Clock, Store};

pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;
pub const T0: u64 = 1_800_000_000_000;

/// A clock the test moves by hand.
#[derive(Debug, Default)]
pub struct FakeClock(AtomicU64);

impl FakeClock {
    pub fn at(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }

    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

pub fn open(root: &Path, clock: &Arc<FakeClock>) -> Store {
    Store::open_with_clock(root, clock.clone()).expect("store opens")
}

/// A fresh store directory inside a temp dir (the store creates `root`).
pub fn temp_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    (dir, root)
}

pub struct Setup {
    pub owner: CallerId,
    pub repo: RepoId,
    pub preparation: PreparedReview,
}

/// Register a repository owned by a fresh caller and store one preparation.
pub fn prepared(store: &Store) -> Setup {
    let owner = CallerId::new();
    let repo = RepoId::new();
    store
        .register_repo(&RepoGrant {
            repo_id: repo,
            canonical_identity: Text::new("/work/project").expect("text"),
            owner,
            allowed_scopes: BoundedVec::new(vec![ScopeMode::Worktree]).expect("bounded"),
            created_at: Timestamp(T0),
        })
        .expect("repo registers");
    let preparation = preparation_for(store, &owner, repo);
    Setup {
        owner,
        repo,
        preparation,
    }
}

pub fn preparation_for(store: &Store, owner: &CallerId, repo: RepoId) -> PreparedReview {
    let preparation = PreparedReview {
        preparation_id: PreparationId::new(),
        snapshot_id: SnapshotId::new(),
        policy_hash: Digest::of(b"policy"),
        plan: ReviewPlan {
            profile_id: Ident::new("offline-strict").expect("ident"),
            roles: BoundedVec::new(vec![RoleKind::Correctness]).expect("bounded"),
            disabled_capabilities: BoundedVec::empty(),
            budget: ResourceBudget::default(),
        },
        expires_at: Timestamp(T0 + DAY_MS),
    };
    store
        .put_preparation(owner, repo, &preparation)
        .expect("preparation stored");
    preparation
}

pub fn request(setup: &Setup, key: &str) -> ReviewRequest {
    ReviewRequest {
        preparation_id: setup.preparation.preparation_id,
        idempotency_key: IdempotencyKey::new(key).expect("key"),
        caller: setup.owner,
    }
}

pub fn stage_event(job: JobId, at: u64, stage: JobStage) -> Event {
    Event {
        id: EventId::new(),
        job_id: job,
        at: Timestamp(at),
        kind: EventKind::StageEntered { stage },
    }
}

pub fn evidence(job_snapshot: SnapshotId, artifacts: Vec<ArtifactId>) -> Evidence {
    Evidence {
        id: EvidenceId::new(),
        kind: EvidenceKind::Execution,
        subject_snapshot: job_snapshot,
        producer: AgentId::new(),
        action: None,
        outcome: None,
        artifact_refs: BoundedVec::new(artifacts).expect("bounded"),
    }
}

#[cfg(unix)]
pub fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
}

pub fn sandbox_request(snapshot: SnapshotId, test: ArtifactId) -> ultra_core::SandboxRequest {
    ultra_core::SandboxRequest {
        preset_id: Ident::new("python-pytest").expect("ident"),
        snapshot_id: snapshot,
        variant: ultra_core::SnapshotVariant::Reviewed,
        test_artifact: Some(test),
        limits: ultra_core::ExecutionLimits {
            cpu_millis: 60_000,
            memory_bytes: 1 << 30,
            wall_time_ms: 120_000,
            output_bytes: 1 << 20,
            processes: 64,
            disk_bytes: 1 << 28,
        },
    }
}

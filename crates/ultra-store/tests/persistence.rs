//! T04: durable jobs, append-only events and content-addressed artefacts.

mod common;

use common::*;
use ultra_core::{
    ArtifactId, Digest, ErrorCode, JobStage, JobState, SnapshotId, Timestamp, TrustLabel,
};
use ultra_store::AtRestProtection;

#[test]
fn acknowledged_job_survives_restart() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let (job, setup) = {
        let store = open(&root, &clock);
        let setup = prepared(&store);
        let job = store
            .create_job(request(&setup, "start-0001"))
            .expect("job acknowledged");
        (job, setup)
        // store dropped here: the process "exits" right after acknowledging.
    };
    let store = open(&root, &clock);
    let reloaded = store
        .get_job(&setup.owner, job.job_id)
        .expect("job survives");
    assert_eq!(reloaded, job);
    assert_eq!(reloaded.status, JobState::Queued);
    assert_eq!(reloaded.budget, setup.preparation.plan.budget);
}

#[test]
fn identical_start_returns_the_same_job() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let first = store
        .create_job(request(&setup, "start-0001"))
        .expect("first");
    let again = store
        .create_job(request(&setup, "start-0001"))
        .expect("retry");
    assert_eq!(
        first.job_id, again.job_id,
        "a lost response retries idempotently"
    );
}

#[test]
fn idempotency_conflict_is_rejected() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    store
        .create_job(request(&setup, "start-0001"))
        .expect("first");

    let other = preparation_for(&store, &setup.owner, setup.repo);
    let mut conflicting = request(&setup, "start-0001");
    conflicting.preparation_id = other.preparation_id;
    let err = store
        .create_job(conflicting)
        .expect_err("same key, different input");
    assert_eq!(err.code(), ErrorCode::IdempotencyConflict);
    assert!(!err.code().retryable());
}

#[test]
fn expired_preparation_cannot_start() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    clock.advance(DAY_MS); // expires_at is exclusive
    let err = store
        .create_job(request(&setup, "start-0001"))
        .expect_err("expired");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn terminal_state_is_immutable() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");

    store
        .transition(job.job_id, JobStage::Discovering, JobState::Running)
        .expect("running");
    store
        .transition(job.job_id, JobStage::Reporting, JobState::Completed)
        .expect("completed");

    // A late worker result tries to rewrite the outcome.
    let err = store
        .transition(job.job_id, JobStage::Verifying, JobState::Failed)
        .expect_err("terminal jobs are immutable");
    assert_eq!(err.code(), ErrorCode::JobTerminal);

    let after = store.get_job(&setup.owner, job.job_id).expect("job");
    assert_eq!(after.status, JobState::Completed);
    assert_eq!(after.stage, JobStage::Reporting);

    // Survives restart unchanged.
    drop(store);
    let store = open(&root, &clock);
    assert_eq!(
        store.get_job(&setup.owner, job.job_id).expect("job").status,
        JobState::Completed
    );
}

#[test]
fn events_are_ordered_and_bound_to_their_job() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");

    let e1 = stage_event(job.job_id, T0 + 1, JobStage::Snapshot);
    let e2 = stage_event(job.job_id, T0 + 2, JobStage::Indexing);
    store.append_event(job.job_id, e1.clone()).expect("append");
    store.append_event(job.job_id, e2.clone()).expect("append");

    let events = store.list_events(&setup.owner, job.job_id).expect("list");
    assert_eq!(events, vec![e1.clone(), e2]);

    // An event naming a different job cannot be filed under this one.
    let other = store
        .create_job(request(&setup, "start-0002"))
        .expect("job");
    let stray = stage_event(other.job_id, T0 + 3, JobStage::Scanning);
    let err = store
        .append_event(job.job_id, stray)
        .expect_err("job mismatch");
    assert_eq!(err.code(), ErrorCode::InvalidInput);

    // Re-appending an existing event id is refused, not overwritten.
    let err = store.append_event(job.job_id, e1).expect_err("duplicate");
    assert_eq!(err.code(), ErrorCode::InvalidInput);
}

#[test]
fn artifact_round_trips_and_detects_tampering() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let bytes = b"pytest output: 1 failed";
    let id = store
        .put_artifact(&anyone(), bytes, TrustLabel::ToolOutput)
        .expect("stored");
    assert_eq!(id, ArtifactId::from(Digest::of(bytes)));
    let (back, trust) = store.read_artifact_unscoped(&id).expect("read");
    assert_eq!(back, bytes);
    assert_eq!(trust, TrustLabel::ToolOutput);

    // Someone edits the blob on disk.
    let path = store.blob_path(&id);
    let mut perms = std::fs::metadata(&path).expect("stat").permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).expect("chmod");
    std::fs::write(&path, b"pytest output: 0 failed").expect("tamper");
    let err = store
        .read_artifact_unscoped(&id)
        .expect_err("tampering detected");
    assert_eq!(err.code(), ErrorCode::EvidenceInvalid);
}

#[test]
fn trust_label_cannot_be_laundered_by_reupload() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let bytes = b"same bytes";
    let id = store
        .put_artifact(&anyone(), bytes, TrustLabel::Untrusted)
        .expect("put");
    store
        .put_artifact(&anyone(), bytes, TrustLabel::Trusted)
        .expect("re-put");
    assert_eq!(
        store.read_artifact_unscoped(&id).expect("read").1,
        TrustLabel::Untrusted
    );

    let id = store
        .put_artifact(&anyone(), b"tool bytes", TrustLabel::ToolOutput)
        .expect("put");
    store
        .put_artifact(&anyone(), b"tool bytes", TrustLabel::ModelOutput)
        .expect("re-put");
    assert_eq!(
        store.read_artifact_unscoped(&id).expect("read").1,
        TrustLabel::Untrusted,
        "conflicting provenance degrades to untrusted"
    );
}

#[test]
fn evidence_requires_stored_artifacts() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");

    let ghost = ArtifactId::from(Digest::of(b"never stored"));
    let err = store
        .put_evidence(job.job_id, &evidence(SnapshotId::new(), vec![ghost]))
        .expect_err("dangling artefact");
    assert_eq!(err.code(), ErrorCode::EvidenceInvalid);

    let real = store
        .put_artifact(&setup.owner, b"log", TrustLabel::ToolOutput)
        .expect("put");
    let ev = evidence(setup.preparation.snapshot_id, vec![real]);
    store.put_evidence(job.job_id, &ev).expect("stored");
    assert_eq!(store.get_evidence(&setup.owner, ev.id).expect("read"), ev);
}

#[cfg(unix)]
#[test]
fn store_permissions_are_owner_only() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let id = store
        .put_artifact(&setup.owner, b"blob", TrustLabel::ToolOutput)
        .expect("put");
    store
        .append_event(job.job_id, stage_event(job.job_id, T0, JobStage::Snapshot))
        .expect("append");

    let mut checked = 0;
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        assert_eq!(mode(&dir), 0o700, "directory {}", dir.display());
        for entry in std::fs::read_dir(&dir).expect("list") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let m = mode(&path);
                assert_eq!(m & 0o077, 0, "{} is {m:o}", path.display());
                checked += 1;
            }
        }
    }
    assert!(checked >= 2, "database and blob files were checked");
    assert!(store.blob_path(&id).exists());
}

#[cfg(unix)]
#[test]
fn loose_store_directory_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, root) = temp_root();
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let clock = FakeClock::at(T0);
    let err = ultra_store::Store::open_with_clock(&root, clock).expect_err("too open");
    assert_eq!(err.code(), ErrorCode::StoreUnavailable);
}

#[test]
fn unencrypted_volume_status_is_not_misrepresented() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let status = store.at_rest_protection();
    assert!(
        matches!(status, AtRestProtection::Unverified { .. }),
        "the store does not encrypt and has not verified the volume: {status:?}"
    );
    let json = serde_json::to_value(&status).expect("serialises");
    assert_eq!(json["status"], "unverified");
    let _ = Timestamp(0);
}

// ---------- review findings ------------------------------------------------

#[test]
fn finished_job_accepts_no_new_evidence() {
    // Review finding 1: a late worker result cannot change a terminal job.
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let a = store
        .put_artifact(&setup.owner, b"late log", TrustLabel::ToolOutput)
        .expect("put");
    store
        .transition(job.job_id, JobStage::Reporting, JobState::Completed)
        .expect("done");
    let err = store
        .put_evidence(
            job.job_id,
            &evidence(setup.preparation.snapshot_id, vec![a]),
        )
        .expect_err("terminal job is immutable");
    assert_eq!(err.code(), ErrorCode::JobTerminal);
}

#[test]
fn second_open_of_the_same_store_is_refused() {
    // Review finding 3: another process's recovery would delete in-flight blobs.
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let first = open(&root, &clock);
    let err = ultra_store::Store::open_with_clock(&root, clock.clone())
        .expect_err("store is exclusively owned");
    assert_eq!(err.code(), ErrorCode::StoreUnavailable);
    drop(first);
    open(&root, &clock); // released on drop
}

#[test]
fn test_artifact_behind_evidence_must_exist() {
    // Review finding 5: every artefact an evidence body names is checked.
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let log = store
        .put_artifact(&setup.owner, b"log", TrustLabel::ToolOutput)
        .expect("put");
    let mut ev = evidence(setup.preparation.snapshot_id, vec![log]);
    ev.action = Some(sandbox_request(
        setup.preparation.snapshot_id,
        ArtifactId::from(Digest::of(b"test never stored")),
    ));
    let err = store
        .put_evidence(job.job_id, &ev)
        .expect_err("dangling test");
    assert_eq!(err.code(), ErrorCode::EvidenceInvalid);
}

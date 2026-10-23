//! T04: crash recovery, migrations and retention.

mod common;

use common::*;
use ultra_core::{ArtifactId, Digest, ErrorCode, JobStage, JobState, TrustLabel};
use ultra_store::{MIGRATIONS, RetentionPolicy, Store};

/// Simulate a crash after the blob was published but before its metadata
/// row committed, plus a half-written temp file from an earlier crash.
#[test]
fn partial_blob_is_never_visible() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let orphan = ArtifactId::from(Digest::of(b"orphan bytes"));
    {
        let store = open(&root, &clock);
        let path = store.blob_path(&orphan);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"orphan bytes").expect("blob without row");
        std::fs::write(root.join("tmp").join("crash.partial"), b"half").expect("partial");

        // Even before recovery runs, a blob without a committed row is not evidence.
        let err = store
            .read_artifact_unscoped(&orphan)
            .expect_err("not visible");
        assert_eq!(err.code(), ErrorCode::EvidenceInvalid);
    }

    let store = open(&root, &clock);
    let report = store.recovery_report();
    assert_eq!(report.orphan_blobs_removed, 1, "{report:?}");
    assert_eq!(report.partial_files_removed, 1, "{report:?}");
    assert!(!store.blob_path(&orphan).exists());
    assert!(
        std::fs::read_dir(root.join("tmp"))
            .expect("tmp")
            .next()
            .is_none(),
        "temp area is empty after recovery"
    );

    // Evidence cannot cite it either.
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let err = store
        .put_evidence(
            job.job_id,
            &evidence(setup.preparation.snapshot_id, vec![orphan]),
        )
        .expect_err("orphan is not evidence");
    assert_eq!(err.code(), ErrorCode::EvidenceInvalid);
}

#[test]
fn missing_blob_file_is_reported_not_silently_valid() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let id = {
        let store = open(&root, &clock);
        let id = store
            .put_artifact(&anyone(), b"log", TrustLabel::ToolOutput)
            .expect("put");
        std::fs::remove_file(store.blob_path(&id)).expect("lose the file");
        id
    };
    let store = open(&root, &clock);
    assert_eq!(store.recovery_report().missing_blobs, 1);
    let err = store.read_artifact_unscoped(&id).expect_err("missing");
    assert_eq!(err.code(), ErrorCode::EvidenceInvalid);
}

#[test]
fn migration_preserves_evidence_links() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let (setup, job, ev) = {
        // A store created by an older release that only knew migration 1.
        let store =
            Store::open_with_migrations(&root, clock.clone(), &MIGRATIONS[..1]).expect("v1");
        assert_eq!(store.schema_version().expect("version"), 1);
        let setup = prepared(&store);
        let job = store
            .create_job(request(&setup, "start-0001"))
            .expect("job");
        let a = store
            .put_artifact(&setup.owner, b"stdout", TrustLabel::ToolOutput)
            .expect("a");
        let b = store
            .put_artifact(&setup.owner, b"stderr", TrustLabel::ToolOutput)
            .expect("b");
        let ev = evidence(setup.preparation.snapshot_id, vec![a, b]);
        store.put_evidence(job.job_id, &ev).expect("evidence");
        (setup, job, ev)
    };

    let store = open(&root, &clock);
    assert_eq!(
        store.schema_version().expect("version"),
        MIGRATIONS.len() as u32
    );
    assert_eq!(
        store.get_evidence(&setup.owner, ev.id).expect("evidence"),
        ev
    );
    for artifact in ev.artifact_refs.iter() {
        store
            .read_evidence_artifact(&setup.owner, ev.id, artifact)
            .expect("link survives migration");
    }
    // Features of the new schema work on migrated rows.
    store
        .pin_evidence(&setup.owner, ev.id)
        .expect("pin after migration");
    assert_eq!(store.get_job(&setup.owner, job.job_id).expect("job"), job);
}

#[test]
fn newer_schema_is_refused() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    drop(open(&root, &clock));
    let err = Store::open_with_migrations(&root, clock, &MIGRATIONS[..1])
        .expect_err("a downgrade must not run against a newer schema");
    assert_eq!(err.code(), ErrorCode::StoreUnavailable);
}

#[test]
fn retention_removes_owned_cache_blobs() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");

    let loose = store
        .put_artifact(&setup.owner, b"scratch log", TrustLabel::ToolOutput)
        .expect("put");
    let kept = store
        .put_artifact(&setup.owner, b"evidence log", TrustLabel::ToolOutput)
        .expect("put");
    let ev = evidence(setup.preparation.snapshot_id, vec![kept.clone()]);
    store.put_evidence(job.job_id, &ev).expect("evidence");

    // Nothing is old enough yet.
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.artifacts_removed, 0);

    clock.advance(8 * DAY_MS);
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.artifacts_removed, 1, "{r:?}");
    assert!(r.bytes_freed >= b"scratch log".len() as u64);
    assert!(!store.blob_path(&loose).exists());
    assert_eq!(
        store
            .read_artifact_unscoped(&loose)
            .expect_err("gone")
            .code(),
        ErrorCode::EvidenceInvalid
    );
    store
        .read_artifact_unscoped(&kept)
        .expect("referenced artefact kept");
    assert!(
        !r.outside_control.is_empty(),
        "retention states what it cannot erase"
    );

    // A finished job past the metadata window goes, with its events and evidence.
    store
        .transition(job.job_id, JobStage::Reporting, JobState::Completed)
        .expect("done");
    store
        .append_event(job.job_id, stage_event(job.job_id, T0, JobStage::Reporting))
        .expect("event");
    clock.advance(31 * DAY_MS);
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.jobs_removed, 1, "{r:?}");
    assert_eq!(
        store
            .get_job(&setup.owner, job.job_id)
            .expect_err("gone")
            .code(),
        ErrorCode::PolicyDenied
    );
    // The evidence artefact is now unreferenced and old, so it goes too.
    assert!(!store.blob_path(&kept).exists());
}

#[test]
fn running_jobs_are_never_expired() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    store
        .transition(job.job_id, JobStage::Verifying, JobState::Running)
        .expect("running");
    clock.advance(90 * DAY_MS);
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.jobs_removed, 0);
    store
        .get_job(&setup.owner, job.job_id)
        .expect("still there");
}

#[test]
fn pinned_evidence_survives_retention() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let a = store
        .put_artifact(&setup.owner, b"repro run", TrustLabel::ToolOutput)
        .expect("put");
    let ev = evidence(setup.preparation.snapshot_id, vec![a.clone()]);
    store.put_evidence(job.job_id, &ev).expect("evidence");
    store.pin_evidence(&setup.owner, ev.id).expect("pin");
    store
        .transition(job.job_id, JobStage::Reporting, JobState::Completed)
        .expect("done");

    clock.advance(365 * DAY_MS);
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.jobs_removed, 0, "{r:?}");
    assert_eq!(r.artifacts_removed, 0, "{r:?}");
    store
        .get_evidence(&setup.owner, ev.id)
        .expect("pinned evidence kept");
    store
        .read_artifact_unscoped(&a)
        .expect("pinned artefact kept");

    store.unpin_evidence(&setup.owner, ev.id).expect("release");
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.jobs_removed, 1, "{r:?}");
    assert!(!store.blob_path(&a).exists());
}

/// Child half of `job_acknowledged_before_abort_survives`: acknowledge a job,
/// report it on stdout, then die without running any destructors.
#[test]
fn crash_child_acknowledges_then_aborts() {
    let Ok(root) = std::env::var("ULTRA_STORE_CRASH_ROOT") else {
        return; // only meaningful when spawned by the parent test
    };
    let clock = FakeClock::at(T0);
    let store = open(std::path::Path::new(&root), &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    println!("ACK {} {}", setup.owner, job.job_id);
    use std::io::Write;
    std::io::stdout().flush().expect("flush");
    std::process::abort();
}

#[test]
fn job_acknowledged_before_abort_survives() {
    let (_tmp, root) = temp_root();
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "crash_child_acknowledges_then_aborts",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ULTRA_STORE_CRASH_ROOT", &root)
        .output()
        .expect("child runs");
    assert!(!output.status.success(), "child must die abnormally");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ack = stdout
        .lines()
        .find_map(|l| l.split_once("ACK ").map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("child never acknowledged: {stdout}"));
    let (owner, job) = ack.split_once(' ').expect("two ids");
    let owner = ultra_core::CallerId::parse(owner).expect("caller id");
    let job = ultra_core::JobId::parse(job.trim()).expect("job id");

    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let reloaded = store
        .get_job(&owner, job)
        .expect("acknowledged job survives a crash");
    assert_eq!(reloaded.status, JobState::Queued);
}

#[test]
fn restored_bytes_are_not_immediately_expired() {
    // Review finding 2: re-storing old bytes refreshes their age.
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let id = store
        .put_artifact(&anyone(), b"", TrustLabel::ToolOutput)
        .expect("put");
    clock.advance(8 * DAY_MS);
    store
        .put_artifact(&anyone(), b"", TrustLabel::ToolOutput)
        .expect("put again");
    let r = store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert_eq!(r.artifacts_removed, 0, "{r:?}");
    assert!(store.blob_path(&id).exists());
}

#[test]
fn test_artifact_of_pinned_evidence_survives_retention() {
    // Review finding 5: the test that produced evidence is needed for replay.
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
    let test = store
        .put_artifact(
            &setup.owner,
            b"def test_add(): assert add(2, 2) == 4",
            TrustLabel::ModelOutput,
        )
        .expect("put");
    let mut ev = evidence(setup.preparation.snapshot_id, vec![log]);
    ev.action = Some(sandbox_request(setup.preparation.snapshot_id, test.clone()));
    store.put_evidence(job.job_id, &ev).expect("evidence");
    store.pin_evidence(&setup.owner, ev.id).expect("pin");

    clock.advance(60 * DAY_MS);
    store
        .apply_retention(&RetentionPolicy::default())
        .expect("retention");
    assert!(store.blob_path(&test).exists(), "replay input kept");
    store
        .read_evidence_artifact(&setup.owner, ev.id, &test)
        .expect("owner can read the test through the evidence");
}

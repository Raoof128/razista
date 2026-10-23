//! T04: every read is checked against the owning caller (R13). Possession of
//! an id is not authority.

mod common;

use common::*;
use ultra_core::policy::{ActionGrant, ActionKind};
use ultra_core::{
    BoundedVec, CallerId, Digest, ErrorCode, GrantId, JobId, JobStage, Timestamp, TrustLabel,
};

#[test]
fn cross_caller_read_is_denied() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    store
        .append_event(job.job_id, stage_event(job.job_id, T0, JobStage::Snapshot))
        .expect("event");
    let a = store
        .put_artifact(b"log", TrustLabel::ToolOutput)
        .expect("put");
    let ev = evidence(setup.preparation.snapshot_id, vec![a.clone()]);
    store.put_evidence(job.job_id, &ev).expect("evidence");

    let intruder = CallerId::new();
    for code in [
        store.get_job(&intruder, job.job_id).map(|_| ()),
        store.list_events(&intruder, job.job_id).map(|_| ()),
        store.get_evidence(&intruder, ev.id).map(|_| ()),
        store
            .read_evidence_artifact(&intruder, ev.id, &a)
            .map(|_| ()),
        store.pin_evidence(&intruder, ev.id),
    ]
    .into_iter()
    .map(|r| r.expect_err("denied").code())
    {
        assert_eq!(code, ErrorCode::PolicyDenied);
    }
}

#[test]
fn unknown_and_foreign_jobs_are_indistinguishable() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let intruder = CallerId::new();
    let foreign = store.get_job(&intruder, job.job_id).expect_err("foreign");
    let unknown = store.get_job(&intruder, JobId::new()).expect_err("unknown");
    assert_eq!(
        foreign, unknown,
        "no existence oracle for other callers' jobs"
    );
}

#[test]
fn artifact_not_linked_to_evidence_cannot_be_read_through_it() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let job = store
        .create_job(request(&setup, "start-0001"))
        .expect("job");
    let mine = store
        .put_artifact(b"mine", TrustLabel::ToolOutput)
        .expect("put");
    let other = store
        .put_artifact(b"another job's log", TrustLabel::ToolOutput)
        .expect("put");
    let ev = evidence(setup.preparation.snapshot_id, vec![mine]);
    store.put_evidence(job.job_id, &ev).expect("evidence");
    let err = store
        .read_evidence_artifact(&setup.owner, ev.id, &other)
        .expect_err("not linked");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn another_callers_preparation_cannot_be_started() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let mut stolen = request(&setup, "start-0001");
    stolen.caller = CallerId::new();
    let err = store.create_job(stolen).expect_err("not the preparer");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn preparation_requires_the_repository_owner() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let intruder = CallerId::new();
    let mut prep = setup.preparation.clone();
    prep.preparation_id = ultra_core::PreparationId::new();
    let err = store
        .put_preparation(&intruder, setup.repo, &prep)
        .expect_err("not the owner");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);

    let err = store
        .put_preparation(&setup.owner, ultra_core::RepoId::new(), &prep)
        .expect_err("unregistered repository");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);

    assert_eq!(
        store
            .get_repo(&intruder, setup.repo)
            .expect_err("foreign repo")
            .code(),
        ErrorCode::PolicyDenied
    );
    store
        .get_repo(&setup.owner, setup.repo)
        .expect("owner reads");
}

#[test]
fn grants_are_loaded_only_by_their_caller() {
    let (_tmp, root) = temp_root();
    let clock = FakeClock::at(T0);
    let store = open(&root, &clock);
    let setup = prepared(&store);
    let grant = ActionGrant {
        grant_id: GrantId::new(),
        caller: setup.owner,
        repo_id: setup.repo,
        job_id: None,
        snapshot_id: Some(setup.preparation.snapshot_id),
        scope_digest: Some(Digest::of(b"scope")),
        actions: BoundedVec::new(vec![ActionKind::ReadSnapshot]).expect("bounded"),
        policy_digest: Digest::of(b"policy"),
        issued_by: CallerId::new(),
        expires_at: Timestamp(T0 + DAY_MS),
    };
    store.put_grant(&grant).expect("stored");
    assert_eq!(
        store.load_grant(&setup.owner, grant.grant_id).expect("own"),
        grant
    );
    let err = store
        .load_grant(&CallerId::new(), grant.grant_id)
        .expect_err("foreign grant");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);

    // A grant id cannot be re-bound by storing it again.
    let mut hijack = grant.clone();
    hijack.caller = CallerId::new();
    let err = store.put_grant(&hijack).expect_err("ids are write-once");
    assert_eq!(err.code(), ErrorCode::InvalidInput);
}

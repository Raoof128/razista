//! T07: offline bundles, provenance and capability diagnosis.
//!
//! Run the doctor tests with `cargo test -p razista-cli --test cli doctor_`.

use std::path::{Path, PathBuf};

use razista_cli::doctor::{Doctor, DoctorConfig};
use razista_cli::prepare::{
    MANIFEST_FILE, PrepareRequest, PrepareSource, load_manifest, prepare_bundle, validate_bundle,
};
use razista_core::{
    BundleKind, CapabilityComponent, CapabilityReport, CapabilityStatus, ErrorCode, Ident,
    Language, Text, Timestamp,
};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
const T0: u64 = 1_800_000_000_000;

struct Workspace {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical");
        Self { _dir: dir, root }
    }

    fn file(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(&p, bytes).expect("write");
        p
    }
}

fn source(name: &str, kind: BundleKind, file: PathBuf) -> PrepareSource {
    PrepareSource {
        name: Ident::new(name).expect("ident"),
        kind,
        version: Text::new("1.0.0").expect("label"),
        licence: Text::new("Apache-2.0").expect("label"),
        origin: Text::new(format!("https://example.invalid/{name}")).expect("origin"),
        file,
        data_as_of: None,
    }
}

/// A prepared bundle with one entry of each kind, created at `T0`.
fn full_bundle(ws: &Workspace) -> PathBuf {
    full_bundle_at(ws, Timestamp(T0))
}

fn full_bundle_at(ws: &Workspace, created: Timestamp) -> PathBuf {
    let out = ws.root.join("bundle");
    let sources = vec![
        source(
            "qwen-test",
            BundleKind::ModelWeights,
            ws.file("in/model.gguf", b"GGUF weights"),
        ),
        source(
            "llama-cpp",
            BundleKind::InferenceRuntime,
            ws.file("in/llama-server", b"runtime"),
        ),
        source(
            "semgrep-rules",
            BundleKind::RulePack,
            ws.file("in/rules.yml", b"rules: []\n"),
        ),
        PrepareSource {
            data_as_of: Some(created),
            ..source(
                "osv-db",
                BundleKind::AdvisoryDatabase,
                ws.file("in/osv.zip", b"advisories"),
            )
        },
        source(
            "pypi-cache",
            BundleKind::LanguageDependencies,
            ws.file("in/wheels.tar", b"wheels"),
        ),
        source(
            "python-3-13",
            BundleKind::Toolchain,
            ws.file("in/python.tar", b"python"),
        ),
        source(
            "worker-image",
            BundleKind::ExecutionImage,
            ws.file("in/image.qcow2", b"image"),
        ),
        source(
            "semgrep",
            BundleKind::Scanner,
            ws.file("in/semgrep.tar", b"scanner"),
        ),
    ];
    prepare_bundle(
        &PrepareRequest {
            out_dir: out.clone(),
            sources,
            protected_roots: vec![],
        },
        created,
    )
    .expect("bundle prepared");
    out
}

fn rows(
    report: &CapabilityReport,
    component: CapabilityComponent,
) -> Vec<&razista_core::Capability> {
    report
        .components
        .iter()
        .filter(|c| c.component == component)
        .collect()
}

fn status_of(report: &CapabilityReport, name: &str) -> CapabilityStatus {
    report
        .components
        .iter()
        .find(|c| c.name.as_str() == name)
        .unwrap_or_else(|| panic!("no row named {name}: {:#?}", report.components))
        .status
}

fn doctor(bundle: Option<&Path>, now: u64) -> CapabilityReport {
    Doctor::new(
        DoctorConfig {
            bundle_dir: bundle.map(Path::to_path_buf),
            ..DoctorConfig::default()
        },
        Timestamp(now),
    )
    .inspect()
    .expect("doctor runs")
}

// ---------- prepare ------------------------------------------------------------

#[test]
fn prepare_records_provenance_and_licence() {
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let manifest = load_manifest(&bundle).expect("manifest");
    assert_eq!(manifest.entries.len(), 8);
    for e in manifest.entries.iter() {
        assert_eq!(e.licence.as_str(), "Apache-2.0");
        assert!(e.origin.as_str().starts_with("https://example.invalid/"));
        assert_eq!(e.created_at, Timestamp(T0));
        let bytes =
            std::fs::read(bundle.join(std::str::from_utf8(e.path.as_bytes()).expect("utf8")))
                .expect("bundled copy");
        assert_eq!(razista_core::Digest::of(&bytes), e.digest);
    }
    assert!(bundle.join(MANIFEST_FILE).is_file());
}

#[test]
fn prepare_refuses_entries_without_a_licence() {
    let ws = Workspace::new();
    for licence in ["unknown", "UNKNOWN", "NOASSERTION"] {
        let mut s = source("m", BundleKind::ModelWeights, ws.file("in/m.gguf", b"w"));
        s.licence = Text::new(licence).expect("label");
        let err = prepare_bundle(
            &PrepareRequest {
                out_dir: ws.root.join(format!("b-{licence}")),
                sources: vec![s],
                protected_roots: vec![],
            },
            Timestamp(T0),
        )
        .expect_err("licence record required");
        assert_eq!(err.code(), ErrorCode::InvalidInput, "{licence}");
    }
}

#[test]
fn prepare_has_no_repository_source_access() {
    let ws = Workspace::new();
    let repo = ws.root.join("projects/app");
    std::fs::create_dir_all(&repo).expect("mkdir");
    let inside = ws.file("projects/app/vendor/model.gguf", b"weights");

    // A source inside a protected repository is refused.
    let err = prepare_bundle(
        &PrepareRequest {
            out_dir: ws.root.join("bundle"),
            sources: vec![source("m", BundleKind::ModelWeights, inside)],
            protected_roots: vec![repo.clone()],
        },
        Timestamp(T0),
    )
    .expect_err("repository source refused");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);

    // So is writing the bundle into a repository.
    let err = prepare_bundle(
        &PrepareRequest {
            out_dir: repo.join("bundle"),
            sources: vec![source(
                "m",
                BundleKind::ModelWeights,
                ws.file("in/m.gguf", b"w"),
            )],
            protected_roots: vec![repo.clone()],
        },
        Timestamp(T0),
    )
    .expect_err("bundle output inside a repository refused");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);

    // A symlink pointing into the repository does not get around it.
    let link = ws.root.join("in/link.gguf");
    std::fs::create_dir_all(link.parent().expect("parent")).expect("mkdir");
    std::os::unix::fs::symlink(repo.join("vendor/model.gguf"), &link).expect("link");
    let err = prepare_bundle(
        &PrepareRequest {
            out_dir: ws.root.join("bundle2"),
            sources: vec![source("m", BundleKind::ModelWeights, link)],
            protected_roots: vec![repo],
        },
        Timestamp(T0),
    )
    .expect_err("links are not followed into repositories");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn prepare_refuses_a_non_empty_output_directory() {
    let ws = Workspace::new();
    let out = ws.root.join("bundle");
    std::fs::create_dir_all(&out).expect("mkdir");
    std::fs::write(out.join("stale"), b"x").expect("write");
    let err = prepare_bundle(
        &PrepareRequest {
            out_dir: out,
            sources: vec![source(
                "m",
                BundleKind::ModelWeights,
                ws.file("in/m.gguf", b"w"),
            )],
            protected_roots: vec![],
        },
        Timestamp(T0),
    )
    .expect_err("never overwrite");
    assert_eq!(err.code(), ErrorCode::InvalidInput);
}

// ---------- validation and doctor ------------------------------------------------

#[test]
fn doctor_missing_weight_never_downloads() {
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let manifest = load_manifest(&bundle).expect("manifest");
    let weights = manifest
        .entries
        .iter()
        .find(|e| e.kind == BundleKind::ModelWeights)
        .expect("weights entry");
    std::fs::remove_file(bundle.join(std::str::from_utf8(weights.path.as_bytes()).expect("utf8")))
        .expect("lose the weights");

    let report = validate_bundle(&bundle, &manifest, Timestamp(T0)).expect("validates");
    assert_eq!(status_of(&report, "qwen-test"), CapabilityStatus::Missing);
    let row = rows(&report, CapabilityComponent::Model)[0];
    assert!(
        row.detail.as_str().contains("DEPENDENCY_UNAVAILABLE"),
        "{}",
        row.detail
    );
    assert!(
        row.detail.as_str().contains("not downloaded"),
        "{}",
        row.detail
    );
    assert!(!report.offline_certified);
}

#[test]
fn doctor_no_network_client_in_dependency_tree() {
    // Structural guarantee behind "never downloads": the CLI's dependency
    // closure contains no HTTP or TLS client.
    let cargo = std::env::var("CARGO").expect("run under cargo");
    let out = std::process::Command::new(cargo)
        .args([
            "tree",
            "--offline",
            "--locked",
            "-p",
            "razista-cli",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect();
    assert!(
        names.iter().any(|n| n == "razista-store"),
        "tree resolved: {names:?}"
    );
    for forbidden in [
        "reqwest",
        "hyper",
        "ureq",
        "curl",
        "isahc",
        "attohttpc",
        "surf",
        "h2",
        "rustls",
        "native-tls",
        "openssl",
    ] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "razista-cli links {forbidden}; doctor/prepare must have no network client"
        );
    }
}

#[test]
fn doctor_stale_advisory_db_is_reported() {
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let manifest = load_manifest(&bundle).expect("manifest");

    let fresh = validate_bundle(&bundle, &manifest, Timestamp(T0 + 7 * DAY_MS)).expect("ok");
    assert_eq!(
        status_of(&fresh, "osv-db"),
        CapabilityStatus::Ready,
        "exactly 7 days is fresh"
    );

    let stale = validate_bundle(&bundle, &manifest, Timestamp(T0 + 7 * DAY_MS + 1)).expect("ok");
    assert_eq!(status_of(&stale, "osv-db"), CapabilityStatus::Stale);
    let row = stale
        .components
        .iter()
        .find(|c| c.name.as_str() == "osv-db")
        .expect("row");
    assert!(row.detail.as_str().contains("7 days"), "{}", row.detail);
    // Other kinds do not go stale.
    assert_eq!(status_of(&stale, "semgrep-rules"), CapabilityStatus::Ready);
}

#[test]
fn doctor_checksum_mismatch_blocks_use() {
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let manifest = load_manifest(&bundle).expect("manifest");
    let rules = manifest
        .entries
        .iter()
        .find(|e| e.name.as_str() == "semgrep-rules")
        .expect("entry");
    std::fs::write(
        bundle.join(std::str::from_utf8(rules.path.as_bytes()).expect("utf8")),
        b"rules: [tampered]\n",
    )
    .expect("tamper");
    let report = validate_bundle(&bundle, &manifest, Timestamp(T0)).expect("validates");
    assert_eq!(
        status_of(&report, "semgrep-rules"),
        CapabilityStatus::Invalid
    );
    let row = report
        .components
        .iter()
        .find(|c| c.name.as_str() == "semgrep-rules")
        .expect("row");
    assert!(row.detail.as_str().contains("checksum"), "{}", row.detail);
}

#[test]
fn doctor_bundle_entries_cannot_escape_the_bundle() {
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let mut manifest = load_manifest(&bundle).expect("manifest");
    // A link inside the bundle pointing elsewhere is not followed.
    let outside = ws.file("elsewhere/model.gguf", b"GGUF weights");
    let entry = manifest
        .entries
        .as_mut_slice()
        .iter_mut()
        .find(|e| e.name.as_str() == "qwen-test")
        .expect("entry");
    let rel = std::str::from_utf8(entry.path.as_bytes())
        .expect("utf8")
        .to_owned();
    std::fs::remove_file(bundle.join(&rel)).expect("rm");
    std::os::unix::fs::symlink(&outside, bundle.join(&rel)).expect("link");
    let report = validate_bundle(&bundle, &manifest, Timestamp(T0)).expect("validates");
    assert_eq!(status_of(&report, "qwen-test"), CapabilityStatus::Invalid);
}

#[test]
fn doctor_unknown_runtime_is_not_offline_certified() {
    let ws = Workspace::new();
    let out = ws.root.join("bundle");
    prepare_bundle(
        &PrepareRequest {
            out_dir: out.clone(),
            sources: vec![source(
                "ollama-localhost",
                BundleKind::InferenceRuntime,
                ws.file("in/ollama", b"some runtime"),
            )],
            protected_roots: vec![],
        },
        Timestamp(T0),
    )
    .expect("prepared");
    let manifest = load_manifest(&out).expect("manifest");
    let report = validate_bundle(&out, &manifest, Timestamp(T0)).expect("validates");
    assert_eq!(
        status_of(&report, "ollama-localhost"),
        CapabilityStatus::Unverified
    );
    assert!(!report.offline_certified);

    let report = doctor(Some(&out), T0);
    assert_eq!(
        status_of(&report, "ollama-localhost"),
        CapabilityStatus::Unverified
    );
    assert!(!report.offline_certified);
}

#[test]
fn doctor_never_claims_offline_certification_without_verified_egress() {
    // Even a complete, valid bundle is not an offline certificate: egress
    // enforcement and the sandbox must self-test first (T05/T08).
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let report = doctor(Some(&bundle), T0);
    assert!(!report.offline_certified);
    for row in rows(&report, CapabilityComponent::Egress) {
        assert_ne!(row.status, CapabilityStatus::Ready, "{row:?}");
    }
    for row in rows(&report, CapabilityComponent::Sandbox) {
        assert_ne!(row.status, CapabilityStatus::Ready, "{row:?}");
    }
}

#[test]
fn doctor_reports_each_component_separately() {
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let report = doctor(Some(&bundle), T0);
    for component in [
        CapabilityComponent::Model,
        CapabilityComponent::InferenceRuntime,
        CapabilityComponent::Sandbox,
        CapabilityComponent::Egress,
        CapabilityComponent::Compiler,
        CapabilityComponent::Packages,
        CapabilityComponent::Scanner,
        CapabilityComponent::Store,
        CapabilityComponent::Git,
    ] {
        assert!(
            !rows(&report, component).is_empty(),
            "{component:?} not reported"
        );
    }
    assert!(report.hardware.is_some());
}

#[test]
fn doctor_without_a_bundle_reports_missing_not_an_error() {
    let report = doctor(None, T0);
    for component in [
        CapabilityComponent::Model,
        CapabilityComponent::InferenceRuntime,
        CapabilityComponent::Scanner,
    ] {
        let r = rows(&report, component);
        assert!(!r.is_empty());
        assert!(
            r.iter().all(|c| c.status == CapabilityStatus::Missing),
            "{r:?}"
        );
    }
}

#[test]
fn doctor_unsupported_language_is_explicit() {
    let report = doctor(None, T0);
    let all = [
        Language::Python,
        Language::TypeScript,
        Language::JavaScript,
        Language::Rust,
        Language::Go,
        Language::Java,
        Language::Kotlin,
        Language::C,
        Language::Cpp,
        Language::CSharp,
        Language::Php,
        Language::Ruby,
        Language::Dart,
        Language::Swift,
        Language::Sql,
        Language::Terraform,
        Language::Yaml,
        Language::Dockerfile,
        Language::Shell,
    ];
    for lang in all {
        let rows: Vec<_> = report
            .languages
            .iter()
            .filter(|l| l.language == lang)
            .collect();
        assert_eq!(rows.len(), 1, "{lang:?} listed exactly once");
    }
    let swift = report
        .languages
        .iter()
        .find(|l| l.language == Language::Swift)
        .expect("swift row");
    assert!(
        !swift.execute && !swift.verify,
        "native Swift verification is not claimed"
    );
    // Nothing executes or verifies until the sandbox exists.
    assert!(report.languages.iter().all(|l| !l.execute && !l.verify));
}

#[test]
fn doctor_cli_offline_flag_prints_every_component() {
    let ws = Workspace::new();
    let bundle = full_bundle_at(&ws, Timestamp::now()); // the binary uses the real clock
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_razista"))
        .args(["doctor", "--offline", "--json", "--bundle"])
        .arg(&bundle)
        .env_clear()
        .output()
        .expect("razista runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: CapabilityReport = serde_json::from_slice(&out.stdout).expect("json report");
    assert!(!report.offline_certified);
    assert_eq!(
        status_of(&report, "qwen-test"),
        CapabilityStatus::Unverified,
        "a checksum is not a model self-test"
    );

    let human = std::process::Command::new(env!("CARGO_BIN_EXE_razista"))
        .args(["doctor", "--offline", "--bundle"])
        .arg(&bundle)
        .env_clear()
        .output()
        .expect("razista runs");
    let text = String::from_utf8_lossy(&human.stdout);
    for word in [
        "model",
        "inference_runtime",
        "sandbox",
        "egress",
        "compiler",
        "packages",
        "scanner",
        "store",
        "git",
    ] {
        assert!(text.contains(word), "human output lacks {word}:\n{text}");
    }
    assert!(text.contains("not offline-certified"), "{text}");
}

#[test]
fn doctor_cli_rejects_unknown_arguments() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_razista"))
        .args(["doctor", "--online"])
        .env_clear()
        .output()
        .expect("razista runs");
    assert_eq!(out.status.code(), Some(2));
}

// ---------- review findings ------------------------------------------------

fn prepare_one(ws: &Workspace, out: &str, s: PrepareSource, protect: Vec<PathBuf>) -> ultra_err::R {
    prepare_bundle(
        &PrepareRequest {
            out_dir: ws.root.join(out),
            sources: vec![s],
            protected_roots: protect,
        },
        Timestamp(T0),
    )
    .map(|_| ())
    .map_err(|e| e.code())
}

mod ultra_err {
    pub type R = Result<(), razista_core::ErrorCode>;
}

#[test]
fn prepare_refuses_an_unresolvable_protected_root() {
    // Finding 1: a mistyped --protect must not silently protect nothing.
    let ws = Workspace::new();
    let s = source("m", BundleKind::ModelWeights, ws.file("in/m.gguf", b"w"));
    assert_eq!(
        prepare_one(&ws, "bundle", s, vec![ws.root.join("projects/typo")]),
        Err(ErrorCode::InvalidInput)
    );
}

#[test]
fn prepare_auto_protects_git_repositories() {
    // Finding 1: a source or output inside any git checkout is refused even
    // when the operator forgot to list it.
    let ws = Workspace::new();
    std::fs::create_dir_all(ws.root.join("work/app/.git")).expect("mkdir");
    let inside = ws.file("work/app/models/m.gguf", b"w");
    assert_eq!(
        prepare_one(
            &ws,
            "bundle",
            source("m", BundleKind::ModelWeights, inside),
            vec![]
        ),
        Err(ErrorCode::PolicyDenied)
    );
    let outside = ws.file("in/m.gguf", b"w");
    assert_eq!(
        prepare_one(
            &ws,
            "work/app/bundle",
            source("m", BundleKind::ModelWeights, outside),
            vec![]
        ),
        Err(ErrorCode::PolicyDenied)
    );
}

#[test]
fn prepare_never_blocks_on_a_fifo_source() {
    // Finding 2.
    let ws = Workspace::new();
    let fifo = ws.root.join("in/pipe.gguf");
    std::fs::create_dir_all(fifo.parent().expect("parent")).expect("mkdir");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo")
            .success()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let root = ws.root.clone();
    std::thread::spawn(move || {
        let r = prepare_bundle(
            &PrepareRequest {
                out_dir: root.join("bundle"),
                sources: vec![source("m", BundleKind::ModelWeights, fifo)],
                protected_roots: vec![],
            },
            Timestamp(T0),
        );
        let _ = tx.send(r.map(|_| ()).map_err(|e| e.code()));
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("prepare returned instead of blocking on a FIFO");
    assert_eq!(result, Err(ErrorCode::InvalidInput));
    assert!(
        !ws.root.join("bundle").exists(),
        "nothing written on failure"
    );
}

#[test]
fn prepare_cli_accepts_a_relative_output_name() {
    // Finding 3, through the real binary and a spec file.
    let ws = Workspace::new();
    let model = ws.file("in/m.gguf", b"weights");
    let advisory = ws.file("in/osv.zip", b"advisories");
    let spec = format!(
        "[[source]]\nname = \"m\"\nkind = \"model_weights\"\nversion = \"1\"\nlicence = \"MIT\"\norigin = \"https://example.invalid/m\"\nfile = \"{}\"\n\n\
         [[source]]\nname = \"osv\"\nkind = \"advisory_database\"\nversion = \"2026-10-20\"\nlicence = \"CC-BY-4.0\"\norigin = \"https://example.invalid/osv\"\nfile = \"{}\"\ndata_as_of = {}\n",
        model.display(),
        advisory.display(),
        T0
    );
    std::fs::write(ws.root.join("spec.toml"), spec).expect("spec");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_razista"))
        .args(["prepare", "--spec", "spec.toml", "--out", "bundle"])
        .current_dir(&ws.root)
        .env_clear()
        .output()
        .expect("razista runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        load_manifest(&ws.root.join("bundle"))
            .expect("manifest")
            .entries
            .len(),
        2
    );
    assert!(
        std::fs::read_dir(&ws.root)
            .expect("list")
            .filter_map(Result::ok)
            .all(|e| !e.file_name().to_string_lossy().contains("partial")),
        "no temporary build directory left behind"
    );
}

#[test]
fn advisory_staleness_uses_the_data_date_not_the_copy_date() {
    // Finding 5: a year-old database copied today is stale today.
    let ws = Workspace::new();
    let s = PrepareSource {
        data_as_of: Some(Timestamp(T0 - 365 * DAY_MS)),
        ..source(
            "osv",
            BundleKind::AdvisoryDatabase,
            ws.file("in/osv.zip", b"db"),
        )
    };
    prepare_one(&ws, "bundle", s, vec![]).expect("prepared");
    let dir = ws.root.join("bundle");
    let report =
        validate_bundle(&dir, &load_manifest(&dir).expect("m"), Timestamp(T0)).expect("ok");
    assert_eq!(status_of(&report, "osv"), CapabilityStatus::Stale);
}

#[test]
fn prepare_requires_a_data_date_for_advisory_databases() {
    let ws = Workspace::new();
    let s = source(
        "osv",
        BundleKind::AdvisoryDatabase,
        ws.file("in/osv.zip", b"db"),
    );
    assert_eq!(
        prepare_one(&ws, "bundle", s, vec![]),
        Err(ErrorCode::InvalidInput)
    );
}

#[test]
fn doctor_marks_unproven_executables_unverified() {
    // Finding 6: Ready must not mean "only a checksum matched" for things
    // that have to run.
    let ws = Workspace::new();
    let bundle = full_bundle(&ws);
    let report = doctor(Some(&bundle), T0);
    for name in [
        "qwen-test",
        "llama-cpp",
        "semgrep",
        "python-3-13",
        "worker-image",
    ] {
        assert_eq!(
            status_of(&report, name),
            CapabilityStatus::Unverified,
            "{name}"
        );
    }
    for name in ["semgrep-rules", "osv-db", "pypi-cache"] {
        assert_eq!(
            status_of(&report, name),
            CapabilityStatus::Ready,
            "data: {name}"
        );
    }
}

#[test]
fn doctor_reports_a_missing_scanner_binary_even_with_rules() {
    let ws = Workspace::new();
    let s = source(
        "rules",
        BundleKind::RulePack,
        ws.file("in/rules.yml", b"rules: []\n"),
    );
    prepare_one(&ws, "bundle", s, vec![]).expect("prepared");
    let report = doctor(Some(&ws.root.join("bundle")), T0);
    let scanner_rows = rows(&report, CapabilityComponent::Scanner);
    assert!(
        scanner_rows
            .iter()
            .any(|r| r.status == CapabilityStatus::Missing && r.detail.as_str().contains("scanner")),
        "{scanner_rows:?}"
    );
}

#[test]
fn doctor_store_probe_is_read_only() {
    // Finding 7: the doctor never creates, migrates or recovers a store, and
    // does not disturb a running instance.
    let ws = Workspace::new();
    let typo = ws.root.join("no-such-store");
    let report = Doctor::new(
        DoctorConfig {
            store_dir: Some(typo.clone()),
            ..DoctorConfig::default()
        },
        Timestamp(T0),
    )
    .inspect()
    .expect("doctor runs");
    assert_eq!(
        rows(&report, CapabilityComponent::Store)[0].status,
        CapabilityStatus::Missing
    );
    assert!(!typo.exists(), "doctor created a store");

    let live_dir = ws.root.join("store");
    let live = razista_store::Store::open(&live_dir).expect("live store");
    let partial = live_dir.join("tmp").join("in-flight.partial");
    std::fs::write(&partial, b"being written").expect("in-flight file");
    let report = Doctor::new(
        DoctorConfig {
            store_dir: Some(live_dir.clone()),
            ..DoctorConfig::default()
        },
        Timestamp(T0),
    )
    .inspect()
    .expect("doctor runs");
    let row = rows(&report, CapabilityComponent::Store)[0];
    assert_eq!(row.status, CapabilityStatus::Ready, "{row:?}");
    assert!(row.detail.as_str().contains("in use"), "{row:?}");
    assert!(partial.exists(), "doctor ran recovery against a live store");
    live.put_artifact(
        &razista_core::CallerId::new(),
        b"still usable",
        razista_core::TrustLabel::ToolOutput,
    )
    .expect("live store unaffected");
}

#[test]
fn doctor_survives_bad_inputs_and_large_bundles() {
    // Finding 8: one bad row never aborts the report.
    let ws = Workspace::new();
    let report = Doctor::new(
        DoctorConfig {
            git: Some(PathBuf::from("/usr/bin/true")),
            ..DoctorConfig::default()
        },
        Timestamp(T0),
    )
    .inspect()
    .expect("doctor still reports");
    assert_eq!(
        rows(&report, CapabilityComponent::Git)[0].status,
        CapabilityStatus::Invalid
    );

    let sources: Vec<PrepareSource> = (0..300)
        .map(|i| {
            source(
                &format!("dep-{i}"),
                BundleKind::LanguageDependencies,
                ws.file(&format!("in/dep{i}.tar"), format!("dep {i}").as_bytes()),
            )
        })
        .collect();
    prepare_bundle(
        &PrepareRequest {
            out_dir: ws.root.join("big"),
            sources,
            protected_roots: vec![],
        },
        Timestamp(T0),
    )
    .expect("prepared");
    let report = doctor(Some(&ws.root.join("big")), T0);
    assert!(rows(&report, CapabilityComponent::Packages).len() >= 300);
}

// ---------- security review of the published commit ------------------------

#[test]
fn prepare_ignores_a_pre_planted_staging_directory() {
    // An attacker who can write to the output's parent pre-creates the old
    // predictable staging name with a planted file; it must not end up in
    // (or be used as) the bundle.
    let ws = Workspace::new();
    for pid_guess in [std::process::id(), 0, 1] {
        let planted = ws.root.join(format!(".bundle.partial-{pid_guess}"));
        std::fs::create_dir_all(planted.join("model_weights/m")).expect("plant");
        std::fs::write(planted.join("model_weights/m/evil.gguf"), b"planted").expect("plant");
    }
    let s = source(
        "m",
        BundleKind::ModelWeights,
        ws.file("in/m.gguf", b"genuine"),
    );
    prepare_one(&ws, "bundle", s, vec![]).expect("prepared");
    let bundle = ws.root.join("bundle");
    assert!(
        !bundle.join("model_weights/m/evil.gguf").exists(),
        "planted file in bundle"
    );
    assert_eq!(
        std::fs::read(bundle.join("model_weights/m/m.gguf")).expect("genuine copy"),
        b"genuine"
    );
}

#[test]
fn bundle_root_that_is_a_symlink_is_refused() {
    let ws = Workspace::new();
    let real = full_bundle(&ws);
    let link = ws.root.join("linked-bundle");
    std::os::unix::fs::symlink(&real, &link).expect("link");
    let err = load_manifest(&link).expect_err("symlinked bundle root");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

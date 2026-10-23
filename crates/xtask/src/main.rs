use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::audit_sources::{SourcePolicy, audit};

const USAGE: &str = "usage: cargo run --locked -p xtask -- <audit-sources|schemas>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("audit-sources") if args.len() == 1 => audit_sources(&workspace_root()),
        Some("schemas") if args.len() == 1 => write_schemas(&workspace_root()),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn audit_sources(root: &Path) -> ExitCode {
    let policy = match SourcePolicy::load(&root.join("source-policy.toml")) {
        Ok(policy) => policy,
        Err(e) => {
            eprintln!("audit-sources: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = match audit(root, &policy) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("audit-sources: {e}");
            return ExitCode::FAILURE;
        }
    };
    for v in &report.violations {
        eprintln!("R26 violation: {}: {}", v.path.display(), v.kind);
    }
    if report.violations.is_empty() {
        println!(
            "audit-sources: {} files checked, 0 violations (R26 Rust-only gate passed)",
            report.files_checked
        );
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "audit-sources: {} files checked, {} violations",
            report.files_checked,
            report.violations.len()
        );
        ExitCode::FAILURE
    }
}

/// Regenerate `schemas/*.schema.json` from the Rust contracts.
fn write_schemas(root: &Path) -> ExitCode {
    let dir = root.join("schemas");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("schemas: cannot create {}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    for (name, schema) in ultra_core::schema::all() {
        let path = dir.join(format!("{name}.schema.json"));
        if let Err(e) = std::fs::write(&path, ultra_core::schema::render(&schema)) {
            eprintln!("schemas: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    println!(
        "schemas: wrote {} contracts to {}",
        ultra_core::schema::all().len(),
        dir.display()
    );
    ExitCode::SUCCESS
}

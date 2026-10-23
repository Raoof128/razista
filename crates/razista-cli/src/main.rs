//! `razista` — local review engine CLI (T07 subset: `doctor`, `prepare`).

use std::path::PathBuf;
use std::process::ExitCode;

use razista_cli::doctor::{Doctor, DoctorConfig};
use razista_cli::prepare::{PrepareRequest, PrepareSource, prepare_bundle};
use razista_core::{BundleKind, CapabilityReport, Ident, Label, Text, Timestamp};
use serde::Deserialize;

const USAGE: &str = "\
usage:
  razista doctor [--offline] [--json] [--bundle DIR] [--store DIR] [--git PATH]
  razista prepare --spec FILE --out DIR [--protect REPO_ROOT]...";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("doctor") => doctor(&args[1..]),
        Some("prepare") => prepare(&args[1..]),
        _ => Err(Usage),
    };
    match result {
        Ok(code) => code,
        Err(Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

struct Usage;

/// Parse `--flag value` pairs and bare `--switch`es.
fn take_value(args: &[String], i: &mut usize) -> Result<String, Usage> {
    *i += 1;
    args.get(*i).cloned().ok_or(Usage)
}

fn doctor(args: &[String]) -> Result<ExitCode, Usage> {
    let mut config = DoctorConfig::default();
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            // The doctor is always offline; the flag states the intent.
            "--offline" => {}
            "--json" => json = true,
            "--bundle" => config.bundle_dir = Some(PathBuf::from(take_value(args, &mut i)?)),
            "--store" => config.store_dir = Some(PathBuf::from(take_value(args, &mut i)?)),
            "--git" => config.git = Some(PathBuf::from(take_value(args, &mut i)?)),
            _ => return Err(Usage),
        }
        i += 1;
    }
    let report = match Doctor::new(config, Timestamp::now()).inspect() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("razista doctor: {e}");
            return Ok(ExitCode::FAILURE);
        }
    };
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("razista doctor: {e}");
                return Ok(ExitCode::FAILURE);
            }
        }
    } else {
        print_report(&report);
    }
    Ok(ExitCode::SUCCESS)
}

fn name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn print_report(report: &CapabilityReport) {
    println!(
        "razista doctor: {}",
        if report.offline_certified {
            "offline-certified"
        } else {
            "not offline-certified (strict offline review is not available on this machine yet)"
        }
    );
    if let Some(h) = &report.hardware {
        println!(
            "hardware: {} {} | {} | {} CPUs | {} MiB RAM",
            h.os,
            h.arch,
            h.cpu,
            h.logical_cpus,
            h.memory_bytes / (1024 * 1024)
        );
    }
    println!();
    println!("{:<18} {:<11} {:<22} detail", "component", "status", "name");
    for c in report.components.iter() {
        println!(
            "{:<18} {:<11} {:<22} {}",
            name(&c.component),
            name(&c.status),
            c.name,
            c.detail
        );
    }
    println!();
    println!("languages (parse/context/analyse/execute/verify):");
    let flag = |b: bool| if b { "yes" } else { "no" };
    for l in report.languages.iter() {
        println!(
            "  {:<12} {}/{}/{}/{}/{}",
            name(&l.language),
            flag(l.parse),
            flag(l.context),
            flag(l.analyse),
            flag(l.execute),
            flag(l.verify)
        );
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    source: Vec<SpecSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecSource {
    name: Ident,
    kind: BundleKind,
    version: Label,
    licence: Label,
    origin: Text<1, 2048>,
    file: PathBuf,
    /// Milliseconds since the Unix epoch when the data was current.
    #[serde(default)]
    data_as_of: Option<u64>,
}

fn prepare(args: &[String]) -> Result<ExitCode, Usage> {
    let (mut spec, mut out, mut protect) = (None, None, Vec::new());
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--spec" => spec = Some(PathBuf::from(take_value(args, &mut i)?)),
            "--out" => out = Some(PathBuf::from(take_value(args, &mut i)?)),
            "--protect" => protect.push(PathBuf::from(take_value(args, &mut i)?)),
            _ => return Err(Usage),
        }
        i += 1;
    }
    let (Some(spec), Some(out)) = (spec, out) else {
        return Err(Usage);
    };
    let text = match std::fs::read_to_string(&spec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("razista prepare: cannot read {}: {e}", spec.display());
            return Ok(ExitCode::FAILURE);
        }
    };
    let parsed: Spec = match toml::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("razista prepare: invalid spec: {e}");
            return Ok(ExitCode::FAILURE);
        }
    };
    let request = PrepareRequest {
        out_dir: out,
        sources: parsed
            .source
            .into_iter()
            .map(|s| PrepareSource {
                name: s.name,
                kind: s.kind,
                version: s.version,
                licence: s.licence,
                origin: s.origin,
                file: s.file,
                data_as_of: s.data_as_of.map(Timestamp),
            })
            .collect(),
        protected_roots: protect,
    };
    match prepare_bundle(&request, Timestamp::now()) {
        Ok(m) => {
            println!(
                "razista prepare: bundled {} entries with provenance and licence records",
                m.entries.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            eprintln!("razista prepare: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

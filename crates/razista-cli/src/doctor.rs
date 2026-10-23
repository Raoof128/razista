//! `razista doctor`: what this machine can and cannot do, component by
//! component (model, inference runtime, sandbox, egress, compiler, packages,
//! scanner, store, git), plus per-language capabilities.
//!
//! The doctor only reads. It never downloads, never starts a model, never
//! runs repository code, and never creates, migrates or recovers a store. It
//! says "not offline-certified" until the model, runtime, sandbox and egress
//! enforcement have all passed their self-tests, which no build can do before
//! T05/T08 exist. One unreadable input becomes one row, never a failed report.

use std::path::{Path, PathBuf};

use razista_core::{
    BoundedVec, BundleKind, BundleManifest, Capability, CapabilityComponent, CapabilityReport,
    CapabilityStatus, HardwareProfile, Ident, Language, LanguageCapability, Result, Summary, Text,
    Timestamp,
};
use razista_repo::GitBinary;
use razista_store::Store as EngineStore;

use crate::prepare::{component_of, load_manifest, validate_bundle};

/// Every language the engine names, so unsupported ones are explicit.
pub const ALL_LANGUAGES: [Language; 19] = [
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

/// Every kind a complete offline bundle needs, with what it is for.
const REQUIRED_KINDS: [(BundleKind, &str, &str); 8] = [
    (BundleKind::ModelWeights, "model-weights", "model weights"),
    (
        BundleKind::InferenceRuntime,
        "inference-runtime",
        "managed inference runtime",
    ),
    (
        BundleKind::Toolchain,
        "toolchain",
        "worker toolchain (compilers/interpreters)",
    ),
    (
        BundleKind::LanguageDependencies,
        "dependencies",
        "language dependency cache",
    ),
    (BundleKind::Scanner, "scanner", "scanner binary"),
    (BundleKind::RulePack, "rule-pack", "scanner rule pack"),
    (
        BundleKind::AdvisoryDatabase,
        "advisory-database",
        "vulnerability advisory database",
    ),
    (
        BundleKind::ExecutionImage,
        "execution-image",
        "verification worker image",
    ),
];

/// Where git is looked for when none is configured. Never the PATH of a
/// reviewed repository.
const GIT_CANDIDATES: &[&str] = &[
    "/usr/bin/git",
    "/opt/homebrew/bin/git",
    "/usr/local/bin/git",
];

#[derive(Debug, Clone, Default)]
pub struct DoctorConfig {
    /// A prepared offline bundle (see `razista prepare`).
    pub bundle_dir: Option<PathBuf>,
    /// The engine's store directory.
    pub store_dir: Option<PathBuf>,
    /// The git binary to use for snapshots.
    pub git: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Doctor {
    config: DoctorConfig,
    now: Timestamp,
}

/// Build a row that cannot fail: names and details are made printable and
/// bounded instead of being rejected.
fn row(
    component: CapabilityComponent,
    name: &str,
    status: CapabilityStatus,
    detail: &str,
) -> Capability {
    let detail: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(1024)
        .collect();
    let detail = if detail.trim().is_empty() {
        "(no detail)".to_owned()
    } else {
        detail
    };
    Capability {
        component,
        name: Ident::new(name)
            .unwrap_or_else(|_| Ident::new("unnamed").unwrap_or_else(|_| unreachable_ident())),
        status,
        detail: Summary::new(detail).unwrap_or_else(|_| unreachable_summary()),
    }
}

// Constant inputs that always validate; kept out of line so `row` stays total.
fn unreachable_ident() -> Ident {
    Ident::new("x").unwrap_or_else(|_| panic!("constant identifier is valid"))
}
fn unreachable_summary() -> Summary {
    Summary::new("-").unwrap_or_else(|_| panic!("constant summary is valid"))
}

impl Doctor {
    pub fn new(config: DoctorConfig, now: Timestamp) -> Self {
        Self { config, now }
    }

    pub fn inspect(&self) -> Result<CapabilityReport> {
        use CapabilityComponent::*;
        use CapabilityStatus::*;
        let mut rows: Vec<Capability> = Vec::new();

        // Offline bundle contents.
        let manifest: Option<BundleManifest> = match &self.config.bundle_dir {
            None => None,
            Some(dir) => match load_manifest(dir) {
                Ok(m) => {
                    match validate_bundle(dir, &m, self.now) {
                        Ok(report) => rows.extend(report.components.into_inner()),
                        Err(e) => rows.push(row(
                            Model,
                            "offline-bundle",
                            Invalid,
                            &format!("offline bundle could not be validated: {}", e.message()),
                        )),
                    }
                    Some(m)
                }
                Err(e) => {
                    rows.push(row(
                        Model,
                        "offline-bundle",
                        Invalid,
                        &format!("offline bundle unusable: {}", e.message()),
                    ));
                    None
                }
            },
        };
        for (kind, name, what) in REQUIRED_KINDS {
            let present = manifest
                .as_ref()
                .is_some_and(|m| m.entries.iter().any(|e| e.kind == kind));
            if !present {
                rows.push(row(
                    component_of(kind),
                    &format!("{name}-missing"),
                    Missing,
                    &format!(
                        "no {what} in an offline bundle; prepare one with `razista prepare` (nothing is downloaded during review)"
                    ),
                ));
            }
        }

        // Execution isolation and egress enforcement are not built yet.
        let lima = [
            "/opt/homebrew/bin/limactl",
            "/usr/local/bin/limactl",
            "/usr/bin/limactl",
        ]
        .iter()
        .find(|p| Path::new(p).is_file());
        rows.push(match lima {
            Some(p) => row(
                Sandbox,
                "isolation-backend",
                Unverified,
                &format!("{p} found; no certified execution backend yet (T05); untrusted code is never run on the host"),
            ),
            None => row(
                Sandbox,
                "isolation-backend",
                Missing,
                "no execution backend found; untrusted code is never run on the host",
            ),
        });
        rows.push(row(
            Egress,
            "egress-enforcement",
            Unverified,
            "network egress enforcement for inference, scanners and workers is not implemented or verified yet (T05/T08); no offline guarantee is made",
        ));

        rows.push(self.store_row());

        // Git for snapshots.
        let git_path = self.config.git.clone().or_else(|| {
            GIT_CANDIDATES
                .iter()
                .map(PathBuf::from)
                .find(|p| p.is_file())
        });
        rows.push(match git_path.as_deref().map(GitBinary::at) {
            Some(Ok(git)) => row(Git, "git", Ready, git.version()),
            Some(Err(e)) => row(Git, "git", Invalid, e.message()),
            None => row(
                Git,
                "git",
                Missing,
                "no git binary found at a trusted location",
            ),
        });

        let ready = |c: CapabilityComponent| {
            let mine: Vec<_> = rows.iter().filter(|r| r.component == c).collect();
            !mine.is_empty() && mine.iter().all(|r| r.status == Ready)
        };
        let offline_certified = [Model, InferenceRuntime, Sandbox, Egress]
            .into_iter()
            .all(ready);

        // No parser, scanner or sandbox exists yet, so no language claims
        // any capability (R20). Each is listed so the absence is explicit.
        let languages = ALL_LANGUAGES
            .iter()
            .map(|&language| LanguageCapability {
                language,
                parse: false,
                context: false,
                analyse: false,
                execute: false,
                verify: false,
            })
            .collect();

        Ok(CapabilityReport {
            offline_certified,
            hardware: Some(hardware()),
            components: BoundedVec::new(rows)?,
            languages: BoundedVec::new(languages)?,
        })
    }

    fn store_row(&self) -> Capability {
        use CapabilityComponent::Store;
        use CapabilityStatus::*;
        let Some(dir) = &self.config.store_dir else {
            return row(Store, "store", Missing, "no store directory configured");
        };
        let probe = match EngineStore::probe(dir) {
            Ok(p) => p,
            Err(e) => {
                return row(
                    Store,
                    "store",
                    Unverified,
                    &format!("cannot inspect: {}", e.message()),
                );
            }
        };
        let schema = probe
            .schema_version
            .map_or_else(|| "schema unknown".to_owned(), |v| format!("schema v{v}"));
        let at_rest =
            "at rest: unverified (the store does not encrypt; volume encryption is not checked)";
        if !probe.exists {
            row(
                Store,
                "store",
                Missing,
                &format!("no store at {}", dir.display()),
            )
        } else if !probe.private {
            row(
                Store,
                "store",
                Invalid,
                &format!(
                    "{} is accessible to other users; the store refuses to open it",
                    dir.display()
                ),
            )
        } else if probe.in_use {
            row(
                Store,
                "store",
                Ready,
                &format!("in use by a running razista instance ({schema}); {at_rest}"),
            )
        } else {
            row(
                Store,
                "store",
                Ready,
                &format!("available ({schema}); {at_rest}"),
            )
        }
    }
}

fn sysctl(name: &str) -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", name])
        .env_clear()
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn meminfo_total() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text
        .lines()
        .find(|l| l.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kb * 1024)
}

fn label(s: &str) -> Text<1, 256> {
    let clean: String = s.chars().filter(|c| !c.is_control()).take(256).collect();
    Text::new(if clean.is_empty() {
        "unknown".into()
    } else {
        clean
    })
    .unwrap_or_else(|_| panic!("sanitised label is valid"))
}

fn hardware() -> HardwareProfile {
    let cpu = sysctl("machdep.cpu.brand_string").unwrap_or_default();
    let memory = sysctl("hw.memsize")
        .and_then(|s| s.parse().ok())
        .or_else(meminfo_total)
        .unwrap_or(0);
    HardwareProfile {
        os: label(std::env::consts::OS),
        arch: Ident::new(std::env::consts::ARCH).unwrap_or_else(|_| unreachable_ident()),
        cpu: label(&cpu),
        logical_cpus: std::thread::available_parallelism()
            .map(|n| u16::try_from(n.get()).unwrap_or(u16::MAX))
            .unwrap_or(1),
        memory_bytes: memory,
        accelerator: None,
    }
}

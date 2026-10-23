//! R26 first-party source-language audit.
//!
//! Every file under the repository root is classified. Rust sources and
//! declared non-executable data are allowed; any other language, any script
//! (shebang or executable bit), any symlink and any unknown file type is a
//! violation unless it sits under an exception root declared in
//! `source-policy.toml`. The classifier fails closed: a file it cannot place is
//! rejected, never ignored.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// Extensions of executable or compiled source languages. Listed so a
/// violation can say "this is another language" rather than "unknown"; both
/// are rejected.
const NON_RUST_SOURCE_EXTENSIONS: &[&str] = &[
    "py",
    "pyw",
    "pyi",
    "ipynb",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "ts",
    "mts",
    "cts",
    "tsx",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ksh",
    "csh",
    "ps1",
    "psm1",
    "psd1",
    "bat",
    "cmd",
    "vbs",
    "go",
    "rb",
    "pl",
    "pm",
    "php",
    "lua",
    "java",
    "kt",
    "kts",
    "scala",
    "groovy",
    "gradle",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hh",
    "m",
    "mm",
    "swift",
    "cs",
    "fs",
    "vb",
    "dart",
    "r",
    "jl",
    "ex",
    "exs",
    "erl",
    "hs",
    "ml",
    "clj",
    "nim",
    "zig",
    "v",
    "wasm",
    "awk",
    "sed",
    "tcl",
    "mk",
    "cmake",
    "nix",
    "applescript",
    "scpt",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    /// A source file in a language other than Rust.
    NonRustSource,
    /// A file whose type is not declared as Rust or data.
    UnknownFileType,
    /// A non-Rust file that starts with `#!`.
    ShebangScript,
    /// A file with an executable permission bit (Rust sources included).
    ExecutableFile,
    /// A Cargo configuration file, which can name runners and wrappers.
    CargoConfig,
    /// A symbolic link inside owned code.
    Symlink,
    /// A vendor exception whose inventory manifest does not exist.
    MissingVendorInventory,
    /// A CI workflow `run:` step that is not a Cargo command.
    CiRunsNonCargo { command: String },
    /// A CI step that executes code without a `run:` command (custom shell,
    /// script action, container action, local action).
    CiForbiddenStep { detail: String },
    /// A CI file that is not valid YAML.
    CiUnparseable,
}

impl fmt::Display for ViolationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonRustSource => f.write_str("first-party source in a non-Rust language"),
            Self::UnknownFileType => f.write_str("file type not declared as Rust or data"),
            Self::ShebangScript => f.write_str("script with a shebang line"),
            Self::ExecutableFile => f.write_str("file with an executable bit"),
            Self::CargoConfig => f.write_str("Cargo config can name runners and wrappers"),
            Self::Symlink => f.write_str("symbolic link in owned code"),
            Self::MissingVendorInventory => {
                f.write_str("vendor exception without its inventory manifest")
            }
            Self::CiRunsNonCargo { command } => {
                write!(f, "CI step runs a non-Cargo command: {command}")
            }
            Self::CiForbiddenStep { detail } => write!(f, "forbidden CI step: {detail}"),
            Self::CiUnparseable => f.write_str("CI file is not valid YAML"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Path relative to the audited root.
    pub path: PathBuf,
    pub kind: ViolationKind,
}

#[derive(Debug, Default)]
pub struct AuditReport {
    pub files_checked: usize,
    pub violations: Vec<Violation>,
}

#[derive(Debug)]
pub struct AuditError(pub String);

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AuditError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionKind {
    /// Untrusted reviewed-language fixture data, executed only in a sandbox.
    Fixture,
    /// Pinned third-party code with a separate inventory manifest.
    Vendor,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    pub path: String,
    pub kind: ExceptionKind,
    pub reason: String,
    #[serde(default)]
    pub inventory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePolicy {
    pub version: u32,
    pub rust_extensions: Vec<String>,
    pub data_extensions: Vec<String>,
    pub data_filenames: Vec<String>,
    pub ignored_dirs: Vec<String>,
    pub ci_workflow_dirs: Vec<String>,
    /// Third-party actions (`owner/name`) a workflow may `uses:`. Empty by default.
    #[serde(default)]
    pub ci_allowed_actions: Vec<String>,
    #[serde(default)]
    pub exceptions: Vec<Exception>,
}

impl SourcePolicy {
    pub fn from_toml(text: &str) -> Result<Self, AuditError> {
        let policy: Self =
            toml::from_str(text).map_err(|e| AuditError(format!("invalid source policy: {e}")))?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn load(path: &Path) -> Result<Self, AuditError> {
        let text = fs::read_to_string(path)
            .map_err(|e| AuditError(format!("cannot read {}: {e}", path.display())))?;
        Self::from_toml(&text)
    }

    fn validate(&self) -> Result<(), AuditError> {
        if self.version != 1 {
            return Err(AuditError(format!(
                "unsupported source policy version {}",
                self.version
            )));
        }
        for list in [&self.ignored_dirs, &self.ci_workflow_dirs] {
            for dir in list {
                require_relative(dir)?;
            }
        }
        for exception in &self.exceptions {
            require_relative(&exception.path)?;
            if exception.reason.trim().is_empty() {
                return Err(AuditError(format!(
                    "exception {:?} has no reason",
                    exception.path
                )));
            }
            match (exception.kind, &exception.inventory) {
                (ExceptionKind::Vendor, None) => {
                    return Err(AuditError(format!(
                        "vendor exception {:?} must name an inventory manifest",
                        exception.path
                    )));
                }
                (_, Some(inventory)) => require_relative(inventory)?,
                (ExceptionKind::Fixture, None) => {}
            }
        }
        Ok(())
    }

    fn exception_for(&self, rel: &Path) -> Option<&Exception> {
        self.exceptions
            .iter()
            .find(|e| rel.starts_with(Path::new(&e.path)))
    }

    fn is_ignored_dir(&self, rel: &Path) -> bool {
        self.ignored_dirs.iter().any(|d| rel == Path::new(d))
    }

    fn is_ci_workflow(&self, rel: &Path) -> bool {
        let yaml = matches!(extension(rel).as_deref(), Some("yml" | "yaml"));
        yaml && self
            .ci_workflow_dirs
            .iter()
            .any(|d| rel.starts_with(Path::new(d)))
    }
}

/// A policy path must be non-empty, relative and free of `..`, so an
/// exception can never widen beyond the repository.
fn require_relative(path: &str) -> Result<(), AuditError> {
    let p = Path::new(path);
    let ok = !path.is_empty()
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if ok {
        Ok(())
    } else {
        Err(AuditError(format!(
            "policy path {path:?} must be relative and stay inside the repository"
        )))
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Audit every file under `root` against `policy`. Violations are sorted by path.
pub fn audit(root: &Path, policy: &SourcePolicy) -> Result<AuditReport, AuditError> {
    let mut report = AuditReport::default();
    walk(root, Path::new(""), policy, &mut report)?;

    for exception in &policy.exceptions {
        if let Some(inventory) = &exception.inventory
            && !root.join(inventory).is_file()
        {
            report.violations.push(Violation {
                path: PathBuf::from(&exception.path),
                kind: ViolationKind::MissingVendorInventory,
            });
        }
    }

    report.violations.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(report)
}

fn walk(
    root: &Path,
    rel: &Path,
    policy: &SourcePolicy,
    report: &mut AuditReport,
) -> Result<(), AuditError> {
    let dir = root.join(rel);
    let mut entries = fs::read_dir(&dir)
        .map_err(|e| AuditError(format!("cannot list {}: {e}", dir.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AuditError(format!("cannot list {}: {e}", dir.display())))?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let child = rel.join(entry.file_name());
        let meta = fs::symlink_metadata(entry.path())
            .map_err(|e| AuditError(format!("cannot stat {}: {e}", child.display())))?;

        if meta.is_dir() {
            if !policy.is_ignored_dir(&child) {
                walk(root, &child, policy, report)?;
            }
            continue;
        }

        report.files_checked += 1;
        if policy.exception_for(&child).is_some() {
            continue;
        }
        if let Some(kind) = classify(root, &child, &meta, policy)? {
            report.violations.push(Violation { path: child, kind });
        }
    }
    Ok(())
}

fn classify(
    root: &Path,
    rel: &Path,
    meta: &fs::Metadata,
    policy: &SourcePolicy,
) -> Result<Option<ViolationKind>, AuditError> {
    if meta.file_type().is_symlink() {
        return Ok(Some(ViolationKind::Symlink));
    }
    if !meta.is_file() {
        return Ok(Some(ViolationKind::UnknownFileType));
    }

    let ext = extension(rel);
    if ext
        .as_deref()
        .is_some_and(|e| NON_RUST_SOURCE_EXTENSIONS.contains(&e))
    {
        return Ok(Some(ViolationKind::NonRustSource));
    }

    // Checked for every remaining file, `.rs` included: a Rust extension does
    // not make a shell script or an executable acceptable.
    let path = root.join(rel);
    if starts_with_shebang(&path)? {
        return Ok(Some(ViolationKind::ShebangScript));
    }
    if is_executable(meta) {
        return Ok(Some(ViolationKind::ExecutableFile));
    }
    if is_cargo_config(rel) {
        return Ok(Some(ViolationKind::CargoConfig));
    }

    if ext
        .as_deref()
        .is_some_and(|e| policy.rust_extensions.iter().any(|r| r == e))
    {
        return Ok(None);
    }

    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let is_data = policy.data_filenames.iter().any(|f| f == name)
        || ext
            .as_deref()
            .is_some_and(|e| policy.data_extensions.iter().any(|d| d == e));
    if !is_data {
        return Ok(Some(ViolationKind::UnknownFileType));
    }

    if policy.is_ci_workflow(rel) {
        let text = fs::read_to_string(&path)
            .map_err(|e| AuditError(format!("cannot read {}: {e}", rel.display())))?;
        return Ok(check_ci_yaml(&text, &policy.ci_allowed_actions));
    }
    Ok(None)
}

/// `#!` at the start of a file, except a Rust inner attribute (`#![`).
fn starts_with_shebang(path: &Path) -> Result<bool, AuditError> {
    let mut head = [0u8; 3];
    let mut file = fs::File::open(path)
        .map_err(|e| AuditError(format!("cannot open {}: {e}", path.display())))?;
    let mut n = 0;
    while n < head.len() {
        let read = file
            .read(&mut head[n..])
            .map_err(|e| AuditError(format!("cannot read {}: {e}", path.display())))?;
        if read == 0 {
            break;
        }
        n += read;
    }
    Ok(n >= 2 && head[..2] == *b"#!" && !(n == 3 && head[2] == b'['))
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

/// Cargo reads `.cargo/config[.toml]` from any ancestor directory, and it can
/// name runners, rustc wrappers and aliases that execute other programs.
fn is_cargo_config(rel: &Path) -> bool {
    let in_cargo_dir = rel
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d == ".cargo");
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    in_cargo_dir && (name == "config" || name == "config.toml")
}

/// Characters that let a `run:` line do more than invoke Cargo.
const SHELL_METACHARACTERS: &[char] = &[';', '&', '|', '`', '$', '<', '>', '\\', '(', ')'];

/// Parse a workflow or action file and return its first violation.
///
/// Every mapping is inspected wherever it occurs, so flow mappings, quoted
/// keys and folded/plain multi-line scalars are seen as the runner sees them.
fn check_ci_yaml(text: &str, allowed_actions: &[String]) -> Option<ViolationKind> {
    let Ok(docs) = yaml_rust2::YamlLoader::load_from_str(text) else {
        return Some(ViolationKind::CiUnparseable);
    };
    docs.iter().find_map(|doc| walk_ci(doc, allowed_actions))
}

fn walk_ci(node: &yaml_rust2::Yaml, allowed_actions: &[String]) -> Option<ViolationKind> {
    use yaml_rust2::Yaml;
    match node {
        Yaml::Hash(map) => map.iter().find_map(|(key, value)| {
            let verdict = match key.as_str() {
                Some("run") => check_run(value),
                Some("shell") => check_shell(value),
                Some("uses") => check_uses(value, allowed_actions),
                _ => None,
            };
            verdict.or_else(|| walk_ci(value, allowed_actions))
        }),
        Yaml::Array(items) => items.iter().find_map(|i| walk_ci(i, allowed_actions)),
        _ => None,
    }
}

/// A `run:` value must be one or more lines, each a plain Cargo invocation.
/// A mapping under `run` (e.g. `defaults.run`) is configuration and is walked.
fn check_run(value: &yaml_rust2::Yaml) -> Option<ViolationKind> {
    use yaml_rust2::Yaml;
    let script = match value {
        Yaml::Hash(_) => return None,
        Yaml::String(s) => s.as_str(),
        Yaml::Null => "",
        other => {
            return Some(ViolationKind::CiForbiddenStep {
                detail: format!("non-string run value {other:?}"),
            });
        }
    };
    let commands: Vec<&str> = script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if commands.is_empty() {
        return Some(ViolationKind::CiRunsNonCargo {
            command: String::new(),
        });
    }
    commands
        .into_iter()
        .find(|c| {
            let is_cargo = *c == "cargo" || c.starts_with("cargo ");
            !is_cargo || c.contains(SHELL_METACHARACTERS)
        })
        .map(|c| ViolationKind::CiRunsNonCargo {
            command: c.to_owned(),
        })
}

/// Only the runner's default shell is allowed; anything else is an
/// interpreter of the workflow's choosing.
fn check_shell(value: &yaml_rust2::Yaml) -> Option<ViolationKind> {
    match value.as_str() {
        Some("bash") => None,
        other => Some(ViolationKind::CiForbiddenStep {
            detail: format!("custom shell {other:?}"),
        }),
    }
}

/// `uses:` must name an allowlisted third-party action (`owner/name@ref`).
/// Script actions, container actions and local actions are never allowed.
fn check_uses(value: &yaml_rust2::Yaml, allowed_actions: &[String]) -> Option<ViolationKind> {
    let Some(reference) = value.as_str() else {
        return Some(ViolationKind::CiForbiddenStep {
            detail: "non-string uses value".into(),
        });
    };
    let name = reference.split('@').next().unwrap_or_default();
    if allowed_actions.iter().any(|a| a == name) {
        None
    } else {
        Some(ViolationKind::CiForbiddenStep {
            detail: format!("action {reference:?} is not allowlisted"),
        })
    }
}

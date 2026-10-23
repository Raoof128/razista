//! Offline bundle preparation and validation (design §13 "Offline dependency
//! preparation", R01, R21).
//!
//! `prepare` assembles a bundle from files the operator has already obtained
//! (with their own, explicitly approved tooling): it copies each into the
//! bundle, hashes it, and records version, licence and origin. It refuses to
//! read from or write into any protected repository, and it never fetches
//! anything. Review uses only what a validated bundle contains; a missing or
//! altered entry is a visible capability gap, never a download.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use razista_core::{
    BoundedVec, BundleEntry, BundleKind, BundleManifest, Capability, CapabilityComponent,
    CapabilityReport, CapabilityStatus, Digest, ErrorCode, Ident, Label, RepoPath, Result,
    ReviewError, Summary, Text, Timestamp,
};
use rustix::fs::{FileType, Mode, OFlags};
use rustix::io::Errno;

/// The manifest file at the bundle root.
pub const MANIFEST_FILE: &str = "bundle.json";

/// Advisory databases older than this are reported as stale (a warning,
/// never a trigger for network access).
pub const ADVISORY_MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// Inference runtimes the engine knows how to manage. Anything else is
/// recorded but never offline-certified.
pub const MANAGED_RUNTIMES: &[&str] = &["llama-cpp"];

const MANIFEST_MAX_BYTES: u64 = 16 * 1024 * 1024;
const CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

/// One operator-supplied file to place in the bundle.
#[derive(Debug, Clone)]
pub struct PrepareSource {
    pub name: Ident,
    pub kind: BundleKind,
    pub version: Label,
    pub licence: Label,
    pub origin: Text<1, 2048>,
    pub file: PathBuf,
    /// When the data was current; required for advisory databases.
    pub data_as_of: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct PrepareRequest {
    /// New or empty directory to create the bundle in.
    pub out_dir: PathBuf,
    pub sources: Vec<PrepareSource>,
    /// Registered repository roots. Preparation never reads from or writes
    /// into them (design: "runs without repository source mounted").
    pub protected_roots: Vec<PathBuf>,
}

fn denied(message: impl Into<String>) -> ReviewError {
    ReviewError::new(ErrorCode::PolicyDenied, message)
}

fn io_err(e: impl std::fmt::Display) -> ReviewError {
    ReviewError::invalid(e.to_string())
}

/// Licence values that are not a licence record.
fn licence_missing(licence: &str) -> bool {
    matches!(
        licence.trim().to_ascii_lowercase().as_str(),
        "" | "unknown" | "noassertion" | "none" | "unlicensed" | "tbd"
    )
}

/// `path` canonicalised; for a path that does not exist yet, its nearest
/// existing ancestor canonicalised with the rest appended.
fn canonical_or_planned(path: &Path) -> Result<PathBuf> {
    // A bare relative name ("bundle") has the empty path as its parent.
    let path = if path.parent() == Some(Path::new("")) {
        Path::new(".").join(path)
    } else {
        path.to_path_buf()
    };
    if let Ok(c) = path.canonicalize() {
        return Ok(c);
    }
    let mut rest = Vec::new();
    let mut cur = path.as_path();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        let name = cur.file_name().ok_or_else(|| {
            ReviewError::invalid(format!("{} has no existing ancestor", path.display()))
        })?;
        rest.push(name.to_owned());
        cur = cur
            .parent()
            .ok_or_else(|| ReviewError::invalid(format!("{} has no parent", path.display())))?;
    }
}

fn inside_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}

/// The nearest ancestor of `path` (itself included) that is a git checkout.
/// Any repository is protected, listed or not.
fn enclosing_git_checkout(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|a| fs::symlink_metadata(a.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

fn kind_dir(kind: BundleKind) -> &'static str {
    match kind {
        BundleKind::ModelWeights => "model_weights",
        BundleKind::InferenceRuntime => "inference_runtime",
        BundleKind::RulePack => "rule_pack",
        BundleKind::AdvisoryDatabase => "advisory_database",
        BundleKind::LanguageDependencies => "language_dependencies",
        BundleKind::ExecutionImage => "execution_image",
        BundleKind::Toolchain => "toolchain",
        BundleKind::Scanner => "scanner",
    }
}

#[cfg(unix)]
fn make_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(io_err)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(io_err)
}

/// Open a source without following a final link or blocking on a FIFO, and
/// confirm it is a regular file. The handle is what gets copied, so a path
/// swapped after this check cannot change what is read.
fn open_source(path: &Path) -> Result<fs::File> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| {
        ReviewError::invalid(format!("{} cannot be opened safely: {e}", path.display()))
    })?;
    let st = rustix::fs::fstat(&fd).map_err(io_err)?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return Err(ReviewError::invalid(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    Ok(fs::File::from(fd))
}

/// Assemble a bundle. Every source is checked and opened before anything is
/// written; the bundle is built in a temporary sibling directory and renamed
/// into place, so a failure never leaves a partial bundle behind.
pub fn prepare_bundle(request: &PrepareRequest, now: Timestamp) -> Result<BundleManifest> {
    let mut protected = Vec::with_capacity(request.protected_roots.len());
    for root in &request.protected_roots {
        protected.push(root.canonicalize().map_err(|e| {
            ReviewError::invalid(format!(
                "protected root {} cannot be resolved ({e}); refusing to guess what to protect",
                root.display()
            ))
        })?);
    }

    let out_dir = canonical_or_planned(&request.out_dir)?;
    if inside_any(&out_dir, &protected) {
        return Err(denied(format!(
            "bundle output {} is inside a protected repository",
            out_dir.display()
        )));
    }
    if let Some(repo) = enclosing_git_checkout(&out_dir) {
        return Err(denied(format!(
            "bundle output {} is inside the git checkout {}",
            out_dir.display(),
            repo.display()
        )));
    }
    if out_dir.exists() {
        let empty = fs::read_dir(&out_dir).map_err(io_err)?.next().is_none();
        if !out_dir.is_dir() || !empty {
            return Err(ReviewError::invalid(format!(
                "{} exists and is not an empty directory; refusing to overwrite",
                out_dir.display()
            )));
        }
    }

    let mut names = HashSet::new();
    let mut opened = Vec::with_capacity(request.sources.len());
    for s in &request.sources {
        if !names.insert(s.name.clone()) {
            return Err(ReviewError::invalid(format!(
                "duplicate bundle entry {}",
                s.name
            )));
        }
        if licence_missing(s.licence.as_str()) {
            return Err(ReviewError::invalid(format!(
                "{} has no licence record ({:?}); record the licence before bundling",
                s.name,
                s.licence.as_str()
            )));
        }
        if s.kind == BundleKind::AdvisoryDatabase && s.data_as_of.is_none() {
            return Err(ReviewError::invalid(format!(
                "{} is an advisory database; record when its data was current (data_as_of)",
                s.name
            )));
        }
        // Resolve links first, so a link into a repository is caught.
        let real = s.file.canonicalize().map_err(io_err)?;
        if inside_any(&real, &protected) {
            return Err(denied(format!(
                "source {} resolves into a protected repository; preparation never reads repository files",
                s.file.display()
            )));
        }
        if let Some(repo) = enclosing_git_checkout(&real) {
            return Err(denied(format!(
                "source {} is inside the git checkout {}; preparation never reads repository files",
                s.file.display(),
                repo.display()
            )));
        }
        let file = open_source(&real)?;
        let file_name = real
            .file_name()
            .ok_or_else(|| ReviewError::invalid("source has no file name"))?;
        let rel = format!(
            "{}/{}/{}",
            kind_dir(s.kind),
            s.name,
            file_name.to_string_lossy()
        );
        opened.push((s, file, RepoPath::from_bytes(rel.as_bytes())?));
    }

    let parent = out_dir
        .parent()
        .ok_or_else(|| ReviewError::invalid("bundle output has no parent directory"))?;
    let staging = parent.join(format!(
        ".{}.partial-{}",
        out_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let result = build_bundle(&staging, opened, now);
    match result {
        Ok(manifest) => {
            if let Err(e) = fs::rename(&staging, &out_dir) {
                let _ = fs::remove_dir_all(&staging);
                return Err(io_err(e));
            }
            Ok(manifest)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

fn build_bundle(
    staging: &Path,
    opened: Vec<(&PrepareSource, fs::File, RepoPath)>,
    now: Timestamp,
) -> Result<BundleManifest> {
    make_private_dir(staging)?;
    let mut entries = Vec::with_capacity(opened.len());
    for (s, file, rel) in opened {
        let dest = staging.join(Path::new(
            std::str::from_utf8(rel.as_bytes()).map_err(io_err)?,
        ));
        make_private_dir(
            dest.parent()
                .ok_or_else(|| ReviewError::invalid("no parent"))?,
        )?;
        let digest = copy_and_hash(file, &dest)?;
        entries.push(BundleEntry {
            name: s.name.clone(),
            kind: s.kind,
            path: rel,
            version: s.version.clone(),
            digest,
            licence: s.licence.clone(),
            origin: s.origin.clone(),
            created_at: now,
            data_as_of: s.data_as_of,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let manifest = BundleManifest {
        schema_version: 1,
        created_at: now,
        entries: BoundedVec::new(entries)?,
    };
    let mut file = create_private_file(&staging.join(MANIFEST_FILE))?;
    let json = serde_json::to_vec_pretty(&manifest).map_err(io_err)?;
    file.write_all(&json).map_err(io_err)?;
    file.sync_all().map_err(io_err)?;
    Ok(manifest)
}

fn copy_and_hash(mut src: fs::File, to: &Path) -> Result<Digest> {
    use sha2_stream::Sha256Writer;
    let mut dst = create_private_file(to)?;
    let mut hasher = Sha256Writer::default();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = src.read(&mut buf).map_err(io_err)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(io_err)?;
        hasher.update(&buf[..n]);
    }
    dst.sync_all().map_err(io_err)?;
    Ok(hasher.finish())
}

/// Streaming SHA-256 in the `sha256:<hex>` form used by [`Digest`].
mod sha2_stream {
    use razista_core::Digest;
    use sha2::{Digest as _, Sha256};

    #[derive(Default)]
    pub struct Sha256Writer(Sha256);

    impl Sha256Writer {
        pub fn update(&mut self, bytes: &[u8]) {
            self.0.update(bytes);
        }

        pub fn finish(self) -> Digest {
            let hex: String = self
                .0
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            // Always well-formed: 64 lowercase hex digits.
            Digest::parse(&format!("sha256:{hex}")).unwrap_or_else(|_| Digest::of(b""))
        }
    }
}

/// Open a bundle file without following links anywhere along its path.
fn open_bundle_file(root: &Path, rel: &Path) -> std::result::Result<fs::File, &'static str> {
    let mut prefix = root.to_path_buf();
    let parts: Vec<Component<'_>> = rel.components().collect();
    for (i, c) in parts.iter().enumerate() {
        let Component::Normal(name) = c else {
            return Err("path leaves the bundle");
        };
        prefix.push(name);
        match fs::symlink_metadata(&prefix) {
            Ok(m) if m.file_type().is_symlink() => return Err("path goes through a symbolic link"),
            Ok(m) if i + 1 < parts.len() && !m.is_dir() => return Err("missing"),
            Ok(_) => {}
            Err(_) => return Err("missing"),
        }
    }
    let fd = rustix::fs::open(
        &prefix,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| {
        if e == Errno::NOENT {
            "missing"
        } else {
            "cannot be opened safely"
        }
    })?;
    let st = rustix::fs::fstat(&fd).map_err(|_| "cannot be inspected")?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return Err("not a regular file");
    }
    Ok(fs::File::from(fd))
}

/// Read and parse `<dir>/bundle.json`.
pub fn load_manifest(dir: &Path) -> Result<BundleManifest> {
    let file = open_bundle_file(dir, Path::new(MANIFEST_FILE)).map_err(|why| {
        ReviewError::new(
            ErrorCode::DependencyUnavailable,
            format!(
                "bundle manifest {}: {why}",
                dir.join(MANIFEST_FILE).display()
            ),
        )
    })?;
    let mut text = String::new();
    file.take(MANIFEST_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io_err)?;
    if text.len() as u64 > MANIFEST_MAX_BYTES {
        return Err(ReviewError::invalid("bundle manifest is implausibly large"));
    }
    let manifest: BundleManifest = serde_json::from_str(&text)
        .map_err(|e| ReviewError::invalid(format!("bundle manifest is invalid: {e}")))?;
    if manifest.schema_version != 1 {
        return Err(ReviewError::invalid(format!(
            "unsupported bundle manifest version {}",
            manifest.schema_version
        )));
    }
    Ok(manifest)
}

pub(crate) fn component_of(kind: BundleKind) -> CapabilityComponent {
    match kind {
        BundleKind::ModelWeights => CapabilityComponent::Model,
        BundleKind::InferenceRuntime => CapabilityComponent::InferenceRuntime,
        BundleKind::RulePack | BundleKind::AdvisoryDatabase | BundleKind::Scanner => {
            CapabilityComponent::Scanner
        }
        BundleKind::LanguageDependencies => CapabilityComponent::Packages,
        BundleKind::Toolchain => CapabilityComponent::Compiler,
        BundleKind::ExecutionImage => CapabilityComponent::Sandbox,
    }
}

fn hash_file(file: fs::File) -> Result<Digest> {
    let mut hasher = sha2_stream::Sha256Writer::default();
    let mut reader = file;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(io_err)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish())
}

fn check_entry(dir: &Path, e: &BundleEntry, now: Timestamp) -> Result<(CapabilityStatus, String)> {
    use CapabilityStatus::*;
    if licence_missing(e.licence.as_str()) {
        return Ok((Invalid, "no licence record; entry blocked".into()));
    }
    let rel = Path::new(std::str::from_utf8(e.path.as_bytes()).map_err(io_err)?);
    let file = match open_bundle_file(dir, rel) {
        Ok(f) => f,
        Err("missing") => {
            return Ok((
                Missing,
                format!(
                    "DEPENDENCY_UNAVAILABLE: {} is not in the bundle; it is not downloaded during review (run `razista prepare`)",
                    e.path.display()
                ),
            ));
        }
        Err(why) => {
            return Ok((
                Invalid,
                format!("{}: {why}; entry blocked", e.path.display()),
            ));
        }
    };
    if hash_file(file)? != e.digest {
        return Ok((
            Invalid,
            format!("checksum mismatch for {}; entry blocked", e.path.display()),
        ));
    }
    if e.created_at.0 > now.0.saturating_add(CLOCK_SKEW_MS) {
        return Ok((
            Unverified,
            "recorded as created in the future; check the system clock".into(),
        ));
    }
    if e.kind == BundleKind::InferenceRuntime && !MANAGED_RUNTIMES.contains(&e.name.as_str()) {
        return Ok((
            Unverified,
            format!(
                "{} is not a managed runtime; it is never offline-certified",
                e.name
            ),
        ));
    }
    if e.kind == BundleKind::AdvisoryDatabase {
        let Some(as_of) = e.data_as_of else {
            return Ok((
                Unverified,
                "advisory database has no recorded data date; its freshness is unknown".into(),
            ));
        };
        let age = now.0.saturating_sub(as_of.0);
        if age > ADVISORY_MAX_AGE_MS {
            let days = age / (24 * 60 * 60 * 1000);
            return Ok((
                Stale,
                format!(
                    "advisory database is {days} days old (older than 7 days); scans will state its age; nothing is refreshed automatically"
                ),
            ));
        }
    }
    let summary = format!(
        "{} {} ({}), checksum verified",
        e.name, e.version, e.licence
    );
    // Anything that has to run is only ready after its own self-test, which
    // belongs to the task that makes it run.
    let pending = match e.kind {
        BundleKind::ModelWeights | BundleKind::InferenceRuntime => Some("load and self-test (T08)"),
        BundleKind::ExecutionImage => Some("isolation self-test (T05)"),
        BundleKind::Toolchain => Some("in-sandbox self-test (T05/T10)"),
        BundleKind::Scanner => Some("in-sandbox scanner self-test (T10)"),
        BundleKind::RulePack | BundleKind::AdvisoryDatabase | BundleKind::LanguageDependencies => {
            None
        }
    };
    Ok(match pending {
        Some(what) => (Unverified, format!("{summary}; {what} not implemented yet")),
        None => (Ready, summary),
    })
}

/// Check every entry of `manifest` against the files under `dir`.
///
/// A bundle alone never makes a system offline-certified; that also needs
/// verified egress enforcement and a certified sandbox.
pub fn validate_bundle(
    dir: &Path,
    manifest: &BundleManifest,
    now: Timestamp,
) -> Result<CapabilityReport> {
    let mut components = Vec::with_capacity(manifest.entries.len());
    for e in manifest.entries.iter() {
        let (status, detail) = check_entry(dir, e, now)?;
        let detail: String = detail.chars().take(1024).collect();
        components.push(Capability {
            component: component_of(e.kind),
            name: e.name.clone(),
            status,
            detail: Summary::new(detail)?,
        });
    }
    Ok(CapabilityReport {
        offline_certified: false,
        hardware: None,
        components: BoundedVec::new(components)?,
        languages: BoundedVec::empty(),
    })
}

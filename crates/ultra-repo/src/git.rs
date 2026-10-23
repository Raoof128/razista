//! A hardened, read-only git runner.
//!
//! Git is a pinned third-party program. It is never run with the user's
//! environment: the environment is cleared, global and system config are
//! disabled, `HOME` is an empty private directory, and command-line config
//! (which outranks repository config) neutralises every setting that can
//! launch a program: hooks, fsmonitor, pagers, credential helpers, ssh,
//! external diff and alternate-refs commands. Only plumbing that reads the
//! object database and index is used, never porcelain that consults the
//! working tree through filters or textconv, and optional locks are off so
//! nothing (not even the index stat cache) is written. Network transports are
//! disallowed and lazy fetches of missing objects are disabled.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ultra_core::{ErrorCode, ObjectId, Result, ReviewError};

const MAX_STDOUT: usize = 256 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

/// Config forced on every invocation. `-c` beats repository config.
const FORCED_CONFIG: &[&str] = &[
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "core.untrackedCache=false",
    "core.pager=cat",
    "core.askPass=",
    "core.sshCommand=false",
    "core.alternateRefsCommand=",
    "core.alternateRefsPrefixes=",
    "core.quotePath=false",
    "credential.helper=",
    "protocol.allow=never",
    "diff.external=",
    "gc.auto=0",
    "maintenance.auto=false",
    "submodule.recurse=false",
    "fetch.recurseSubmodules=false",
    "safe.bareRepository=explicit",
];

/// The git program the engine trusts. Never resolved from a repository's PATH.
#[derive(Debug, Clone)]
pub struct GitBinary {
    path: PathBuf,
    version: String,
}

impl GitBinary {
    /// Use the git at `path`, which must be an absolute path to a file.
    pub fn at(path: &Path) -> Result<Self> {
        if !path.is_absolute() || !path.is_file() {
            return Err(ReviewError::new(
                ErrorCode::DependencyUnavailable,
                format!("git must be an absolute path to a file: {}", path.display()),
            ));
        }
        let out = Command::new(path)
            .arg("--version")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| ReviewError::new(ErrorCode::DependencyUnavailable, e.to_string()))?;
        if !out.status.success() {
            return Err(ReviewError::new(
                ErrorCode::DependencyUnavailable,
                "git --version failed",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            version: String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

/// One tree or index entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitEntry {
    pub mode: u32,
    pub oid: String,
    /// Sparse checkout: the index entry is not expected in the working tree.
    pub skip_worktree: bool,
}

pub(crate) const MODE_GITLINK: u32 = 0o160000;
pub(crate) const MODE_SYMLINK: u32 = 0o120000;
pub(crate) const MODE_EXECUTABLE: u32 = 0o100755;

/// A runner bound to one repository.
pub(crate) struct Git<'a> {
    bin: &'a GitBinary,
    git_dir: PathBuf,
    work_tree: PathBuf,
    home: tempfile::TempDir,
}

fn git_failed(what: &str, stderr: &[u8]) -> ReviewError {
    ReviewError::invalid(format!(
        "git {what} failed: {}",
        String::from_utf8_lossy(stderr).trim()
    ))
}

impl<'a> Git<'a> {
    pub fn new(bin: &'a GitBinary, git_dir: PathBuf, work_tree: PathBuf) -> Result<Self> {
        let home = tempfile::Builder::new()
            .prefix("ultra-git-home-")
            .tempdir()
            .map_err(|e| ReviewError::new(ErrorCode::StoreUnavailable, e.to_string()))?;
        Ok(Self {
            bin,
            git_dir,
            work_tree,
            home,
        })
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.bin.path);
        cmd.current_dir(&self.work_tree)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path())
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.work_tree)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_PROTOCOL_FROM_USER", "0")
            .env("GIT_ALLOW_PROTOCOL", "none")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C");
        for kv in FORCED_CONFIG {
            cmd.args(["-c", kv]);
        }
        cmd.args([
            "--no-replace-objects",
            "--no-lazy-fetch",
            "--literal-pathspecs",
        ]);
        cmd.args(args);
        cmd
    }

    /// Run with optional stdin; return stdout on success.
    fn run(&self, what: &str, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>> {
        let mut cmd = self.command(args);
        cmd.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| ReviewError::new(ErrorCode::DependencyUnavailable, e.to_string()))?;

        let writer = match (input, child.stdin.take()) {
            (Some(bytes), Some(mut stdin)) => Some(thread::spawn(move || {
                let _ = stdin.write_all(&bytes);
            })),
            _ => None,
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = thread::spawn(move || read_capped(stdout, MAX_STDOUT));
        let err_reader = thread::spawn(move || read_capped(stderr, MAX_STDERR));

        let status = wait_with_deadline(&mut child, TIMEOUT)?;
        if let Some(w) = writer {
            let _ = w.join();
        }
        let (stdout, overflowed) = out_reader.join().unwrap_or_default();
        let (stderr, _) = err_reader.join().unwrap_or_default();
        if overflowed {
            return Err(ReviewError::new(
                ErrorCode::BudgetExhausted,
                format!("git {what} produced more than {MAX_STDOUT} bytes"),
            ));
        }
        match status {
            Some(s) if s.success() => Ok(stdout),
            Some(_) => Err(git_failed(what, &stderr)),
            None => Err(ReviewError::new(
                ErrorCode::WorkerTimeout,
                format!("git {what} exceeded {}s and was killed", TIMEOUT.as_secs()),
            )),
        }
    }

    /// Resolve `rev` to a commit, or `BASE_UNAVAILABLE`. Never fetches.
    pub fn resolve_commit(&self, rev: &str) -> Result<ObjectId> {
        let spec = format!("{rev}^{{commit}}");
        let out = self
            .run(
                "rev-parse",
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &spec,
                ],
                None,
            )
            .map_err(|_| {
                ReviewError::new(
                    ErrorCode::BaseUnavailable,
                    format!("revision {rev:?} is not available locally; nothing was fetched"),
                )
            })?;
        ObjectId::new(String::from_utf8_lossy(&out).trim().to_owned())
    }

    pub fn merge_base(&self, a: &ObjectId, b: &ObjectId) -> Result<ObjectId> {
        let out = self
            .run(
                "merge-base",
                &[
                    "merge-base",
                    "--all",
                    "--end-of-options",
                    a.as_str(),
                    b.as_str(),
                ],
                None,
            )
            .map_err(|_| {
                ReviewError::new(
                    ErrorCode::BaseUnavailable,
                    "the base and head share no local merge base",
                )
            })?;
        let text = String::from_utf8_lossy(&out);
        let bases: Vec<&str> = text.split_whitespace().collect();
        match bases.as_slice() {
            [one] => ObjectId::new((*one).to_owned()),
            [] => Err(ReviewError::new(
                ErrorCode::BaseUnavailable,
                "the base and head share no merge base",
            )),
            _ => Err(ReviewError::invalid(
                "the branch has several merge bases; choose an explicit range instead",
            )),
        }
    }

    pub fn parents(&self, commit: &ObjectId) -> Result<Vec<ObjectId>> {
        let out = self.run(
            "rev-list",
            &[
                "rev-list",
                "--no-walk",
                "--parents",
                "--end-of-options",
                commit.as_str(),
            ],
            None,
        )?;
        String::from_utf8_lossy(&out)
            .split_whitespace()
            .skip(1)
            .map(|p| ObjectId::new(p.to_owned()))
            .collect()
    }

    /// Recursive tree listing of `commit`: path bytes -> entry.
    pub fn ls_tree(&self, commit: &ObjectId) -> Result<HashMap<Vec<u8>, GitEntry>> {
        let out = self.run(
            "ls-tree",
            &[
                "ls-tree",
                "-r",
                "-z",
                "--full-tree",
                "--end-of-options",
                commit.as_str(),
            ],
            None,
        )?;
        let mut map = HashMap::new();
        for record in out.split(|&b| b == 0).filter(|r| !r.is_empty()) {
            // "<mode> <type> <oid>\t<path>"
            let (meta, path) = split_tab(record)?;
            let fields: Vec<&str> = std::str::from_utf8(meta)
                .map_err(|_| ReviewError::invalid("malformed ls-tree output"))?
                .split(' ')
                .collect();
            let [mode, _kind, oid] = fields.as_slice() else {
                return Err(ReviewError::invalid("malformed ls-tree record"));
            };
            map.insert(path.to_vec(), entry(mode, oid)?);
        }
        Ok(map)
    }

    /// Index listing: path bytes -> entry. Unmerged entries are refused.
    pub fn ls_index(&self) -> Result<HashMap<Vec<u8>, GitEntry>> {
        let out = self.run("ls-files", &["ls-files", "-t", "--stage", "-z"], None)?;
        let mut map = HashMap::new();
        for record in out.split(|&b| b == 0).filter(|r| !r.is_empty()) {
            // "<tag> <mode> <oid> <stage>\t<path>"; tag S = skip-worktree
            let (meta, path) = split_tab(record)?;
            let fields: Vec<&str> = std::str::from_utf8(meta)
                .map_err(|_| ReviewError::invalid("malformed ls-files output"))?
                .split(' ')
                .collect();
            let [tag, mode, oid, stage] = fields.as_slice() else {
                return Err(ReviewError::invalid("malformed ls-files record"));
            };
            if *stage != "0" {
                return Err(ReviewError::invalid(
                    "the index has unresolved merge conflicts; resolve them before review",
                ));
            }
            let mut e = entry(mode, oid)?;
            e.skip_worktree = *tag == "S";
            map.insert(path.to_vec(), e);
        }
        Ok(map)
    }

    /// Untracked, non-ignored paths.
    pub fn ls_untracked(&self) -> Result<Vec<Vec<u8>>> {
        let out = self.run(
            "ls-files",
            &["ls-files", "--others", "--exclude-standard", "-z"],
            None,
        )?;
        Ok(out
            .split(|&b| b == 0)
            .filter(|r| !r.is_empty())
            .map(<[u8]>::to_vec)
            .collect())
    }

    /// Object sizes for `oids` (blobs only; others are absent from the map).
    pub fn blob_sizes(&self, oids: &[&str]) -> Result<HashMap<String, u64>> {
        if oids.is_empty() {
            return Ok(HashMap::new());
        }
        let input = oids
            .iter()
            .flat_map(|o| format!("{o}\n").into_bytes())
            .collect();
        let out = self.run(
            "cat-file",
            &[
                "cat-file",
                "--batch-check=%(objectname) %(objecttype) %(objectsize)",
            ],
            Some(input),
        )?;
        let mut sizes = HashMap::new();
        for line in String::from_utf8_lossy(&out).lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            if let [oid, "blob", size] = parts.as_slice()
                && let Ok(n) = size.parse()
            {
                sizes.insert((*oid).to_owned(), n);
            }
        }
        Ok(sizes)
    }

    /// Raw blob contents for `oids`. No filters, no textconv.
    pub fn blobs(&self, oids: &[&str]) -> Result<HashMap<String, Vec<u8>>> {
        if oids.is_empty() {
            return Ok(HashMap::new());
        }
        let input = oids
            .iter()
            .flat_map(|o| format!("{o}\n").into_bytes())
            .collect();
        let out = self.run("cat-file", &["cat-file", "--batch"], Some(input))?;
        let mut blobs = HashMap::new();
        let mut rest = out.as_slice();
        while !rest.is_empty() {
            let nl = rest
                .iter()
                .position(|&b| b == b'\n')
                .ok_or_else(|| ReviewError::invalid("truncated cat-file output"))?;
            let header = String::from_utf8_lossy(&rest[..nl]).into_owned();
            rest = &rest[nl + 1..];
            let parts: Vec<&str> = header.split(' ').collect();
            let [oid, _kind, size] = parts.as_slice() else {
                return Err(ReviewError::invalid(format!(
                    "unexpected cat-file header {header:?}"
                )));
            };
            let size: usize = size
                .parse()
                .map_err(|_| ReviewError::invalid("bad cat-file size"))?;
            if rest.len() < size + 1 {
                return Err(ReviewError::invalid("truncated cat-file content"));
            }
            blobs.insert((*oid).to_owned(), rest[..size].to_vec());
            rest = &rest[size + 1..];
        }
        Ok(blobs)
    }
}

fn split_tab(record: &[u8]) -> Result<(&[u8], &[u8])> {
    let tab = record
        .iter()
        .position(|&b| b == b'\t')
        .ok_or_else(|| ReviewError::invalid("malformed git record"))?;
    Ok((&record[..tab], &record[tab + 1..]))
}

fn entry(mode: &str, oid: &str) -> Result<GitEntry> {
    let mode =
        u32::from_str_radix(mode, 8).map_err(|_| ReviewError::invalid("bad git file mode"))?;
    Ok(GitEntry {
        mode,
        oid: oid.to_owned(),
        skip_worktree: false,
    })
}

fn read_capped<R: Read>(source: Option<R>, cap: usize) -> (Vec<u8>, bool) {
    let Some(mut source) = source else {
        return (Vec::new(), false);
    };
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut overflowed = false;
    loop {
        match source.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if buf.len() + n > cap {
                    overflowed = true;
                    // Keep draining so the child is not blocked on a full pipe.
                    continue;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
    (buf, overflowed)
}

/// Wait for `child`, killing it at the deadline. `None` means it was killed.
fn wait_with_deadline(
    child: &mut Child,
    limit: Duration,
) -> Result<Option<std::process::ExitStatus>> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => {
                return Err(ReviewError::new(ErrorCode::WorkerTimeout, e.to_string()));
            }
        }
    }
}

/// A revision as typed by a caller, limited to characters git revision
/// syntax needs, never starting with `-`.
pub(crate) fn check_revision(rev: &str) -> Result<()> {
    let ok = !rev.is_empty()
        && !rev.starts_with('-')
        && rev.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'.' | b'_' | b'/' | b'@' | b'^' | b'~' | b'{' | b'}' | b'-'
                )
        });
    if ok {
        Ok(())
    } else {
        Err(ReviewError::invalid(format!(
            "revision {rev:?} contains characters outside [A-Za-z0-9._/@^~{{}}-] or starts with '-'"
        )))
    }
}

pub(crate) fn mode_is_executable(mode: u32) -> bool {
    mode == MODE_EXECUTABLE
}

/// Conversion-relevant attributes of one path (values as git prints them:
/// `set`, `unset`, `unspecified`, or a value).
#[derive(Debug, Default, Clone)]
pub(crate) struct Attrs {
    pub text: String,
    pub eol: String,
    pub crlf: String,
    pub filter: String,
    pub working_tree_encoding: String,
    pub ident: String,
}

fn specified(v: &str) -> bool {
    !v.is_empty() && v != "unspecified" && v != "unset"
}

impl Attrs {
    /// A filter, encoding or ident conversion git would apply and the engine
    /// does not reproduce.
    pub fn opaque_conversion(&self) -> Option<&'static str> {
        if specified(&self.filter) {
            Some("filter")
        } else if specified(&self.working_tree_encoding) {
            Some("working-tree-encoding")
        } else if self.ident == "set" {
            Some("ident")
        } else {
            None
        }
    }

    /// Whether git's built-in CRLF -> LF clean conversion applies.
    pub fn eol_normalised(&self, autocrlf: Option<&str>) -> bool {
        if self.text == "unset" || self.crlf == "unset" {
            return false;
        }
        if self.text == "set" || self.crlf == "set" || specified(&self.eol) {
            return true;
        }
        let auto = self.text == "auto" || self.crlf == "input";
        let autocrlf_on = matches!(autocrlf, Some("true" | "input"));
        auto || (self.text == "unspecified" && autocrlf_on)
    }
}

impl Git<'_> {
    /// Attributes for `paths` from `.gitattributes` and `info/attributes`.
    /// A pure read: no driver named by an attribute is run.
    pub fn check_attrs(&self, paths: &[&[u8]]) -> Result<HashMap<Vec<u8>, Attrs>> {
        if paths.is_empty() {
            return Ok(HashMap::new());
        }
        let mut input = Vec::new();
        for p in paths {
            input.extend_from_slice(p);
            input.push(0);
        }
        let out = self.run(
            "check-attr",
            &[
                "check-attr",
                "-z",
                "--stdin",
                "text",
                "eol",
                "crlf",
                "filter",
                "working-tree-encoding",
                "ident",
            ],
            Some(input),
        )?;
        let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
        let mut map: HashMap<Vec<u8>, Attrs> = HashMap::new();
        for triple in fields.chunks(3) {
            let [path, attr, value] = triple else {
                continue;
            };
            let a = map.entry(path.to_vec()).or_default();
            let value = String::from_utf8_lossy(value).into_owned();
            match *attr {
                b"text" => a.text = value,
                b"eol" => a.eol = value,
                b"crlf" => a.crlf = value,
                b"filter" => a.filter = value,
                b"working-tree-encoding" => a.working_tree_encoding = value,
                b"ident" => a.ident = value,
                _ => {}
            }
        }
        Ok(map)
    }

    /// One config value from the repository's own config (global and system
    /// config are disabled). A pure read.
    pub fn config_value(&self, key: &str) -> Result<Option<String>> {
        match self.run("config", &["config", "--get", key], None) {
            Ok(out) => Ok(Some(String::from_utf8_lossy(&out).trim().to_lowercase())),
            Err(_) => Ok(None),
        }
    }
}

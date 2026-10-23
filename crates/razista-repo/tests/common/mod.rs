//! Synthetic repositories for the snapshot tests, built by driving git.
//!
//! Setup uses its own git invocations with global/system config disabled and
//! hooks pointed at /dev/null, so hostile configuration planted *after* the
//! commits never fires during setup. Only the code under test reads it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use razista_core::{
    BoundedVec, CallerId, Digest, RepoGrant, RepoId, RepoPath, ReviewScope, ScopeMode, Snapshot,
    Text, Timestamp,
};
use razista_repo::{CaptureOptions, GitBinary, Snapshotter};
use razista_store::Store;

/// The git used for setup and handed to the code under test.
pub fn git_path() -> PathBuf {
    for candidate in [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
    ] {
        let p = Path::new(candidate);
        if p.is_file() {
            return p.to_path_buf();
        }
    }
    panic!("no git binary found for tests");
}

pub struct Repo {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub outside: PathBuf,
    home: PathBuf,
}

impl Repo {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        // Canonical, because macOS temp dirs sit behind the /var -> /private/var link.
        let base = dir.path().canonicalize().expect("canonical");
        let root = base.join("repo");
        let outside = base.join("outside");
        let home = base.join("home");
        for d in [&root, &outside, &home] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        let repo = Self {
            _dir: dir,
            root,
            outside,
            home,
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    /// An empty directory (no repository yet), e.g. as a clone target.
    pub fn empty() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().canonicalize().expect("canonical");
        let root = base.join("repo");
        let outside = base.join("outside");
        let home = base.join("home");
        for d in [&root, &outside, &home] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        Self {
            _dir: dir,
            root,
            outside,
            home,
        }
    }

    /// Clone `source` into this (empty) repo over the local file protocol.
    pub fn clone_from(&self, source: &Repo, extra: &[&str]) {
        let url = format!("file://{}", source.root.display());
        let mut args = vec!["-c", "protocol.file.allow=always", "clone", "-q"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&[url.as_str(), "."]);
        self.git(&args);
    }

    fn setup_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(git_path());
        cmd.current_dir(&self.root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "protocol.allow=never",
            ])
            .args(args);
        cmd
    }

    /// Run setup git; panic on failure; return stdout.
    pub fn git(&self, args: &[&str]) -> String {
        let out = self.setup_command(args).output().expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    pub fn git_stdin(&self, args: &[&str], input: &[u8]) -> String {
        let mut child = self
            .setup_command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("git spawns");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(input)
            .expect("write");
        let out = child.wait_with_output().expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    pub fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, bytes).expect("write");
    }

    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Add a path to the index with raw bytes, bypassing the filesystem (for
    /// names a case-insensitive or UTF-8-only volume cannot hold).
    pub fn index_raw(&self, path: &[u8], content: &[u8]) {
        let oid = self.git_stdin(&["hash-object", "-w", "--stdin"], content);
        let mut line = format!("100644 {oid}\t").into_bytes();
        line.extend_from_slice(path);
        line.push(b'\n');
        self.git_stdin(&["update-index", "--add", "--index-info"], &line);
    }

    pub fn grant(&self, owner: CallerId) -> RepoGrant {
        RepoGrant {
            repo_id: RepoId::new(),
            canonical_identity: Text::new(self.root.to_str().expect("utf-8 root")).expect("text"),
            owner,
            allowed_scopes: BoundedVec::new(vec![
                ScopeMode::Branch,
                ScopeMode::Range,
                ScopeMode::Commit,
                ScopeMode::Staged,
                ScopeMode::Worktree,
                ScopeMode::Paths,
                ScopeMode::ImportedPr,
            ])
            .expect("bounded"),
            created_at: Timestamp(0),
        }
    }

    /// Path, mode and content of every file under the root, `.git` included.
    pub fn fingerprint(&self) -> BTreeMap<PathBuf, (u32, Digest)> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("list") {
                let path = entry.expect("entry").path();
                let meta = std::fs::symlink_metadata(&path).expect("stat");
                use std::os::unix::fs::PermissionsExt;
                let mode = meta.permissions().mode();
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.file_type().is_symlink() {
                    let target = std::fs::read_link(&path).expect("readlink");
                    out.insert(
                        path,
                        (mode, Digest::of(target.as_os_str().as_encoded_bytes())),
                    );
                } else if meta.is_file() {
                    out.insert(
                        path.clone(),
                        (mode, Digest::of(&std::fs::read(&path).expect("read"))),
                    );
                } else {
                    out.insert(path, (mode, Digest::of(b"special")));
                }
            }
        }
        out
    }
}

pub struct Harness {
    _store_dir: tempfile::TempDir,
    pub store: Arc<Store>,
    pub owner: CallerId,
}

impl Harness {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open(&dir.path().join("store")).expect("store"));
        Self {
            _store_dir: dir,
            store,
            owner: CallerId::new(),
        }
    }

    pub fn snapshotter(&self, options: CaptureOptions) -> Snapshotter {
        let git = GitBinary::at(&git_path()).expect("git usable");
        Snapshotter::new(git, self.store.clone(), options)
    }

    pub fn default_snapshotter(&self) -> Snapshotter {
        self.snapshotter(CaptureOptions::default())
    }

    /// Every byte the store holds, concatenated (to prove secrets never entered it).
    pub fn all_stored_bytes(&self) -> Vec<u8> {
        let mut all = Vec::new();
        let mut stack = vec![self.store.root().join("blobs")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("list") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    all.extend(std::fs::read(&path).expect("read"));
                }
            }
        }
        all
    }
}

pub fn scope(mode: ScopeMode, base: Option<&str>, head: Option<&str>) -> ReviewScope {
    ReviewScope {
        mode,
        base: base.map(|b| Text::new(b).expect("rev")),
        head: head.map(|h| Text::new(h).expect("rev")),
        paths: BoundedVec::empty(),
        include_untracked: false,
        merge_parent: None,
    }
}

pub fn path(p: &str) -> RepoPath {
    RepoPath::from_bytes(p.as_bytes()).expect("path")
}

pub fn entry<'a>(snap: &'a Snapshot, p: &str) -> &'a razista_core::ManifestEntry {
    let want = path(p);
    snap.manifest
        .entries
        .iter()
        .find(|e| e.path == want)
        .unwrap_or_else(|| panic!("no manifest entry for {p}: {:#?}", snap.manifest.entries))
}

pub fn entries_for(snap: &Snapshot, p: &str) -> usize {
    let want = path(p);
    snap.manifest
        .entries
        .iter()
        .filter(|e| e.path == want)
        .count()
}

pub fn skipped_reason(snap: &Snapshot, p: &str) -> Option<razista_core::SkipReason> {
    let want = path(p);
    snap.manifest
        .skipped
        .iter()
        .find(|s| s.path == want)
        .map(|s| s.reason)
}

pub fn canary_absent(path: &Path) {
    assert!(
        !path.exists(),
        "external helper executed: {} was created",
        path.display()
    );
}

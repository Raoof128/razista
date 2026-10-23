//! Safe path handling: where the git directory really is, and reading
//! working-tree files without following links out of the repository,
//! blocking on special files, or reading oversized inputs.
//!
//! Working-tree reads walk the path with `openat` one component at a time,
//! each opened `O_NOFOLLOW`, so a directory swapped for a symlink mid-walk
//! cannot redirect the read: there is no check-then-open window.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use razista_core::{ErrorCode, Result, ReviewError, SkipReason};
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags};
use rustix::io::Errno;

fn denied(message: impl Into<String>) -> ReviewError {
    ReviewError::new(ErrorCode::PolicyDenied, message)
}

fn io_err(e: impl std::fmt::Display) -> ReviewError {
    ReviewError::invalid(e.to_string())
}

/// The repository root named by a grant: absolute and already canonical.
pub(crate) fn checked_root(identity: &str) -> Result<PathBuf> {
    let root = PathBuf::from(identity);
    if !root.is_absolute() {
        return Err(ReviewError::invalid(format!(
            "repository root {identity:?} must be absolute"
        )));
    }
    let canonical = root.canonicalize().map_err(io_err)?;
    if canonical != root {
        return Err(ReviewError::invalid(format!(
            "repository root {identity:?} is not canonical ({})",
            canonical.display()
        )));
    }
    Ok(root)
}

fn within_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}

/// Open a git metadata file safely: never through a link, never blocking on
/// a FIFO, and only if it is a regular file. `Ok(None)` if absent.
fn open_metadata_file(path: &Path) -> Result<Option<fs::File>> {
    let fd = match rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(e) => {
            return Err(denied(format!(
                "{} cannot be read safely: {e}",
                path.display()
            )));
        }
    };
    let stat = rustix::fs::fstat(&fd).map_err(io_err)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(denied(format!("{} is not a regular file", path.display())));
    }
    Ok(Some(fs::File::from(fd)))
}

/// Read a git metadata file of at most `limit` bytes.
fn read_metadata_file(path: &Path, limit: u64) -> Result<Option<String>> {
    let Some(file) = open_metadata_file(path)? else {
        return Ok(None);
    };
    let mut text = String::new();
    file.take(limit + 1)
        .read_to_string(&mut text)
        .map_err(io_err)?;
    if text.len() as u64 > limit {
        return Err(denied(format!("{} is implausibly large", path.display())));
    }
    Ok(Some(text))
}

/// Git metadata that must be a plain file or directory inside its git dir.
const GIT_INTERNALS: &[&str] = &[
    "HEAD",
    "config",
    "config.worktree",
    "index",
    "packed-refs",
    "shallow",
    "commondir",
    "gitdir",
    "objects",
    "objects/info",
    "objects/pack",
    "refs",
    "info",
    "hooks",
    "logs",
    "worktrees",
];

fn check_internals(dir: &Path) -> Result<()> {
    for name in GIT_INTERNALS {
        let path = dir.join(name);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(denied(format!(
                    "{} is a symbolic link; refusing to follow git metadata out of the repository",
                    path.display()
                )));
            }
            Ok(m) if !(m.is_file() || m.is_dir()) => {
                return Err(denied(format!("{} is a special file", path.display())));
            }
            _ => {}
        }
    }
    // A link anywhere below the object store or refs could point git at
    // another repository's objects; walk them fully.
    for sub in ["objects", "refs", "info"] {
        let top = dir.join(sub);
        if top.is_dir() {
            reject_links_below(&top, 0)?;
        }
    }
    Ok(())
}

/// Refuse any symlink or special file below `dir` (directories are walked
/// without following links).
fn reject_links_below(dir: &Path, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(denied(format!(
            "{} is nested implausibly deep",
            dir.display()
        )));
    }
    for entry in fs::read_dir(dir).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        let kind = entry.file_type().map_err(io_err)?;
        if kind.is_symlink() {
            return Err(denied(format!(
                "{} is a symbolic link inside git metadata",
                entry.path().display()
            )));
        }
        if kind.is_dir() {
            reject_links_below(&entry.path(), depth + 1)?;
        } else if !kind.is_file() {
            return Err(denied(format!(
                "{} is a special file inside git metadata",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Where git keeps this checkout's metadata.
#[derive(Debug, Clone)]
pub(crate) struct GitDirs {
    /// Per-worktree git directory (`GIT_DIR`).
    pub git_dir: PathBuf,
    /// Shared directory with objects and refs (same as `git_dir` unless linked).
    pub common_dir: PathBuf,
}

/// Find the git directory for `root`, refusing indirection the caller has
/// not authorised: a symlinked `.git`, a gitfile or `commondir` pointing
/// outside the repository, symlinked or special metadata, and object alternates.
pub(crate) fn resolve_git_dir(root: &Path, authorised: &[PathBuf]) -> Result<GitDirs> {
    let authorised: Vec<PathBuf> = authorised
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    let dotgit = root.join(".git");
    let meta = fs::symlink_metadata(&dotgit)
        .map_err(|_| ReviewError::invalid(format!("{} is not a git repository", root.display())))?;

    let git_dir = if meta.file_type().is_symlink() {
        return Err(denied(".git is a symbolic link; refusing to follow it"));
    } else if meta.is_dir() {
        dotgit
    } else if meta.is_file() {
        let text = read_metadata_file(&dotgit, 4096)?.unwrap_or_default();
        let target = text
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| denied(".git file does not name a gitdir"))?;
        let target = root.join(target).canonicalize().map_err(io_err)?;
        if !within_any(&target, &authorised) {
            return Err(denied(format!(
                ".git points to {} which is not an authorised git directory",
                target.display()
            )));
        }
        target
    } else {
        return Err(denied(".git is neither a directory nor a gitfile"));
    };
    check_internals(&git_dir)?;

    let mut common_dir = git_dir.clone();
    if let Some(text) = read_metadata_file(&git_dir.join("commondir"), 4096)? {
        let common = git_dir.join(text.trim()).canonicalize().map_err(io_err)?;
        let local = common.starts_with(root.join(".git"));
        if !local && !within_any(&common, &authorised) {
            return Err(denied(format!(
                "git common directory {} is not authorised",
                common.display()
            )));
        }
        check_internals(&common)?;
        common_dir = common;
    }

    if let Some(text) = read_metadata_file(&common_dir.join("objects/info/alternates"), 4096)?
        && text
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
    {
        return Err(denied(
            "object alternates read another object store; refusing (not authorised)",
        ));
    }
    Ok(GitDirs {
        git_dir,
        common_dir,
    })
}

/// Commits listed as shallow boundaries (their parents are not local).
pub(crate) fn shallow_commits(dirs: &GitDirs) -> Result<Vec<String>> {
    Ok(
        read_metadata_file(&dirs.common_dir.join("shallow"), 16 * 1024 * 1024)?
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    )
}

/// What a tracked path holds in the working tree.
#[derive(Debug)]
pub(crate) enum WorktreeFile {
    Regular {
        bytes: Vec<u8>,
        executable: bool,
    },
    /// A link inside the repository: its target, recorded but not followed.
    Symlink {
        target: Vec<u8>,
    },
    Missing,
    TooLarge,
    Unsafe,
}

pub(crate) fn os_path(rel: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(rel))
}

fn open_dir_nofollow(parent: &OwnedFd, name: &OsStr) -> std::result::Result<OwnedFd, Errno> {
    rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// A working-tree path opened without leaving the repository.
enum Opened {
    File {
        file: fs::File,
        size: u64,
        executable: bool,
    },
    Symlink {
        target: Vec<u8>,
    },
    Missing,
    Unsafe,
}

/// Walk `rel` under `root` with `openat`, never following a link.
fn open_worktree(root: &Path, rel: &[u8]) -> Result<Opened> {
    let components: Vec<Component<'_>> = os_path(rel).components().collect();
    let Some((last, parents)) = components.split_last() else {
        return Ok(Opened::Unsafe);
    };
    let mut dir = rustix::fs::openat(
        CWD,
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_err)?;
    for c in parents {
        let Component::Normal(name) = c else {
            return Ok(Opened::Unsafe);
        };
        dir = match open_dir_nofollow(&dir, name) {
            Ok(fd) => fd,
            Err(Errno::NOENT | Errno::NOTDIR | Errno::LOOP) => {
                // A link, a file where a directory was, or nothing at all.
                return Ok(
                    match rustix::fs::statat(&dir, *name, AtFlags::SYMLINK_NOFOLLOW) {
                        Ok(st) if FileType::from_raw_mode(st.st_mode) == FileType::Symlink => {
                            Opened::Unsafe
                        }
                        _ => Opened::Missing,
                    },
                );
            }
            Err(e) => return Err(io_err(e)),
        };
    }
    let Component::Normal(name) = last else {
        return Ok(Opened::Unsafe);
    };
    let stat = match rustix::fs::statat(&dir, *name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(st) => st,
        Err(Errno::NOENT | Errno::NOTDIR) => return Ok(Opened::Missing),
        Err(e) => return Err(io_err(e)),
    };
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::Symlink => {
            let target = rustix::fs::readlinkat(&dir, *name, Vec::new()).map_err(io_err)?;
            return Ok(Opened::Symlink {
                target: target.into_bytes(),
            });
        }
        FileType::Directory => return Ok(Opened::Missing),
        FileType::RegularFile => {}
        _ => return Ok(Opened::Unsafe),
    }
    let fd = match rustix::fs::openat(
        &dir,
        *name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        // Swapped for a link or removed between stat and open.
        Err(_) => return Ok(Opened::Unsafe),
    };
    let opened = rustix::fs::fstat(&fd).map_err(io_err)?;
    if FileType::from_raw_mode(opened.st_mode) != FileType::RegularFile
        || opened.st_ino != stat.st_ino
        || opened.st_dev != stat.st_dev
    {
        return Ok(Opened::Unsafe);
    }
    Ok(Opened::File {
        file: fs::File::from(fd),
        size: u64::try_from(opened.st_size).unwrap_or(u64::MAX),
        executable: u32::from(opened.st_mode) & 0o100 != 0,
    })
}

/// Read `rel` under `root` without leaving the repository.
pub(crate) fn read_worktree(root: &Path, rel: &[u8], max_bytes: u64) -> Result<WorktreeFile> {
    Ok(match open_worktree(root, rel)? {
        Opened::Missing => WorktreeFile::Missing,
        Opened::Unsafe => WorktreeFile::Unsafe,
        Opened::Symlink { target } => {
            if link_stays_inside(rel, &target) {
                WorktreeFile::Symlink { target }
            } else {
                WorktreeFile::Unsafe
            }
        }
        Opened::File { size, .. } if size > max_bytes => WorktreeFile::TooLarge,
        Opened::File {
            file, executable, ..
        } => {
            let mut bytes = Vec::new();
            file.take(max_bytes + 1)
                .read_to_end(&mut bytes)
                .map_err(io_err)?;
            if bytes.len() as u64 > max_bytes {
                WorktreeFile::TooLarge
            } else {
                WorktreeFile::Regular { bytes, executable }
            }
        }
    })
}

/// The git object id of a working-tree file, computed by streaming it (no
/// size limit, nothing held in memory). `sha256` selects the object format.
/// `None` if the path is not a regular file inside the repository.
pub(crate) fn worktree_blob_oid(root: &Path, rel: &[u8], sha256: bool) -> Result<Option<String>> {
    use sha1::Digest as _;
    let Opened::File { mut file, size, .. } = open_worktree(root, rel)? else {
        return Ok(None);
    };
    let header = format!("blob {size}\0");
    let mut buf = vec![0u8; 64 * 1024];
    macro_rules! stream {
        ($hasher:expr) => {{
            let mut h = $hasher;
            h.update(header.as_bytes());
            loop {
                let n = file.read(&mut buf).map_err(io_err)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(Some(hex_lower(&h.finalize())))
        }};
    }
    if sha256 {
        stream!(sha2::Sha256::new())
    } else {
        stream!(sha1::Sha1::new())
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A symlink at `rel` whose `target` resolves (lexically) inside the repository.
pub(crate) fn link_stays_inside(rel: &[u8], target: &[u8]) -> bool {
    let target = os_path(target);
    if target.is_absolute() {
        return false;
    }
    let mut depth: i64 = os_path(rel).components().count() as i64 - 1;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

const VENDORED_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "third_party",
    "bower_components",
    ".venv",
    "venv",
    "site-packages",
];

/// A path-based reason to skip, before looking at content.
pub(crate) fn skip_by_path(rel: &[u8]) -> Option<SkipReason> {
    let path = os_path(rel);
    let vendored = path.components().any(|c| match c {
        Component::Normal(n) => VENDORED_DIRS.iter().any(|v| n.as_bytes() == v.as_bytes()),
        _ => false,
    });
    if vendored {
        return Some(SkipReason::Vendored);
    }
    let name = path.file_name().map(OsStr::as_bytes).unwrap_or_default();
    if name.ends_with(b".min.js") || name.ends_with(b".min.css") {
        return Some(SkipReason::Generated);
    }
    None
}

/// A content-based reason to skip.
pub(crate) fn skip_by_content(bytes: &[u8]) -> Option<SkipReason> {
    if bytes.starts_with(b"version https://git-lfs.github.com/spec/") {
        return Some(SkipReason::LfsPointer);
    }
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Some(SkipReason::Binary);
    }
    None
}

/// Lines as git and every span count them: terminated by `\n` only.
pub(crate) fn line_count(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let trailing = usize::from(bytes.last().is_some_and(|&b| b != b'\n'));
    u32::try_from(newlines + trailing).unwrap_or(u32::MAX)
}

/// CRLF -> LF, the clean form of git's built-in end-of-line conversion.
pub(crate) fn crlf_to_lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

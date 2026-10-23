//! Immutable snapshot capture (design §7).
//!
//! The scope is resolved to exact commits before anything is read; nothing is
//! ever fetched and no default branch is guessed. Each attempt captures the
//! repository twice and compares the two manifests; if they differ the
//! attempt is repeated once, then `SNAPSHOT_UNSTABLE`. Every tracked path, base
//! path and untracked path is either an entry in the manifest or a skipped
//! input with a reason, and any skipped input that changed raises a gap.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ultra_core::{
    ArtifactId, BoundedVec, ChangeKind, ConsistencyEvidence, CoverageGap, Digest, ErrorCode,
    FileMode, GapKind, ManifestEntry, ObjectId, RepoGrant, RepoPath, ResolvedScope, Result,
    ReviewError, ReviewScope, ScopeMode, SkipReason, SkippedInput, Snapshot, SnapshotId,
    SnapshotManifest, Summary, Text, TrustLabel,
};
use ultra_store::Store;

use crate::git::{
    Attrs, Git, GitBinary, GitEntry, MODE_GITLINK, MODE_SYMLINK, check_revision, mode_is_executable,
};
use crate::paths::{
    WorktreeFile, checked_root, crlf_to_lf, line_count, link_stays_inside, read_worktree,
    resolve_git_dir, shallow_commits, skip_by_content, skip_by_path, worktree_blob_oid,
};

/// Capture limits and authorisations. Defaults follow design §7.
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    /// Git directories outside the repository root that a linked worktree
    /// may use (its common directory). Nothing else outside is followed.
    pub authorised_git_dirs: Vec<PathBuf>,
    /// Per-file limit for source text (default 2 MiB).
    pub max_file_bytes: u64,
    /// Maximum included files per snapshot (default 20,000). Changed files
    /// are included first; the rest are skipped with `file_limit`.
    pub max_files: usize,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            authorised_git_dirs: Vec::new(),
            max_file_bytes: 2 * 1024 * 1024,
            max_files: 20_000,
        }
    }
}

type PassHook = Box<dyn Fn(u8) + Send + Sync>;

pub struct Snapshotter {
    git: GitBinary,
    pub(crate) store: Arc<Store>,
    options: CaptureOptions,
    between_passes: Option<PassHook>,
}

impl std::fmt::Debug for Snapshotter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Snapshotter")
            .field("git", &self.git)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// Which state is being reviewed.
enum Reviewed {
    Tree(ObjectId),
    Index,
    Worktree,
}

struct Resolved {
    base: Option<ObjectId>,
    head: Option<ObjectId>,
    reviewed: Reviewed,
}

/// One capture pass.
struct Pass {
    resolved_scope: ResolvedScope,
    manifest: SnapshotManifest,
    gaps: Vec<CoverageGap>,
    digest: Digest,
}

#[derive(Clone)]
struct Content {
    bytes: Vec<u8>,
    mode: FileMode,
}

/// What is known about a skipped path's reviewed state, to decide whether
/// it changed relative to the base.
#[derive(Clone)]
enum Seen {
    Absent,
    Oid(String),
    Bytes(Digest),
    Unknown { index_oid: Option<String> },
}

const MAX_LISTED_GAPS: usize = 900;

fn file_mode(git_mode: u32) -> FileMode {
    if git_mode == MODE_SYMLINK {
        FileMode::Symlink
    } else if mode_is_executable(git_mode) {
        FileMode::Executable
    } else {
        FileMode::Regular
    }
}

fn gap(kind: GapKind, subject: &str, code: Option<ErrorCode>, detail: &str) -> Result<CoverageGap> {
    let subject: String = subject.chars().take(512).collect();
    let detail: String = detail.chars().take(1024).collect();
    Ok(CoverageGap {
        kind,
        subject: Text::new(if subject.is_empty() {
            "-".into()
        } else {
            subject
        })?,
        code,
        detail: Summary::new(detail)?,
    })
}

fn display(rel: &[u8]) -> String {
    RepoPath::from_bytes(rel).map_or_else(
        |_| String::from_utf8_lossy(rel).escape_debug().to_string(),
        |p| p.display(),
    )
}

fn skip_reason_name(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::Binary => "binary",
        SkipReason::TooLarge => "too large",
        SkipReason::LfsPointer => "LFS pointer",
        SkipReason::Submodule => "submodule",
        SkipReason::Generated => "generated",
        SkipReason::Vendored => "vendored",
        SkipReason::UnsafePath => "unsafe path",
        SkipReason::UntrackedNotApproved => "untracked, not approved",
        SkipReason::FileLimit => "file limit",
    }
}

impl Snapshotter {
    pub fn new(git: GitBinary, store: Arc<Store>, options: CaptureOptions) -> Self {
        Self {
            git,
            store,
            options,
            between_passes: None,
        }
    }

    /// Run `hook(attempt)` between the two passes of each attempt. Used to
    /// simulate concurrent edits deterministically.
    #[doc(hidden)]
    pub fn with_between_passes_hook(mut self, hook: impl Fn(u8) + Send + Sync + 'static) -> Self {
        self.between_passes = Some(Box::new(hook));
        self
    }

    /// Freeze the scope of `grant`'s repository. Repository bytes are stored
    /// as untrusted artefacts owned by the grant's owner.
    pub fn capture(&self, grant: &RepoGrant, scope: &ReviewScope) -> Result<Snapshot> {
        for rev in [&scope.base, &scope.head].into_iter().flatten() {
            check_revision(rev.as_str())?;
        }
        let root = checked_root(grant.canonical_identity.as_str())?;
        let dirs = resolve_git_dir(&root, &self.options.authorised_git_dirs)?;
        let shallow: HashSet<String> = shallow_commits(&dirs)?.into_iter().collect();
        let git = Git::new(&self.git, dirs.git_dir, root.clone())?;

        for attempt in 1..=2u8 {
            let first = self.pass(&git, &root, scope, grant, &shallow, false)?;
            if let Some(hook) = &self.between_passes {
                hook(attempt);
            }
            let second = self.pass(&git, &root, scope, grant, &shallow, true)?;
            if first.digest == second.digest {
                return Ok(Snapshot {
                    snapshot_id: SnapshotId::new(),
                    repo_id: grant.repo_id,
                    resolved_scope: second.resolved_scope,
                    manifest: second.manifest,
                    consistency_evidence: ConsistencyEvidence {
                        capture_passes: attempt * 2,
                        manifest_digest: second.digest,
                    },
                    gaps: BoundedVec::new(second.gaps)?,
                });
            }
        }
        Err(ReviewError::new(
            ErrorCode::SnapshotUnstable,
            "the repository changed during capture, twice; retry when it is idle",
        ))
    }

    fn resolve(
        &self,
        git: &Git<'_>,
        scope: &ReviewScope,
        shallow: &HashSet<String>,
    ) -> Result<Resolved> {
        let rev = |r: &str| -> Result<ObjectId> {
            check_revision(r)?;
            git.resolve_commit(r)
        };
        let named = |t: &Option<Text<1, 256>>| t.as_ref().map(|t| t.as_str().to_owned());
        match scope.mode {
            ScopeMode::Worktree | ScopeMode::Paths => Ok(Resolved {
                base: Some(rev(&named(&scope.base).unwrap_or_else(|| "HEAD".into()))?),
                head: git.resolve_commit("HEAD").ok(),
                reviewed: Reviewed::Worktree,
            }),
            ScopeMode::Staged => Ok(Resolved {
                base: Some(rev(&named(&scope.base).unwrap_or_else(|| "HEAD".into()))?),
                head: None,
                reviewed: Reviewed::Index,
            }),
            ScopeMode::Commit => {
                let head = rev(&named(&scope.head).unwrap_or_else(|| "HEAD".into()))?;
                if shallow.contains(head.as_str()) {
                    return Err(ReviewError::new(
                        ErrorCode::BaseUnavailable,
                        "the commit's parents are beyond this shallow clone's boundary; nothing was fetched",
                    ));
                }
                let parents = git.parents(&head)?;
                let base = match (parents.len(), scope.merge_parent) {
                    (0, None) => None,
                    (1, None | Some(1)) => Some(parents[0].clone()),
                    (n, Some(p)) if p >= 1 && usize::from(p) <= n => {
                        Some(parents[usize::from(p) - 1].clone())
                    }
                    (n, None) => {
                        return Err(ReviewError::invalid(format!(
                            "commit has {n} parents; choose merge_parent explicitly"
                        )));
                    }
                    (n, Some(p)) => {
                        return Err(ReviewError::invalid(format!(
                            "merge_parent {p} is out of range for a commit with {n} parents"
                        )));
                    }
                };
                Ok(Resolved {
                    base,
                    head: Some(head.clone()),
                    reviewed: Reviewed::Tree(head),
                })
            }
            ScopeMode::Branch => {
                let base_rev = named(&scope.base).ok_or_else(|| {
                    ReviewError::new(
                        ErrorCode::BaseUnavailable,
                        "branch scope needs an explicit local base; no default branch is guessed",
                    )
                })?;
                let head = rev(&named(&scope.head).unwrap_or_else(|| "HEAD".into()))?;
                let base = git.merge_base(&rev(&base_rev)?, &head)?;
                Ok(Resolved {
                    base: Some(base),
                    head: Some(head.clone()),
                    reviewed: Reviewed::Tree(head),
                })
            }
            ScopeMode::Range => {
                let base_rev = named(&scope.base).ok_or_else(|| {
                    ReviewError::new(ErrorCode::BaseUnavailable, "range scope needs a base")
                })?;
                let head_rev = named(&scope.head)
                    .ok_or_else(|| ReviewError::invalid("range scope needs a head"))?;
                let head = rev(&head_rev)?;
                Ok(Resolved {
                    base: Some(rev(&base_rev)?),
                    head: Some(head.clone()),
                    reviewed: Reviewed::Tree(head),
                })
            }
            ScopeMode::ImportedPr => Err(ReviewError::invalid(
                "imported_pr scope needs a locally prepared PR snapshot, which is not supported yet",
            )),
        }
    }

    fn selected(scope: &ReviewScope, path: &[u8]) -> bool {
        scope.paths.is_empty()
            || scope.paths.iter().any(|p| {
                let p = p.as_bytes();
                path == p || (path.starts_with(p) && path.get(p.len()) == Some(&b'/'))
            })
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    fn pass(
        &self,
        git: &Git<'_>,
        root: &Path,
        scope: &ReviewScope,
        grant: &RepoGrant,
        shallow: &HashSet<String>,
        store: bool,
    ) -> Result<Pass> {
        let max = self.options.max_file_bytes;
        let resolved = self.resolve(git, scope, shallow)?;
        let base_tree: HashMap<Vec<u8>, GitEntry> = match &resolved.base {
            Some(c) => git.ls_tree(c)?,
            None => HashMap::new(),
        };
        let (tracked, from_disk) = match &resolved.reviewed {
            Reviewed::Tree(c) => (git.ls_tree(c)?, false),
            Reviewed::Index => (git.ls_index()?, false),
            Reviewed::Worktree => (git.ls_index()?, true),
        };

        // Every blob either side names must be local: a missing object would
        // otherwise turn a change into an addition. Nothing is fetched.
        let mut wanted: Vec<(&str, &Vec<u8>)> = base_tree
            .iter()
            .chain(tracked.iter())
            .filter(|(_, e)| e.mode != MODE_GITLINK)
            .map(|(p, e)| (e.oid.as_str(), p))
            .collect();
        wanted.sort_unstable();
        wanted.dedup_by(|a, b| a.0 == b.0);
        let oids: Vec<&str> = wanted.iter().map(|(o, _)| *o).collect();
        let sizes = git.blob_sizes(&oids)?;
        if let Some((oid, rel)) = wanted.iter().find(|(o, _)| !sizes.contains_key(*o)) {
            return Err(ReviewError::new(
                ErrorCode::BaseUnavailable,
                format!(
                    "object {oid} for {} is not available locally (partial clone?); nothing was fetched",
                    display(rel)
                ),
            ));
        }
        let small: Vec<&str> = oids
            .iter()
            .copied()
            .filter(|o| sizes.get(*o).is_some_and(|s| *s <= max))
            .collect();
        let blobs = git.blobs(&small)?;

        let attrs: HashMap<Vec<u8>, Attrs> = if from_disk {
            let paths: Vec<&[u8]> = tracked
                .iter()
                .filter(|(_, e)| e.mode != MODE_GITLINK && !e.skip_worktree)
                .map(|(p, _)| p.as_slice())
                .collect();
            git.check_attrs(&paths)?
        } else {
            HashMap::new()
        };
        let autocrlf = if from_disk {
            git.config_value("core.autocrlf")?
        } else {
            None
        };

        let mut notes: Vec<CoverageGap> = Vec::new();
        let mut skips: BTreeMap<Vec<u8>, (SkipReason, Seen)> = BTreeMap::new();
        let mut candidates: BTreeMap<Vec<u8>, Content> = BTreeMap::new();

        let read_disk = |rel: &[u8]| read_worktree(root, rel, max);
        // Oversized files are identified by their git object id, streamed
        // from disk, so an edit that keeps the size cannot hide.
        let sha256 = oids.first().is_some_and(|o| o.len() == 64);
        let large_seen = |rel: &[u8]| -> Result<Seen> {
            Ok(match worktree_blob_oid(root, rel, sha256)? {
                Some(oid) => Seen::Oid(oid),
                None => Seen::Unknown { index_oid: None },
            })
        };

        let mut tracked_paths: Vec<&Vec<u8>> = tracked.keys().collect();
        tracked_paths.sort();
        for rel in tracked_paths {
            let e = &tracked[rel];
            let index_seen = Seen::Oid(e.oid.clone());
            let disk = from_disk && !e.skip_worktree;
            if e.mode == MODE_GITLINK {
                skips.insert(rel.clone(), (SkipReason::Submodule, index_seen));
                continue;
            }
            if let Some(reason) = skip_by_path(rel) {
                let seen = if disk {
                    match read_disk(rel)? {
                        WorktreeFile::Regular { bytes, .. } => Seen::Bytes(Digest::of(&bytes)),
                        WorktreeFile::Symlink { target } => Seen::Bytes(Digest::of(&target)),
                        WorktreeFile::Missing => Seen::Absent,
                        WorktreeFile::TooLarge => large_seen(rel)?,
                        WorktreeFile::Unsafe => Seen::Unknown {
                            index_oid: Some(e.oid.clone()),
                        },
                    }
                } else {
                    index_seen
                };
                skips.insert(rel.clone(), (reason, seen));
                continue;
            }

            let content = if disk {
                match read_disk(rel)? {
                    WorktreeFile::Regular { bytes, executable } => {
                        let a = attrs.get(rel).cloned().unwrap_or_default();
                        let index_bytes = blobs.get(&e.oid);
                        let normalised = if a.eol_normalised(autocrlf.as_deref())
                            && skip_by_content(&bytes).is_none()
                        {
                            crlf_to_lf(&bytes)
                        } else {
                            bytes.clone()
                        };
                        let effective = match index_bytes {
                            Some(ib) if *ib == normalised || *ib == bytes => ib.clone(),
                            _ => {
                                if let Some(kind) = a.opaque_conversion() {
                                    notes.push(gap(
                                        GapKind::InputSkipped,
                                        &display(rel),
                                        None,
                                        &format!(
                                            "a {kind} attribute applies; the worktree bytes were reviewed without that conversion"
                                        ),
                                    )?);
                                }
                                normalised
                            }
                        };
                        Some(Content {
                            bytes: effective,
                            mode: if executable {
                                FileMode::Executable
                            } else {
                                FileMode::Regular
                            },
                        })
                    }
                    WorktreeFile::Symlink { target } => Some(Content {
                        bytes: target,
                        mode: FileMode::Symlink,
                    }),
                    WorktreeFile::Missing => None,
                    WorktreeFile::TooLarge => {
                        skips.insert(rel.clone(), (SkipReason::TooLarge, large_seen(rel)?));
                        continue;
                    }
                    WorktreeFile::Unsafe => {
                        skips.insert(
                            rel.clone(),
                            (
                                SkipReason::UnsafePath,
                                Seen::Unknown {
                                    index_oid: Some(e.oid.clone()),
                                },
                            ),
                        );
                        continue;
                    }
                }
            } else {
                match blobs.get(&e.oid) {
                    None => {
                        skips.insert(rel.clone(), (SkipReason::TooLarge, index_seen));
                        continue;
                    }
                    Some(bytes) if e.mode == MODE_SYMLINK && !link_stays_inside(rel, bytes) => {
                        skips.insert(rel.clone(), (SkipReason::UnsafePath, index_seen));
                        continue;
                    }
                    Some(bytes) => Some(Content {
                        bytes: bytes.clone(),
                        mode: file_mode(e.mode),
                    }),
                }
            };
            if let Some(c) = content {
                if c.mode != FileMode::Symlink
                    && let Some(reason) = skip_by_content(&c.bytes)
                {
                    skips.insert(rel.clone(), (reason, Seen::Bytes(Digest::of(&c.bytes))));
                } else {
                    candidates.insert(rel.clone(), c);
                }
            }
        }

        let mut untracked_unapproved = 0usize;
        if from_disk {
            for rel in git.ls_untracked()? {
                if !(scope.include_untracked && Self::selected(scope, &rel)) {
                    untracked_unapproved += 1;
                    skips.insert(rel, (SkipReason::UntrackedNotApproved, Seen::Absent));
                    continue;
                }
                let content = match read_disk(&rel)? {
                    WorktreeFile::Regular { bytes, executable } => Content {
                        bytes,
                        mode: if executable {
                            FileMode::Executable
                        } else {
                            FileMode::Regular
                        },
                    },
                    WorktreeFile::Symlink { target } => Content {
                        bytes: target,
                        mode: FileMode::Symlink,
                    },
                    WorktreeFile::Missing => continue,
                    WorktreeFile::TooLarge => {
                        let seen = Seen::Unknown { index_oid: None };
                        skips.insert(rel, (SkipReason::TooLarge, seen));
                        continue;
                    }
                    WorktreeFile::Unsafe => {
                        let seen = Seen::Unknown { index_oid: None };
                        skips.insert(rel, (SkipReason::UnsafePath, seen));
                        continue;
                    }
                };
                let reason = skip_by_path(&rel).or_else(|| {
                    (content.mode != FileMode::Symlink)
                        .then(|| skip_by_content(&content.bytes))
                        .flatten()
                });
                match reason {
                    Some(r) => {
                        skips.insert(rel, (r, Seen::Bytes(Digest::of(&content.bytes))));
                    }
                    None => {
                        candidates.insert(rel, content);
                    }
                }
            }
        }

        // Classify candidates against the base.
        let base_bytes = |rel: &[u8]| -> Option<(&Vec<u8>, FileMode)> {
            base_tree
                .get(rel)
                .filter(|e| e.mode != MODE_GITLINK)
                .and_then(|e| blobs.get(&e.oid).map(|b| (b, file_mode(e.mode))))
        };
        let mut classified: Vec<(Vec<u8>, Content, ChangeKind, Option<Digest>)> = Vec::new();
        for (rel, c) in candidates {
            let digest = Digest::of(&c.bytes);
            let (change, base_digest) = match (base_tree.get(&rel), base_bytes(&rel)) {
                (None, _) => (ChangeKind::Added, None),
                (Some(_), Some((bytes, mode))) => {
                    let bd = Digest::of(bytes);
                    if bd == digest && mode == c.mode {
                        (ChangeKind::Unchanged, None)
                    } else {
                        (ChangeKind::Modified, Some(bd))
                    }
                }
                (Some(_), None) => {
                    notes.push(gap(
                        GapKind::InputSkipped,
                        &display(&rel),
                        None,
                        "the base version is a submodule or too large to compare; reviewed as new content",
                    )?);
                    (ChangeKind::Modified, None)
                }
            };
            classified.push((rel, c, change, base_digest));
        }

        // Include changed files first, then unchanged ones, up to the limit.
        classified.sort_by(|a, b| {
            (a.2 == ChangeKind::Unchanged)
                .cmp(&(b.2 == ChangeKind::Unchanged))
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut entries: Vec<ManifestEntry> = Vec::new();
        let mut present: HashSet<Vec<u8>> = HashSet::new();
        let mut to_store: Vec<Vec<u8>> = Vec::new();
        for (rel, c, change, base_digest) in classified {
            present.insert(rel.clone());
            if entries.len() >= self.options.max_files {
                skips.insert(
                    rel,
                    (SkipReason::FileLimit, Seen::Bytes(Digest::of(&c.bytes))),
                );
                continue;
            }
            if base_digest.is_some()
                && let Some((bytes, _)) = base_bytes(&rel)
            {
                to_store.push(bytes.clone());
            }
            entries.push(ManifestEntry {
                path: RepoPath::from_bytes(&rel)?,
                digest: Digest::of(&c.bytes),
                base_digest,
                mode: c.mode,
                size_bytes: c.bytes.len() as u64,
                line_count: line_count(&c.bytes),
                change,
                renamed_from: None,
            });
            to_store.push(c.bytes);
        }

        // Deletions: base paths absent from the reviewed state.
        let mut deleted: Vec<ManifestEntry> = Vec::new();
        let mut base_paths: Vec<&Vec<u8>> = base_tree.keys().collect();
        base_paths.sort();
        for rel in base_paths {
            if present.contains(rel) || skips.contains_key(rel) {
                continue;
            }
            let e = &base_tree[rel];
            let excluded = if e.mode == MODE_GITLINK {
                Some(SkipReason::Submodule)
            } else {
                skip_by_path(rel)
            };
            if let Some(reason) = excluded {
                skips.insert(rel.clone(), (reason, Seen::Absent));
                continue;
            }
            match blobs.get(&e.oid) {
                Some(bytes) if e.mode == MODE_SYMLINK || skip_by_content(bytes).is_none() => {
                    to_store.push(bytes.clone());
                    deleted.push(ManifestEntry {
                        path: RepoPath::from_bytes(rel)?,
                        digest: Digest::of(bytes),
                        base_digest: None,
                        mode: file_mode(e.mode),
                        size_bytes: bytes.len() as u64,
                        line_count: line_count(bytes),
                        change: ChangeKind::Deleted,
                        renamed_from: None,
                    });
                }
                Some(bytes) => {
                    let reason = skip_by_content(bytes).unwrap_or(SkipReason::Binary);
                    skips.insert(rel.clone(), (reason, Seen::Absent));
                }
                None => {
                    skips.insert(rel.clone(), (SkipReason::TooLarge, Seen::Absent));
                }
            }
        }

        // Exact-content renames (git ignores empty files for renames; so do we).
        let empty = Digest::of(b"");
        for entry in entries.iter_mut().filter(|e| e.change == ChangeKind::Added) {
            if entry.digest == empty {
                continue;
            }
            if let Some(pos) = deleted
                .iter()
                .position(|d| d.digest == entry.digest && d.mode == entry.mode)
            {
                let source = deleted.remove(pos);
                entry.change = ChangeKind::Renamed;
                entry.base_digest = Some(source.digest);
                entry.renamed_from = Some(source.path);
            }
        }
        entries.extend(deleted);
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        // Gaps: changed inputs that were skipped are never silent.
        let mut changed_skips: Vec<(String, SkipReason)> = Vec::new();
        for (rel, (reason, seen)) in &skips {
            if *reason == SkipReason::UntrackedNotApproved {
                continue;
            }
            let base = base_tree.get(rel);
            let changed = match seen {
                Seen::Absent => base.is_some(),
                Seen::Oid(o) => base.map(|b| &b.oid) != Some(o),
                Seen::Bytes(d) => match (base, base.and_then(|b| blobs.get(&b.oid))) {
                    (None, _) => true,
                    (Some(_), Some(bytes)) => Digest::of(bytes) != *d,
                    (Some(_), None) => true,
                },
                Seen::Unknown { index_oid } => base.map(|b| &b.oid) != index_oid.as_ref(),
            };
            if changed {
                changed_skips.push((display(rel), *reason));
            }
        }
        let mut gaps: Vec<CoverageGap> = Vec::new();
        let listed = changed_skips.len().min(MAX_LISTED_GAPS);
        for (p, reason) in &changed_skips[..listed] {
            let code = (*reason == SkipReason::FileLimit).then_some(ErrorCode::BudgetExhausted);
            gaps.push(gap(
                GapKind::InputSkipped,
                p,
                code,
                &format!(
                    "this input changed but was not reviewed ({})",
                    skip_reason_name(*reason)
                ),
            )?);
        }
        if changed_skips.len() > listed {
            gaps.push(gap(
                GapKind::InputSkipped,
                "changed inputs",
                None,
                &format!(
                    "{} changed inputs were skipped and not reviewed; the first {listed} are listed",
                    changed_skips.len()
                ),
            )?);
        }
        if skips.values().any(|(r, _)| *r == SkipReason::FileLimit) {
            gaps.push(gap(
                GapKind::InputSkipped,
                "file limit",
                Some(ErrorCode::BudgetExhausted),
                &format!(
                    "more than {} files; changed files were kept first and the rest skipped",
                    self.options.max_files
                ),
            )?);
        }
        if untracked_unapproved > 0 {
            gaps.push(gap(
                GapKind::InputSkipped,
                "untracked files",
                None,
                &format!(
                    "{untracked_unapproved} untracked files were not approved for review and were skipped"
                ),
            )?);
        }

        let mut by_case: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in entries.iter().filter(|e| e.change != ChangeKind::Deleted) {
            by_case
                .entry(e.path.display().to_lowercase())
                .or_default()
                .push(e.path.display());
        }
        for group in by_case.values().filter(|g| g.len() > 1) {
            gaps.push(gap(
                GapKind::CaseCollision,
                &group.join(", "),
                None,
                "paths differ only by case; they collide on case-insensitive volumes",
            )?);
        }
        if !scope.paths.is_empty() {
            let outside = entries
                .iter()
                .filter(|e| {
                    e.change != ChangeKind::Unchanged && !Self::selected(scope, e.path.as_bytes())
                })
                .count();
            if outside > 0 {
                gaps.push(gap(
                    GapKind::InputSkipped,
                    "path scope",
                    None,
                    &format!(
                        "{outside} changed files lie outside the requested paths and are not reviewed"
                    ),
                )?);
            }
        }
        gaps.extend(notes);
        if gaps.len() > 1000 {
            let dropped = gaps.len() - 999;
            gaps.truncate(999);
            gaps.push(gap(
                GapKind::InputSkipped,
                "gaps",
                None,
                &format!("{dropped} further coverage notes were omitted"),
            )?);
        }

        let mut skipped_list: Vec<SkippedInput> = Vec::with_capacity(skips.len());
        let mut unrepresentable = 0usize;
        for (rel, (reason, _)) in skips {
            match RepoPath::from_bytes(&rel) {
                Ok(path) => skipped_list.push(SkippedInput { path, reason }),
                Err(_) => unrepresentable += 1,
            }
        }
        if unrepresentable > 0 && gaps.len() < 1000 {
            gaps.push(gap(
                GapKind::InputSkipped,
                "unrepresentable paths",
                None,
                &format!(
                    "{unrepresentable} paths could not be represented safely and were skipped"
                ),
            )?);
        }

        let resolved_scope = ResolvedScope {
            mode: scope.mode,
            base_commit: resolved.base,
            head_commit: resolved.head,
            merge_parent: scope.merge_parent,
            paths: scope.paths.clone(),
            include_untracked: scope.include_untracked,
        };
        let manifest = SnapshotManifest {
            entries: BoundedVec::new(entries)?,
            skipped: BoundedVec::new(skipped_list)?,
        };
        let digest = Digest::of(
            &serde_json::to_vec(&(&resolved_scope, &manifest, &gaps))
                .map_err(|e| ReviewError::invalid(e.to_string()))?,
        );

        if store {
            let mut seen = HashSet::new();
            for bytes in to_store {
                if seen.insert(Digest::of(&bytes)) {
                    let _: ArtifactId =
                        self.store
                            .put_artifact(&grant.owner, &bytes, TrustLabel::Untrusted)?;
                }
            }
        }
        Ok(Pass {
            resolved_scope,
            manifest,
            gaps,
            digest,
        })
    }
}

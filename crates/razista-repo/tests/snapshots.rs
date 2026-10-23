//! T06: immutable snapshots with exact scope semantics.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::*;
use razista_core::{
    ChangeKind, ContextProvenance, Digest, ErrorCode, FileMode, ScopeMode, SkipReason, TrustLabel,
};
use razista_repo::CaptureOptions;

const APP_V1: &[u8] = b"def add(a, b):\n    return a - b\n";
const APP_V2: &[u8] = b"def add(a, b):\n    return a + b\n";

fn base_repo() -> Repo {
    let repo = Repo::new();
    repo.write("src/app.py", APP_V1);
    repo.write("src/util.py", b"X = 1\n");
    repo.write("README.md", b"# demo\n");
    repo.commit_all("base");
    repo
}

#[test]
fn staged_and_unstaged_are_not_double_counted() {
    let repo = base_repo();
    let h = Harness::new();
    // Stage one edit, then edit the same file again without staging.
    repo.write("src/app.py", b"def add(a, b):\n    return 0\n");
    repo.git(&["add", "src/app.py"]);
    repo.write("src/app.py", APP_V2);
    // A new file, staged.
    repo.write("src/new.py", b"Y = 2\n");
    repo.git(&["add", "src/new.py"]);

    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(entries_for(&snap, "src/app.py"), 1);
    let app = entry(&snap, "src/app.py");
    assert_eq!(app.digest, Digest::of(APP_V2), "effective worktree content");
    assert_eq!(app.base_digest, Some(Digest::of(APP_V1)));
    assert_eq!(app.change, ChangeKind::Modified);
    assert_eq!(entries_for(&snap, "src/new.py"), 1);
    assert_eq!(entry(&snap, "src/new.py").change, ChangeKind::Added);
    assert_eq!(entry(&snap, "src/util.py").change, ChangeKind::Unchanged);
}

#[test]
fn staged_mode_reviews_the_index_not_the_worktree() {
    let repo = base_repo();
    let h = Harness::new();
    repo.write("src/app.py", APP_V2);
    repo.git(&["add", "src/app.py"]);
    repo.write("src/app.py", b"unstaged scribble\n");
    let snap = h
        .default_snapshotter()
        .capture(&repo.grant(h.owner), &scope(ScopeMode::Staged, None, None))
        .expect("capture");
    assert_eq!(entry(&snap, "src/app.py").digest, Digest::of(APP_V2));
}

#[test]
fn range_mode_compares_exactly_two_endpoints() {
    let repo = base_repo();
    let h = Harness::new();
    let c1 = repo.git(&["rev-parse", "HEAD"]);
    repo.write("src/app.py", APP_V2);
    repo.commit_all("fix");
    repo.write("src/util.py", b"X = 2\n");
    let c3 = repo.commit_all("tweak");
    repo.write("src/app.py", b"uncommitted\n"); // must be ignored

    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Range, Some(&c1), Some(&c3)),
        )
        .expect("capture");
    assert_eq!(
        snap.resolved_scope.base_commit.as_ref().map(|o| o.as_str()),
        Some(c1.as_str())
    );
    assert_eq!(
        snap.resolved_scope.head_commit.as_ref().map(|o| o.as_str()),
        Some(c3.as_str())
    );
    assert_eq!(entry(&snap, "src/app.py").digest, Digest::of(APP_V2));
    assert_eq!(entry(&snap, "src/util.py").change, ChangeKind::Modified);
}

#[test]
fn branch_mode_never_guesses_a_base() {
    let repo = base_repo();
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(&repo.grant(h.owner), &scope(ScopeMode::Branch, None, None))
        .expect_err("no implicit main");
    assert_eq!(err.code(), ErrorCode::BaseUnavailable);
}

#[test]
fn missing_base_never_fetches() {
    let repo = base_repo();
    let h = Harness::new();
    let canary = repo.outside.join("fetched");
    // A remote whose transport would run a command if any fetch were attempted.
    repo.git(&[
        "remote",
        "add",
        "origin",
        &format!("ext::sh -c touch% {}", canary.display()),
    ]);
    repo.git(&["config", "remote.origin.promisor", "true"]);
    repo.git(&["config", "extensions.partialClone", "origin"]);
    let before = repo.fingerprint();

    let err = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Branch, Some("origin/main"), None),
        )
        .expect_err("base is not local");
    assert_eq!(err.code(), ErrorCode::BaseUnavailable);
    canary_absent(&canary);
    assert_eq!(repo.fingerprint(), before, "no refs or objects were added");
}

#[test]
fn merge_commit_requires_parent() {
    let repo = base_repo();
    let h = Harness::new();
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("src/feature.py", b"F = 1\n");
    repo.commit_all("feature");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("src/main_only.py", b"M = 1\n");
    repo.commit_all("main work");
    repo.git(&["merge", "-q", "--no-ff", "--no-edit", "feature"]);
    let merge = repo.git(&["rev-parse", "HEAD"]);
    let s = h.default_snapshotter();
    let grant = repo.grant(h.owner);

    let mut sc = scope(ScopeMode::Commit, None, Some(&merge));
    let err = s.capture(&grant, &sc).expect_err("parent required");
    assert_eq!(err.code(), ErrorCode::InvalidInput);

    sc.merge_parent = Some(3);
    let err = s.capture(&grant, &sc).expect_err("no third parent");
    assert_eq!(err.code(), ErrorCode::InvalidInput);

    sc.merge_parent = Some(1);
    let snap = s.capture(&grant, &sc).expect("first parent");
    assert_eq!(entry(&snap, "src/feature.py").change, ChangeKind::Added);
    assert_eq!(
        entry(&snap, "src/main_only.py").change,
        ChangeKind::Unchanged
    );

    sc.merge_parent = Some(2);
    let snap = s.capture(&grant, &sc).expect("second parent");
    assert_eq!(entry(&snap, "src/main_only.py").change, ChangeKind::Added);
}

#[test]
fn concurrent_edit_cannot_mix_snapshots() {
    let repo = base_repo();
    let h = Harness::new();
    let calls = Arc::new(AtomicU32::new(0));
    let (calls2, root) = (calls.clone(), repo.root.clone());
    let s = h
        .default_snapshotter()
        .with_between_passes_hook(move |_attempt| {
            let n = calls2.fetch_add(1, Ordering::SeqCst);
            std::fs::write(root.join("src/app.py"), format!("edit {n}\n")).expect("edit");
        });
    let err = s
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect_err("inputs keep changing");
    assert_eq!(err.code(), ErrorCode::SnapshotUnstable);
    assert_eq!(calls.load(Ordering::SeqCst), 2, "one retry, no more");
}

#[test]
fn single_concurrent_edit_is_retried_once() {
    let repo = base_repo();
    let h = Harness::new();
    let root = repo.root.clone();
    let s = h
        .default_snapshotter()
        .with_between_passes_hook(move |attempt| {
            if attempt == 1 {
                std::fs::write(root.join("src/app.py"), APP_V2).expect("edit");
            }
        });
    let snap = s
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("stable on retry");
    assert_eq!(snap.consistency_evidence.capture_passes, 4);
    assert_eq!(entry(&snap, "src/app.py").digest, Digest::of(APP_V2));
}

#[test]
fn rename_and_delete_preserve_identity() {
    let repo = Repo::new();
    repo.write("a.py", b"A = 1\n");
    repo.write("c.py", b"C = 1\n");
    repo.write("d.py", b"D = 1\n");
    repo.commit_all("base");
    let h = Harness::new();
    repo.git(&["mv", "a.py", "b.py"]);
    repo.git(&["rm", "-q", "c.py"]);
    std::fs::remove_file(repo.root.join("d.py")).expect("unstaged delete");

    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    let b = entry(&snap, "b.py");
    assert_eq!(b.change, ChangeKind::Renamed);
    assert_eq!(b.renamed_from, Some(path("a.py")));
    assert_eq!(b.base_digest, Some(Digest::of(b"A = 1\n")));
    assert_eq!(entries_for(&snap, "a.py"), 0, "a rename is one entry");
    for gone in ["c.py", "d.py"] {
        let e = entry(&snap, gone);
        assert_eq!(e.change, ChangeKind::Deleted, "{gone}");
    }
    assert_eq!(entry(&snap, "c.py").digest, Digest::of(b"C = 1\n"));
}

#[test]
fn git_filter_and_hook_never_execute() {
    let repo = base_repo();
    repo.write(".gitattributes", b"* filter=evil diff=evil merge=evil\n");
    repo.commit_all("attributes");
    let h = Harness::new();
    let canary = repo.outside.join("pwned");
    let touch = format!("touch {}", canary.display());
    // Hostile configuration planted after the commits.
    for (key, value) in [
        ("filter.evil.clean", touch.as_str()),
        ("filter.evil.smudge", touch.as_str()),
        ("filter.evil.process", touch.as_str()),
        ("filter.evil.required", "true"),
        ("diff.evil.textconv", touch.as_str()),
        ("diff.evil.command", touch.as_str()),
        ("diff.external", touch.as_str()),
        ("core.fsmonitor", touch.as_str()),
        ("core.pager", touch.as_str()),
        ("core.sshCommand", touch.as_str()),
        ("credential.helper", &format!("!{touch}")),
        ("core.alternateRefsCommand", touch.as_str()),
        ("merge.evil.driver", touch.as_str()),
    ] {
        repo.git(&["config", key, value]);
    }
    for hook in [
        "pre-commit",
        "post-checkout",
        "post-index-change",
        "reference-transaction",
        "fsmonitor-watchman",
        "pre-auto-gc",
    ] {
        let p = repo.root.join(".git/hooks").join(hook);
        std::fs::write(&p, format!("#!/bin/sh\n{touch}\n")).expect("hook");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    repo.write("src/app.py", APP_V2);
    canary_absent(&canary); // planting the configuration fired nothing

    let s = h.default_snapshotter();
    let grant = repo.grant(h.owner);
    for mode in [ScopeMode::Worktree, ScopeMode::Staged] {
        let snap = s
            .capture(&grant, &scope(mode, None, None))
            .expect("capture");
        s.diff(&h.owner, &snap).expect("diff");
    }
    canary_absent(&canary);
}

#[test]
fn review_does_not_change_checkout() {
    let repo = base_repo();
    let h = Harness::new();
    repo.write("src/app.py", APP_V2);
    repo.git(&["add", "src/app.py"]);
    repo.write("src/util.py", b"X = 3\n");
    repo.write("notes.txt", b"untracked\n");
    let before = repo.fingerprint();
    let s = h.default_snapshotter();
    let mut sc = scope(ScopeMode::Worktree, None, None);
    sc.include_untracked = true;
    let snap = s.capture(&repo.grant(h.owner), &sc).expect("capture");
    s.diff(&h.owner, &snap).expect("diff");
    assert_eq!(
        repo.fingerprint(),
        before,
        "R04: checkout byte-for-byte unchanged"
    );
}

#[test]
fn snapshot_digest_tracks_content_and_mode() {
    let repo = base_repo();
    let h = Harness::new();
    let s = h.default_snapshotter();
    let grant = repo.grant(h.owner);
    let sc = scope(ScopeMode::Worktree, None, None);
    let d1 = s
        .capture(&grant, &sc)
        .expect("c1")
        .consistency_evidence
        .manifest_digest;
    let d1_again = s
        .capture(&grant, &sc)
        .expect("c1'")
        .consistency_evidence
        .manifest_digest;
    assert_eq!(d1, d1_again, "unchanged inputs give the same digest");

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        repo.root.join("src/util.py"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("chmod");
    let snap = s.capture(&grant, &sc).expect("c2");
    assert_ne!(snap.consistency_evidence.manifest_digest, d1, "mode change");
    assert_eq!(entry(&snap, "src/util.py").mode, FileMode::Executable);

    repo.write("src/util.py", b"X = 9\n");
    let d3 = s
        .capture(&grant, &sc)
        .expect("c3")
        .consistency_evidence
        .manifest_digest;
    assert_ne!(
        d3, snap.consistency_evidence.manifest_digest,
        "content change"
    );
}

#[test]
fn every_included_and_skipped_path_is_explained() {
    let repo = base_repo();
    repo.write("assets/logo.bin", b"\x89PNG\0\0\0binary");
    repo.write("big/data.txt", &vec![b'a'; 4096]);
    repo.write(
        "models/weights.bin",
        b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 12\n",
    );
    repo.write("node_modules/dep/index.js", b"module.exports = 1;\n");
    repo.commit_all("assorted");
    let sub = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{sub},vendor/sub"),
    ]);
    repo.git(&["commit", "-q", "-m", "submodule"]);
    repo.write("scratch.txt", b"untracked, not approved\n");
    let h = Harness::new();
    let options = CaptureOptions {
        max_file_bytes: 1024,
        ..CaptureOptions::default()
    };
    let snap = h
        .snapshotter(options)
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");

    for (p, reason) in [
        ("assets/logo.bin", SkipReason::Binary),
        ("big/data.txt", SkipReason::TooLarge),
        ("models/weights.bin", SkipReason::LfsPointer),
        ("node_modules/dep/index.js", SkipReason::Vendored),
        ("vendor/sub", SkipReason::Submodule),
        ("scratch.txt", SkipReason::UntrackedNotApproved),
    ] {
        assert_eq!(skipped_reason(&snap, p), Some(reason), "{p}");
        assert_eq!(entries_for(&snap, p), 0, "{p} is not also included");
    }
    // Gate: everything git knows about is either included or explained.
    let tracked = repo.git(&["ls-files", "-z"]);
    let untracked = repo.git(&["ls-files", "-z", "--others", "--exclude-standard"]);
    let base = repo.git(&["ls-tree", "-r", "-z", "--name-only", "HEAD"]);
    for p in tracked
        .split('\0')
        .chain(untracked.split('\0'))
        .chain(base.split('\0'))
        .filter(|p| !p.is_empty())
    {
        let explained = entries_for(&snap, p) == 1 || skipped_reason(&snap, p).is_some();
        assert!(explained, "{p} is neither included nor explained");
    }
}

#[test]
fn snapshot_bytes_are_stored_for_later_stages() {
    let repo = base_repo();
    let h = Harness::new();
    repo.write("src/app.py", APP_V2);
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    let app = entry(&snap, "src/app.py");
    let id = razista_core::ArtifactId::from(app.digest.clone());
    let (bytes, trust) = h.store.read_artifact_unscoped(&id).expect("stored");
    assert_eq!(bytes, APP_V2);
    assert_eq!(
        trust,
        TrustLabel::Untrusted,
        "repository content is untrusted"
    );
    let base = razista_core::ArtifactId::from(app.base_digest.clone().expect("base"));
    assert_eq!(
        h.store
            .read_artifact_unscoped(&base)
            .expect("base stored")
            .0,
        APP_V1
    );
}

#[test]
fn diff_packet_cites_reviewed_bytes() {
    let repo = base_repo();
    let h = Harness::new();
    repo.write("src/app.py", APP_V2);
    let s = h.default_snapshotter();
    let snap = s
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    let packet = s.diff(&h.owner, &snap).expect("diff");
    assert_eq!(packet.snapshot_id, snap.snapshot_id);
    let item = packet
        .items
        .iter()
        .find(|i| i.span.path == path("src/app.py"))
        .expect("changed file has a hunk");
    let app = entry(&snap, "src/app.py");
    assert_eq!(item.span.file_digest, app.digest);
    assert!(item.span.start_line >= 1 && item.span.end_line <= app.line_count);
    assert_eq!(item.trust, TrustLabel::Untrusted);
    assert_eq!(item.provenance, ContextProvenance::Diff);
    assert!(
        item.content.as_str().contains("-    return a - b"),
        "{}",
        item.content.as_str()
    );
    assert!(
        item.content.as_str().contains("+    return a + b"),
        "{}",
        item.content.as_str()
    );
    assert!(
        packet
            .items
            .iter()
            .all(|i| i.span.path != path("src/util.py")),
        "unchanged files are not diff context"
    );
}

// ---------- review findings ------------------------------------------------

fn gaps_mentioning(snap: &razista_core::Snapshot, needle: &str) -> usize {
    snap.gaps
        .iter()
        .filter(|g| g.subject.as_str().contains(needle) || g.detail.as_str().contains(needle))
        .count()
}

#[test]
fn line_ending_conversion_does_not_fake_modifications() {
    // Finding 1: CRLF on disk, LF in the index, as a checkout would leave it.
    let repo = Repo::new();
    repo.write(".gitattributes", b"*.bat text eol=crlf\n");
    repo.write("run.bat", b"echo one\r\necho two\r\n");
    repo.write("notes.txt", b"alpha\r\nbeta\r\n");
    repo.commit_all("base");
    repo.git(&["config", "core.autocrlf", "true"]);
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(entry(&snap, "run.bat").change, ChangeKind::Unchanged);
    assert_eq!(entry(&snap, "notes.txt").change, ChangeKind::Unchanged);

    // A real edit is still a modification.
    repo.write("run.bat", b"echo one\r\necho three\r\n");
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(entry(&snap, "run.bat").change, ChangeKind::Modified);
}

#[test]
fn filter_attributes_are_reported_not_trusted() {
    let repo = Repo::new();
    repo.write(".gitattributes", b"*.dat filter=custom\n");
    repo.write("table.dat", b"clean form\n");
    repo.commit_all("base");
    repo.write("table.dat", b"smudged form\n");
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert!(
        gaps_mentioning(&snap, "table.dat") > 0,
        "a filtered path whose bytes differ is flagged: {:?}",
        snap.gaps
    );
}

#[test]
fn sparse_checkout_does_not_fake_deletions() {
    // Finding 2.
    let repo = Repo::new();
    repo.write("in/a.py", b"A = 1\n");
    repo.write("out/b.py", b"B = 1\n");
    repo.commit_all("base");
    repo.git(&["sparse-checkout", "set", "--no-cone", "in/"]);
    assert!(!repo.root.join("out/b.py").exists());
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(entry(&snap, "out/b.py").change, ChangeKind::Unchanged);
    assert!(
        snap.manifest
            .entries
            .iter()
            .all(|e| e.change != ChangeKind::Deleted)
    );
}

#[test]
fn deleted_excluded_paths_are_explained_and_flagged() {
    // Finding 3: deleting a vendored config file must not vanish.
    let repo = Repo::new();
    repo.write("vendor/auth/config.php", b"<?php $key = 1;\n");
    repo.write("src/app.py", b"A = 1\n");
    repo.commit_all("base");
    std::fs::remove_file(repo.root.join("vendor/auth/config.php")).expect("rm");
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(
        skipped_reason(&snap, "vendor/auth/config.php"),
        Some(SkipReason::Vendored)
    );
    assert!(
        gaps_mentioning(&snap, "vendor/auth/config.php") > 0,
        "{:?}",
        snap.gaps
    );
}

#[test]
fn changed_skipped_inputs_raise_visible_gaps() {
    // Finding 4: binary, oversized and unsafe changes are never silent.
    let repo = Repo::new();
    repo.write("keys/server.p12", b"\0\x01binary-v1");
    repo.write("keys/static.bin", b"\0\x01never changes");
    repo.write("data/big.txt", &vec![b'a'; 2000]);
    repo.write("data/big_same.txt", &vec![b'c'; 3000]);
    repo.write("vendor/lib.js", b"v1\n");
    repo.commit_all("base");
    repo.write("keys/server.p12", b"\0\x01binary-v2");
    repo.write("data/big.txt", &vec![b'b'; 2000]);
    repo.write("vendor/lib.js", b"v2 unstaged edit\n");
    for i in 0..3 {
        repo.write(&format!("scratch/{i}.tmp"), b"x\n");
    }
    let h = Harness::new();
    let snap = h
        .snapshotter(CaptureOptions {
            max_file_bytes: 1024,
            ..CaptureOptions::default()
        })
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    for p in ["keys/server.p12", "data/big.txt", "vendor/lib.js"] {
        assert!(
            gaps_mentioning(&snap, p) > 0,
            "{p} changed but no gap: {:?}",
            snap.gaps
        );
    }
    assert_eq!(
        gaps_mentioning(&snap, "keys/static.bin"),
        0,
        "unchanged binary is quiet"
    );
    assert_eq!(
        gaps_mentioning(&snap, "data/big_same.txt"),
        0,
        "an unchanged oversized file is quiet (its streamed object id matches git's)"
    );
    assert_eq!(
        gaps_mentioning(&snap, "scratch/"),
        0,
        "untracked files are summarised, not listed one by one"
    );
    assert!(
        snap.gaps
            .iter()
            .any(|g| g.detail.as_str().contains("3 untracked")),
        "{:?}",
        snap.gaps
    );
}

#[test]
fn file_limit_keeps_changed_files_and_never_errors() {
    // Finding 5.
    let repo = Repo::new();
    for name in [
        "a1.py",
        "a2.py",
        "a3.py",
        "a4.py",
        "z1.py",
        "z2.py",
        "zz_gone.py",
    ] {
        repo.write(name, format!("{name} = 1\n").as_bytes());
    }
    repo.commit_all("base");
    repo.write("z1.py", b"changed\n");
    repo.write("z2.py", b"changed too\n");
    std::fs::remove_file(repo.root.join("zz_gone.py")).expect("rm");
    let h = Harness::new();
    let snap = h
        .snapshotter(CaptureOptions {
            max_files: 3,
            ..CaptureOptions::default()
        })
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("limits degrade, never fail");
    assert_eq!(entry(&snap, "z1.py").change, ChangeKind::Modified);
    assert_eq!(entry(&snap, "z2.py").change, ChangeKind::Modified);
    assert_eq!(entry(&snap, "zz_gone.py").change, ChangeKind::Deleted);
    assert!(
        snap.manifest
            .skipped
            .iter()
            .any(|s| s.reason == SkipReason::FileLimit)
    );
    assert!(
        snap.gaps
            .iter()
            .any(|g| g.code == Some(ErrorCode::BudgetExhausted))
    );
}

#[test]
fn many_untracked_and_vendored_files_fit_the_bounds() {
    let repo = base_repo();
    for i in 0..1200 {
        repo.write(&format!("node_modules/pkg{i}/index.js"), b"x\n");
    }
    repo.commit_all("vendor drop");
    for i in 0..1200 {
        repo.write(&format!("node_modules/pkg{i}/index.js"), b"y\n");
        repo.write(&format!("tmp/u{i}.txt"), b"u\n");
    }
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("bounded capture");
    assert!(snap.gaps.len() <= 1000);
    assert!(
        gaps_mentioning(&snap, "1200") > 0,
        "aggregated: {:?}",
        snap.gaps.len()
    );
}

#[test]
fn partial_clone_missing_objects_are_not_misattributed() {
    // Finding 6: a base blob absent locally must not turn a change into an add.
    let source = Repo::new();
    source.write("src/app.py", APP_V1);
    let c1 = source.commit_all("one");
    source.write("src/app.py", APP_V2);
    let c2 = source.commit_all("two");
    source.git(&["config", "uploadpack.allowFilter", "true"]);
    let clone = Repo::empty();
    clone.clone_from(&source, &["--filter=blob:none", "--no-checkout"]);
    let before = clone.fingerprint();
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(
            &clone.grant(h.owner),
            &scope(ScopeMode::Range, Some(&c1), Some(&c2)),
        )
        .expect_err("objects are missing locally");
    assert_eq!(err.code(), ErrorCode::BaseUnavailable);
    assert_eq!(clone.fingerprint(), before, "nothing was fetched");
}

#[test]
fn shallow_boundary_is_not_a_root_commit() {
    let source = base_repo();
    source.write("src/app.py", APP_V2);
    source.commit_all("two");
    let clone = Repo::empty();
    clone.clone_from(&source, &["--depth", "1"]);
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(
            &clone.grant(h.owner),
            &scope(ScopeMode::Commit, None, Some("HEAD")),
        )
        .expect_err("parent is beyond the shallow boundary");
    assert_eq!(err.code(), ErrorCode::BaseUnavailable);
}

#[test]
fn lone_carriage_return_does_not_shift_spans() {
    // Finding 7: lines are counted by '\n' everywhere.
    let mut v1 = b"a = 1\rb = 2\n".to_vec();
    for i in 2..=12 {
        v1.extend_from_slice(format!("l{i} = {i}\n").as_bytes());
    }
    let v2 = String::from_utf8(v1.clone())
        .expect("utf8")
        .replace("l12 = 12", "l12 = 99");
    let repo = Repo::new();
    repo.write("m.py", &v1);
    repo.commit_all("base");
    repo.write("m.py", v2.as_bytes());
    let h = Harness::new();
    let s = h.default_snapshotter();
    let snap = s
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(entry(&snap, "m.py").line_count, 12);
    let packet = s.diff(&h.owner, &snap).expect("diff");
    let item = &packet.items.as_slice()[0];
    assert_eq!(
        (item.span.start_line, item.span.end_line),
        (9, 12),
        "{}",
        item.content.as_str()
    );
    assert!(item.content.as_str().contains("+l12 = 99"));
}

#[test]
fn emptied_file_shows_what_was_removed() {
    // Finding 8.
    let repo = base_repo();
    repo.write("src/app.py", b"");
    let h = Harness::new();
    let s = h.default_snapshotter();
    let snap = s
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    let app = entry(&snap, "src/app.py");
    let packet = s.diff(&h.owner, &snap).expect("diff");
    let item = packet
        .items
        .iter()
        .find(|i| i.span.path == path("src/app.py"))
        .expect("removed content is shown");
    assert!(item.content.as_str().contains("-    return a - b"));
    assert_eq!(
        Some(&item.span.file_digest),
        app.base_digest.as_ref(),
        "cites the base bytes"
    );
    assert_eq!((item.span.start_line, item.span.end_line), (1, 2));
}

// ---------- security review of the T06 commit ------------------------------

#[test]
fn capture_respects_the_grants_allowed_scopes() {
    let repo = base_repo();
    let h = Harness::new();
    let mut grant = repo.grant(h.owner);
    grant.allowed_scopes =
        razista_core::BoundedVec::new(vec![ScopeMode::Worktree]).expect("bounded");
    let s = h.default_snapshotter();
    s.capture(&grant, &scope(ScopeMode::Worktree, None, None))
        .expect("allowed scope");
    for mode in [
        ScopeMode::Commit,
        ScopeMode::Staged,
        ScopeMode::Range,
        ScopeMode::Branch,
    ] {
        let err = s
            .capture(&grant, &scope(mode, Some("HEAD"), Some("HEAD")))
            .expect_err("scope not granted");
        assert_eq!(err.code(), ErrorCode::PolicyDenied, "{mode:?}");
    }
}

#[test]
fn diff_reads_only_the_snapshot_owners_bytes() {
    let repo = base_repo();
    let h = Harness::new();
    repo.write("src/app.py", APP_V2);
    let s = h.default_snapshotter();
    let snap = s
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    let err = s
        .diff(&razista_core::CallerId::new(), &snap)
        .expect_err("another caller cannot rebuild this diff");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn capture_for_loads_the_grant_from_the_store() {
    // Review finding 10: the entry point takes (caller, repo id), never a
    // caller-built grant.
    let repo = base_repo();
    let h = Harness::new();
    let grant = repo.grant(h.owner);
    let s = h.default_snapshotter();
    let sc = scope(ScopeMode::Worktree, None, None);
    let err = s
        .capture_for(&h.owner, grant.repo_id, &sc)
        .expect_err("unregistered repository");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
    h.store.register_repo(&grant).expect("registered");
    s.capture_for(&h.owner, grant.repo_id, &sc)
        .expect("owner captures");
    let err = s
        .capture_for(&razista_core::CallerId::new(), grant.repo_id, &sc)
        .expect_err("another caller");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

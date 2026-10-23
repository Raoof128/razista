//! T06: hostile paths, git indirection and byte-safe names.

mod common;

use common::*;
use razista_core::{ErrorCode, FileMode, GapKind, RepoPath, ScopeMode, SkipReason};
use razista_repo::CaptureOptions;

const SECRET: &[u8] = b"CANARY-SECRET-7d1f";

fn repo_with(files: &[(&str, &[u8])]) -> Repo {
    let repo = Repo::new();
    for (p, b) in files {
        repo.write(p, b);
    }
    repo.commit_all("base");
    repo
}

#[test]
fn symlink_escape_is_rejected() {
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    std::fs::write(repo.outside.join("secret.txt"), SECRET).expect("secret");
    std::os::unix::fs::symlink(repo.outside.join("secret.txt"), repo.root.join("leak.txt"))
        .expect("symlink");
    std::os::unix::fs::symlink(
        "../../outside/secret.txt",
        repo.root.join("src/rel_leak.txt"),
    )
    .expect("relative symlink");
    std::os::unix::fs::symlink("app.py", repo.root.join("src/alias.py")).expect("internal link");
    repo.commit_all("links");
    let h = Harness::new();

    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(
        skipped_reason(&snap, "leak.txt"),
        Some(SkipReason::UnsafePath)
    );
    assert_eq!(
        skipped_reason(&snap, "src/rel_leak.txt"),
        Some(SkipReason::UnsafePath)
    );
    let alias = entry(&snap, "src/alias.py");
    assert_eq!(
        alias.mode,
        FileMode::Symlink,
        "internal links are recorded, not followed"
    );
    assert!(
        !h.all_stored_bytes()
            .windows(SECRET.len())
            .any(|w| w == SECRET),
        "secret bytes outside the repository never entered the store"
    );
}

#[test]
fn symlinked_directory_component_is_not_followed() {
    let repo = repo_with(&[("pkg/mod.py", b"M = 1\n")]);
    std::fs::create_dir_all(repo.outside.join("pkg")).expect("mkdir");
    std::fs::write(repo.outside.join("pkg/mod.py"), SECRET).expect("secret");
    std::fs::remove_dir_all(repo.root.join("pkg")).expect("rm");
    std::os::unix::fs::symlink(repo.outside.join("pkg"), repo.root.join("pkg")).expect("dir link");
    let h = Harness::new();

    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(
        skipped_reason(&snap, "pkg/mod.py"),
        Some(SkipReason::UnsafePath)
    );
    assert!(
        !h.all_stored_bytes()
            .windows(SECRET.len())
            .any(|w| w == SECRET)
    );
}

#[test]
fn fifo_in_place_of_a_file_is_skipped_not_read() {
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    std::fs::remove_file(repo.root.join("src/app.py")).expect("rm");
    let status = std::process::Command::new("mkfifo")
        .arg(repo.root.join("src/app.py"))
        .status()
        .expect("mkfifo");
    assert!(status.success());
    let h = Harness::new();
    // Opening a FIFO for reading would block forever; capture must not hang.
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect("capture");
    assert_eq!(
        skipped_reason(&snap, "src/app.py"),
        Some(SkipReason::UnsafePath)
    );
}

#[test]
fn non_utf8_path_round_trips() {
    let repo = repo_with(&[("README.md", b"# r\n")]);
    // APFS refuses non-UTF-8 names, so the path goes straight into the index.
    let raw: &[u8] = b"src/caf\xe9.py";
    repo.index_raw(raw, b"C = 1\n");
    let head = {
        repo.git(&["commit", "-q", "-m", "latin-1 name"]);
        repo.git(&["rev-parse", "HEAD"])
    };
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Commit, None, Some(&head)),
        )
        .expect("capture");
    let want = RepoPath::from_bytes(raw).expect("byte-safe path");
    let e = snap
        .manifest
        .entries
        .iter()
        .find(|e| e.path == want)
        .expect("non-UTF-8 path preserved");
    assert_eq!(e.path.as_bytes(), raw);
    assert_eq!(e.path.display(), "src/caf%E9.py");
    let json = serde_json::to_string(&snap).expect("serialise");
    let back: razista_core::Snapshot = serde_json::from_str(&json).expect("round trip");
    assert_eq!(back, snap);
}

#[test]
fn case_collision_is_reported() {
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    repo.index_raw(b"docs/Readme.md", b"one\n");
    repo.index_raw(b"docs/README.md", b"two\n");
    repo.git(&["commit", "-q", "-m", "case twins"]);
    let head = repo.git(&["rev-parse", "HEAD"]);
    let h = Harness::new();
    let snap = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Commit, None, Some(&head)),
        )
        .expect("capture");
    let gap = snap
        .gaps
        .iter()
        .find(|g| g.kind == GapKind::CaseCollision)
        .expect("collision reported");
    assert!(gap.subject.as_str().contains("docs/Readme.md"), "{gap:?}");
    assert!(gap.subject.as_str().contains("docs/README.md"), "{gap:?}");
    assert_eq!(entries_for(&snap, "docs/Readme.md"), 1);
    assert_eq!(entries_for(&snap, "docs/README.md"), 1);
}

#[test]
fn linked_worktree_requires_authorised_common_dir() {
    let main = repo_with(&[("src/app.py", b"A = 1\n")]);
    let linked = main.outside.join("linked");
    main.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        linked.to_str().expect("utf-8"),
    ]);
    let linked = linked.canonicalize().expect("canonical");
    let h = Harness::new();
    let mut grant = main.grant(h.owner);
    grant.canonical_identity =
        razista_core::Text::new(linked.to_str().expect("utf-8")).expect("text");

    let err = h
        .default_snapshotter()
        .capture(&grant, &scope(ScopeMode::Worktree, None, None))
        .expect_err("common dir not authorised");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);

    let options = CaptureOptions {
        authorised_git_dirs: vec![main.root.join(".git")],
        ..CaptureOptions::default()
    };
    let snap = h
        .snapshotter(options)
        .capture(&grant, &scope(ScopeMode::Worktree, None, None))
        .expect("authorised linked worktree");
    entry(&snap, "src/app.py");
}

#[test]
fn hostile_gitfile_is_rejected() {
    let victim = repo_with(&[("secret.py", SECRET)]);
    let repo = Repo::new();
    std::fs::remove_dir_all(repo.root.join(".git")).expect("rm .git");
    std::fs::write(
        repo.root.join(".git"),
        format!("gitdir: {}\n", victim.root.join(".git").display()),
    )
    .expect("gitfile");
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect_err("gitdir points outside the grant");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn symlinked_git_dir_is_rejected() {
    let victim = repo_with(&[("secret.py", SECRET)]);
    let repo = Repo::new();
    std::fs::remove_dir_all(repo.root.join(".git")).expect("rm .git");
    std::os::unix::fs::symlink(victim.root.join(".git"), repo.root.join(".git")).expect("link");
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect_err(".git is a symlink");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn object_alternates_are_rejected() {
    let other = repo_with(&[("x.py", b"X = 1\n")]);
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    std::fs::write(
        repo.root.join(".git/objects/info/alternates"),
        format!("{}\n", other.root.join(".git/objects").display()),
    )
    .expect("alternates");
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect_err("alternates read another object store");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn option_shaped_revisions_are_refused() {
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    let h = Harness::new();
    let s = h.default_snapshotter();
    let grant = repo.grant(h.owner);
    let out = repo.outside.join("written");
    for bad in [
        format!("--output={}", out.display()),
        "-c".to_owned(),
        "HEAD extra".to_owned(),
        "HEAD\nmain".to_owned(),
    ] {
        let Ok(rev) = razista_core::Text::<1, 256>::new(bad.clone()) else {
            continue; // the type already refuses control characters
        };
        let mut sc = scope(ScopeMode::Range, Some("HEAD"), Some("HEAD"));
        sc.base = Some(rev);
        let err = s.capture(&grant, &sc).expect_err("refused");
        assert_eq!(err.code(), ErrorCode::InvalidInput, "{bad:?}");
    }
    canary_absent(&out);
}

#[test]
fn repository_root_must_be_canonical_and_absolute() {
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    let h = Harness::new();
    let s = h.default_snapshotter();
    for root in [
        "relative/path".to_owned(),
        format!("{}/src/..", repo.root.display()),
    ] {
        let mut grant = repo.grant(h.owner);
        grant.canonical_identity = razista_core::Text::new(root.clone()).expect("text");
        let err = s
            .capture(&grant, &scope(ScopeMode::Worktree, None, None))
            .expect_err("non-canonical root");
        assert_eq!(err.code(), ErrorCode::InvalidInput, "{root}");
    }
}

// ---------- review findings ------------------------------------------------

#[test]
fn special_files_inside_git_dir_cannot_hang_capture() {
    // Finding 9: a FIFO where git metadata should be must fail fast.
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    let status = std::process::Command::new("mkfifo")
        .arg(repo.root.join(".git/commondir"))
        .status()
        .expect("mkfifo");
    assert!(status.success());
    let grant = repo.grant(razista_core::CallerId::new());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let h = Harness::new();
        let r = h
            .default_snapshotter()
            .capture(&grant, &scope(ScopeMode::Worktree, None, None))
            .map(|_| ());
        let _ = tx.send(r.map_err(|e| e.code()));
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("capture returned instead of blocking on a FIFO");
    assert_eq!(result, Err(ErrorCode::PolicyDenied));
}

#[test]
fn symlinked_git_internals_are_rejected() {
    let other = repo_with(&[("x.py", SECRET)]);
    let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
    std::fs::rename(
        repo.root.join(".git/objects"),
        repo.outside.join("objects.bak"),
    )
    .expect("mv");
    std::os::unix::fs::symlink(
        other.root.join(".git/objects"),
        repo.root.join(".git/objects"),
    )
    .expect("link");
    let h = Harness::new();
    let err = h
        .default_snapshotter()
        .capture(
            &repo.grant(h.owner),
            &scope(ScopeMode::Worktree, None, None),
        )
        .expect_err("objects dir is a link into another repository");
    assert_eq!(err.code(), ErrorCode::PolicyDenied);
}

#[test]
fn symlinks_deep_in_the_object_store_are_rejected() {
    // Security review: a link below objects/ or refs/ must not redirect git.
    let other = repo_with(&[("x.py", SECRET)]);
    for (label, plant) in [
        ("fan-out directory", "objects/zz"),
        ("ref", "refs/heads/evil"),
        ("attributes", "info/attributes"),
    ] {
        let repo = repo_with(&[("src/app.py", b"A = 1\n")]);
        let target = if plant.starts_with("objects") {
            other.root.join(".git/objects/info")
        } else {
            other.root.join(".git/HEAD")
        };
        std::os::unix::fs::symlink(&target, repo.root.join(".git").join(plant)).expect("link");
        let h = Harness::new();
        let err = h
            .default_snapshotter()
            .capture(
                &repo.grant(h.owner),
                &scope(ScopeMode::Worktree, None, None),
            )
            .expect_err("deep link refused");
        assert_eq!(err.code(), ErrorCode::PolicyDenied, "{label}");
    }
}

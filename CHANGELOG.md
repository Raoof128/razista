# Changelog

All notable changes to RAZISTA (Local Ultra Review MCP). Newest first.

---

Raouf: 2026-10-23 (Australia/Sydney) — T06 immutable snapshots
- **Scope:** Plan task T06 (immutable Git and worktree snapshots), its independent review
  fixes, and a post-commit security fix in `ultra-store`.
- **Summary:**
  - New crate `ultra-repo` with `Snapshotter::capture(grant, scope)` and
    `Snapshotter::diff(snapshot)`. Scopes: worktree, staged, commit (explicit merge parent),
    branch (explicit base, merge-base), range, paths. Nothing is fetched and no default
    branch is guessed; unavailable revisions or objects (partial or shallow clones) are
    `BASE_UNAVAILABLE`.
  - Git is run with a cleared environment, no global/system config, an empty private HOME,
    forced `-c` overrides for every program-launching setting, `--no-lazy-fetch`,
    `protocol.allow=never` and no optional locks. Only read-only plumbing is used; working-tree
    bytes are read by Rust through an `openat(O_NOFOLLOW)` walk, never through status/diff.
  - Two passes per attempt, one retry, then `SNAPSHOT_UNSTABLE`. Exact-content renames,
    staged/unstaged without double counting, sparse-checkout aware, CRLF conversion
    reproduced, filter/encoding/ident conversions flagged.
  - Every tracked, base and untracked path is either a manifest entry or a skipped input with a
    reason (binary, too large, LFS pointer, submodule, vendored, generated, unsafe path,
    untracked not approved, file limit); every changed skipped input raises a gap. Changed
    files are kept first under the file limit. Case-only path collisions are reported.
  - Hostile repositories: escaping symlinks, symlinked or special git metadata, unauthorised
    gitfiles/common dirs (linked worktrees need explicit authorisation) and object alternates
    are refused; FIFOs are never read.
  - Diff context cites stored bytes with `\n`-based line numbers; deletion-only hunks cite the
    base bytes. Snapshot bytes are stored as untrusted artefacts owned by the grant owner.
  - `ultra-core`: `ManifestEntry.base_digest`, `Snapshot.gaps`, `GapKind::CaseCollision`;
    manifest bounds raised to 50,000 entries / 200,000 skipped.
  - `ultra-store` (security review): artefacts record the owner who stored them and evidence
    may only cite its job owner's artefacts.
- **Files changed:** `crates/ultra-repo/**` (new), `crates/ultra-core/src/types.rs`,
  `crates/ultra-core/tests/contracts.rs`, `crates/ultra-store/src/{cas,migrations}.rs`,
  `crates/ultra-store/tests/*`, `Cargo.toml`, `Cargo.lock`, `schemas/*`,
  `vendor/manifests/toolchain.json`, `security/threat-model.md`, `AGENT.md`, `CHANGELOG.md`.
- **Verification:** `cargo test --workspace --locked` 154/154 (ultra-repo 39); clippy
  `-D warnings` clean; fmt clean; `audit-sources` 0 violations over 79 files. Mutation checks:
  the hostile fixture fires both filter and fsmonitor under plain `git status`; the symlink and
  alternates tests fail with their guards disabled; the owner-scoping test fails without its check.
- **Follow-ups (deferred minors):** all `rev-parse` failures read as `BASE_UNAVAILABLE`; no retry on
  errors inside a pass; untracked nested repositories are dropped silently; a FIFO named by
  `core.excludesFile`/`include.path` stalls git for 60 s; `git rm --cached` files appear untracked;
  git is not yet digest-pinned (T07).

---

Raouf: 2026-10-23 (Australia/Sydney) — T04 durable store
- **Scope:** Plan task T04 (durable jobs, evidence blobs, ownership), its independent
  review fixes, and one post-commit security-review fix in `ultra-core`.
- **Summary:**
  - New crate `ultra-store`: SQLite (bundled, WAL, `synchronous=FULL`, `fullfsync`)
    plus a content-addressed blob directory. Versioned migrations; a newer schema is
    refused. Owner-only permissions (dirs 0700, files 0600, blobs 0400); a looser
    existing store directory is refused; an exclusive lock file stops a second process.
  - Jobs are committed before `create_job` returns; idempotency is scoped to
    (owner, repo, key) and a reused key with a different preparation is
    `IDEMPOTENCY_CONFLICT`; terminal jobs refuse transitions and new evidence
    (`JOB_TERMINAL`), also enforced by a database trigger. Events are append-only.
  - Blobs publish tmp → fsync → rename → dir fsync → row; recovery removes partial
    files and orphan blobs and reports missing ones; every read re-hashes the bytes.
    Conflicting trust labels for the same bytes collapse to `untrusted`.
  - Ownership: one uniform "not found or not owned" denial for jobs, events,
    evidence, grants, repositories and preparations; artefact bytes are served only
    through evidence the caller owns; preparations need the repo owner; grants are
    loaded only by their caller and their ids are write-once.
  - Retention (30 days job metadata, 7 days unreferenced artefacts) keeps running
    jobs and pinned evidence, keeps the test behind evidence, and lists what it cannot
    erase. At-rest status is reported as `unverified`, never as encrypted.
  - `ultra-core`: new error codes `IDEMPOTENCY_CONFLICT`, `JOB_TERMINAL`; one evidence
    record may back only one verification.
- **Files changed:** `crates/ultra-store/**` (new), `crates/ultra-core/src/{errors,validate}.rs`,
  `crates/ultra-core/tests/contracts.rs`, `Cargo.toml`, `Cargo.lock`, `schemas/*`,
  `vendor/manifests/toolchain.json`, `security/threat-model.md`, `AGENT.md`, `CHANGELOG.md`.
- **Verification:** `cargo test --workspace --locked` 114/114 (ultra-store 33 including a
  real `abort()` crash test); clippy `-D warnings` clean; fmt clean; `audit-sources` 0
  violations over 70 files.
- **Follow-ups (deferred minors):** shard directories not fsynced into `blobs/`; a
  directory inside `tmp/` blocks open; file owner uid and existing `-wal`/`-shm` modes not
  checked; `transition()` has no state machine yet (T11); preparations, grants and repos
  never expire; power-loss durability is configured but not tested.

---

Raouf: 2026-10-23 (Australia/Sydney) — independent review fix pass for T01–T03
- **Scope:** Critical/Important findings from a fresh-context review of T01–T03.
- **Summary:**
  - Evidence gate: a finding now names its `hypothesis_id`; cited verifications must
    be of that hypothesis, cited by no other finding, not disproved, and backed by
    evidence of the kind the class requires (execution / diagnostic / static
    argument) from a producer other than the discoverer. Duplicate source and
    evidence ids are rejected. New `validate_report_against(report, snapshot)` ties
    `sources` and `manifest_digest` to the real snapshot.
  - Policy: `run_preset`, `propose_patch` and `apply_patch` require grants bound to
    job, snapshot and scope (`grant_not_bound`). `Decision` is `#[must_use]`.
    `authorize` documents that it accepts only store-loaded grants.
  - R26 gate: `.rs` files with a shell shebang or an executable bit, and any
    `.cargo/config[.toml]`, are rejected. CI files (`.github/workflows`,
    `.github/actions`) are parsed with `yaml-rust2`; `run:` lines must be single
    Cargo commands with no shell metacharacters; custom `shell:` values and
    non-allowlisted `uses:` actions are rejected; unparseable YAML fails closed.
- **Files changed:** `crates/ultra-core/src/{types,validate,policy,lib}.rs`,
  `crates/ultra-core/tests/{contracts,policy}.rs`, `crates/xtask/src/audit_sources.rs`,
  `crates/xtask/tests/source_policy.rs`, `crates/xtask/Cargo.toml`, `Cargo.toml`,
  `Cargo.lock`, `source-policy.toml`, `schemas/*`, `security/threat-model.md`.
- **Verification:** each fix's test watched failing first, then passing.
  `cargo test --workspace --locked` 80/80 (contracts 28, policy 19, probe 4,
  source_policy 29); clippy `-D warnings` clean; fmt clean; `audit-sources` 0 violations.
- **Follow-ups (deferred minors):** schema looser than serde for `RepoPath` traversal and
  integer maxima; `Timestamp::now()` returns 0 before 1970; bidi controls in `Text`;
  audit walks the filesystem not the git index; fixture exceptions not restricted;
  `offline_strict=false` not rejected for the strict id; no cargo-deny in CI. Grant
  provenance and repository-registry checks land with the store (T04/T11).

---

Raouf: 2026-10-23 (Australia/Sydney)
- **Scope:** Implementation plan tasks T01, T02, T03.
- **Summary:**
  - T01 — Cargo workspace (Rust 1.99.1, edition 2024) with `ultra-core`, `ultra-mcp`
    and `xtask`. `xtask audit-sources` enforces R26: fails closed on non-Rust source,
    unknown file types, shebang scripts, executable non-Rust files, symlinks and
    non-Cargo CI steps; fixture/vendor exceptions must be declared. Official MCP SDK
    `rmcp` pinned to 3.7.0 after a probe showed modern (`server/discover`, per-request
    `_meta`, `resultType`, `ttlMs`/`cacheScope`) and legacy (`initialize`) round trips
    and unknown-argument rejection. llama.cpp pinned by stable tag/commit only; its
    binary digest and the verification VM image are recorded as open blockers.
  - T02 — Domain contracts for the full type registry: UUIDv7 ids, `sha256:` digests,
    bounded control-char-free text, bounded arrays, byte-safe repository paths, closed
    enums, `deny_unknown_fields` everywhere, no command fields. `validate_report`
    enforces section/evidence-class separation, independent verifier identity,
    evidence presence and source-span validity. 27 JSON Schemas generated into
    `schemas/` and checked for staleness and serde/schema agreement.
  - T03 — Trusted profiles (`profiles/offline-strict.toml`) that fail closed;
    `Policy` with a canonical digest; caller-, repo-, job-, snapshot- and scope-bound
    grants; principal ceilings (models read context only, MCP clients never apply
    patches, only local humans issue grants); structured, serialisable decisions;
    `security/threat-model.md`.
- **Files changed:** `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.gitignore`,
  `source-policy.toml`, `.github/workflows/ci.yml`, `crates/xtask/**`,
  `crates/ultra-core/**`, `crates/ultra-mcp/**`, `schemas/*.schema.json`,
  `profiles/offline-strict.toml`, `docs/adr/001-stack.md`,
  `vendor/manifests/toolchain.json`, `compat/client-matrix.json`,
  `security/threat-model.md`, `AGENT.md`, `CHANGELOG.md`.
- **Verification:** `cargo fmt --all -- --check` clean;
  `cargo clippy --workspace --all-targets --locked -- -D warnings` clean;
  `cargo test --workspace --locked` 62/62 pass (source_policy 19, protocol_probe 4,
  contracts 21, policy 18); `cargo run --locked -p xtask -- audit-sources` 0 violations
  over 58 files, and a planted `.py` file makes it exit 1.
- **Follow-ups:** T04 durable store; T05 sandbox (pin a Lima VM image by digest and
  pass isolation tests); T06 safe Git snapshots; T07/T08 offline bundles and the
  llama.cpp binary + a model that fits 8 GiB RAM. Nothing is committed yet.

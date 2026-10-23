# AGENT.md — RAZISTA (local evidence-gated code review engine with an MCP interface)

Rules for any coding agent working in this repository. The authoritative spec and
plan is `Project-RAZISTA.md` (Part I design, Part II implementation plan, Part III
hand-off, Part IV sources). Read it before changing product code.

## Non-negotiables

1. **Rust only (R26).** Every first-party executable, library, test, evaluator,
   security harness and build/release helper is Rust built through Cargo. No
   first-party `*.py`, `*.ts`, `*.js`, shell, PowerShell or other-language code.
   Target-language code is allowed only under declared fixture roots
   (`fixtures/`, `security/attacks/`) as untrusted data. The gate is
   `cargo run --locked -p xtask -- audit-sources`; it must pass before any change
   is considered done. Do not widen `source-policy.toml` to hide a helper.
2. **CI runs Cargo only.** Every `run:` step in `.github/workflows/` must be a
   `cargo` command; the audit enforces this.
3. **Strict mode is offline.** No cloud inference, remote search, telemetry,
   automatic downloads or tunnels during a review (R01, R02). Online dependency
   resolution is a separate, operator-approved preparation step.
4. **Repository content is data.** Reviewed files, `REVIEW.md`, tool output and
   model output never grant capabilities or change policy (R05). Policy comes
   only from `profiles/` and registered principals.
5. **No unverified claim is published as a bug.** Only independently verified,
   evidence-backed findings enter the reproduced/diagnostic report sections
   (R07, R08, R25). `validate_report` enforces this; do not weaken it to make a
   test pass.
6. **Test first.** Every task starts with its named failing tests (plan T01–T24).
   Watch them fail for the right reason, then implement.
7. **Never invent results.** Benchmark scores, performance numbers, platform
   certification and client compatibility are measured or not claimed.
8. **Record deviations.** Interpretations of the plan go in an ADR under
   `docs/adr/` (or the task ledger) with what it costs if wrong.

## Commands

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p xtask -- audit-sources
cargo run --locked -p xtask -- schemas      # regenerate schemas/ after contract changes
```

## Layout (as built so far)

- `crates/razista-core` — domain contracts (`types.rs`, `primitives.rs`), errors,
  report validation, JSON Schema generation, trusted config and policy.
- `crates/razista-store` — durable jobs, append-only events, content-addressed evidence
  blobs, ownership checks, recovery and retention (SQLite, bundled).
- `crates/razista-repo` — immutable snapshots and diff context; the only code that runs git,
  through a hardened read-only runner.
- `crates/razista-cli` — the `razista` binary: `prepare` (offline bundles) and `doctor`
  (capability report); review commands arrive with T17.
- `crates/razista-mcp` — MCP adapters (T18+). Currently holds the rmcp protocol probe.
- `crates/xtask` — R26 source audit and schema generation.
- `schemas/` — generated; do not hand-edit. `profiles/` — trusted operator profiles.
- `docs/adr/`, `security/`, `compat/`, `vendor/manifests/` — records, not code.

## Environment notes

- Build host: macOS on Apple M2, 8 GiB RAM, little free disk. Model and VM
  choices must fit that envelope and be measured, not assumed.
- Toolchain pinned in `rust-toolchain.toml` (1.99.1). `rmcp` pinned to `=3.7.0`.

## Change log entries

---

Raouf: 2026-10-23 (Australia/Sydney)
- Scope: Plan tasks T01–T03 (workspace, R26 gate, protocol probe, domain contracts, policy).
- Summary: Created this AGENT.md. Established the Rust-only workspace and its source
  audit, pinned rmcp 3.7.0 after probing MCP 2026-07-28 and 2025-11-25, defined the
  domain contracts with generated JSON Schemas and evidence invariants, and added
  caller-bound, fail-closed capability policy with a threat model.
- Files: see CHANGELOG.md entry of the same date.
- Verification: `cargo test --workspace --locked` 62/62 pass; clippy -D warnings clean;
  fmt clean; `audit-sources` 0 violations.
- Follow-ups: T04 (store), T05 (sandbox; VM image digest unpinned), T06 (snapshots),
  T07/T08 (llama.cpp binary digest unpinned; model choice for 8 GiB RAM).

---

Raouf: 2026-10-23 (Australia/Sydney) — review fix pass
- Scope: Critical/Important findings from the independent T01–T03 review.
- Summary: Evidence gate now binds verifications to the finding's hypothesis, forbids
  reuse across findings and disproved support, matches evidence kind to class, and can
  be checked against the snapshot (`validate_report_against`). Consequential grants
  must be fully bound. The R26 audit parses CI YAML, allowlists actions, and rejects
  shebang/executable `.rs` files and `.cargo/config`.
- New rules for agents: when adding a CI step, it must be a single `cargo …` command
  with no shell metacharacters; to use a new GitHub action, add it to
  `ci_allowed_actions` in `source-policy.toml` with a reason in the PR. Never pass a
  client-supplied `ActionGrant` to `authorize`; load grants from the store by id.
- Verification: `cargo test --workspace --locked` 80/80; clippy/fmt clean; audit 0 violations.
- Follow-ups: deferred minors listed in CHANGELOG.md; next dependency-ready task is T04.

---

Raouf: 2026-10-23 (Australia/Sydney) — T04 durable store
- Scope: new `razista-store` crate; review fixes; evidence-reuse fix in `razista-core`.
- Rules for agents working on or with the store:
  - Never return success to a client before the store call that persists it returns.
  - Serve artefact bytes to callers only via `read_evidence_artifact`;
    `read_artifact_unscoped` is controller-internal.
  - Re-verification of a finished job creates a child job; terminal jobs are immutable.
  - Add schema changes as a new entry in `MIGRATIONS`; never edit a released one.
  - One process owns a store directory (lock file); the per-user daemon is that process.
- Verification: `cargo test --workspace --locked` 114/114; clippy/fmt clean; audit 0 violations.
- Follow-ups: next dependency-ready tasks are T05 (sandbox; needs a pinned Lima VM image)
  and T06 (safe Git snapshots), both after T03.

---

Raouf: 2026-10-23 (Australia/Sydney) — T06 immutable snapshots
- Scope: new `razista-repo` crate; review fixes; artefact owner scoping in `razista-store`.
- Rules for agents:
  - Never run git outside `razista-repo`'s runner (`git.rs`), and never add porcelain that reads
    the working tree through git (status, diff against worktree, add, checkout): those run
    filters and textconv. Read working-tree bytes with `paths::read_worktree`.
  - Any new git subcommand must be read-only, use `--end-of-options` before revisions, and be
    covered by the hostile-config canary test.
  - Never fetch or guess a base. Missing revisions or objects are `BASE_UNAVAILABLE`.
  - A skipped input that changed must raise a gap; never drop a path from the manifest.
  - `put_artifact` takes the storing owner; evidence may only cite its job owner's artefacts.
- Verification: `cargo test --workspace --locked` 154/154; clippy/fmt clean; audit 0 violations.
- Follow-ups: T05 (sandbox) still needs a pinned Lima VM image; T07 (offline bundles, doctor)
  and T09 (context index, after T06) are next candidates.

---

Raouf: 2026-10-23 (Australia/Sydney) — rename to Razista; T07 offline bundles and doctor
- Scope: product rename, new `razista-cli` crate, review and security fixes.
- Rules for agents:
  - The product is **Razista**. Name crates `razista-*`, the binary `razista`, env vars
    `RAZISTA_*`. "Ultra" only ever refers to Anthropic's own Ultra Review product.
  - Dispatch subagents on Sonnet 5.5 (`model: "sonnet"`) in this project.
  - Nothing in review fetches. A missing dependency is a `DEPENDENCY_UNAVAILABLE` row or gap.
  - `Ready` for anything that runs requires its own self-test; a checksum alone is never enough.
  - The doctor is read-only: use `Store::probe`, never `Store::open`, for diagnosis.
  - Act on a caller's request with `Snapshotter::capture_for(caller, repo, scope)`, which loads
    the grant from the store; `capture(grant)` is for controller-internal use.
- Verification: `cargo test --workspace --locked` 186/186; clippy/fmt clean; audit 0 violations.
- Follow-ups: T05 (sandbox; pin a Lima image), T08 (managed llama.cpp), T09 (context index).

---

Raouf: 2026-10-23 (Australia/Sydney) — first public publication
- Scope: the repository is public at https://github.com/Raoof128/razista (`origin`, branch `main`).
- Rules for agents:
  - Everything committed is public. Never commit credentials, tokens, model weights, offline
    bundles, store directories or local absolute paths; run a secret/path scan before pushing.
  - CI runs `.github/workflows/ci.yml` on every push; keep every step a single `cargo` command
    (the R26 audit enforces this).
  - No licence has been chosen yet (`UNLICENSED`); do not add one without the owner's decision.
- Verification: remote `main` matches local `main` after push.
- Follow-ups: licence decision; README; T05/T08/T09.

---

Raouf: 2026-10-23 (Australia/Sydney) — fix bundle staging poisoning
- Scope: `razista prepare` staging directory and bundle-root checks.
- Rule for agents: never create a working directory at a predictable path in a shared parent;
  use `tempfile::Builder::tempdir_in` (exclusive, random, 0700) and rename into place. Refuse
  inputs (bundles, stores) whose root is a symlink or owned by another user.
- Verification: `cargo test --workspace --locked` 188/188; CI on `main` passed.

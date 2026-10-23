# Changelog

All notable changes to RAZISTA (Local Ultra Review MCP). Newest first.

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

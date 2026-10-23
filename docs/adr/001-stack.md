# ADR 001: Workspace stack and pinned toolchain

- Status: accepted (T01)
- Date: 2026-10-23 (Australia/Sydney)
- Requirements: R17, R21, R26

## Context

The product is a local, evidence-gated code reviewer exposed through a CLI and MCP.
R26 requires every first-party executable component and test to be Rust built through
Cargo. R17 requires tested support for MCP 2026-07-28 and 2025-11-25.

## Decisions

1. **Rust toolchain 1.99.1** (`rust-toolchain.toml`), edition 2024, resolver 3. Newest
   stable release installed on the build machine (rustc 1.99.1, 2026-10-13) with rustfmt
   and clippy. Stable `1.97.1` was also present; 1.99.1 is newer and non-prerelease.
2. **Official MCP SDK `rmcp` = 3.7.0**, pinned with `=` in the workspace manifest.
   `ProtocolVersion` exposes `V_2026_07_28` and `V_2025_11_25`; the SDK implements
   `server/discover`, per-request `_meta` validation, `resultType` gated to 2026 peers,
   and `ttlMs`/`cacheScope` on discovery. The probe in
   `crates/razista-mcp/tests/protocol_probe.rs` demonstrates both revisions end to end
   over stdio framing. The SDK is a boundary adapter; review state lives in application
   handles, not protocol sessions.
3. **Async-trait convention:** native `async fn` in traits (stable since Rust 1.75).
   Where a backend trait needs `dyn` dispatch, return
   `Pin<Box<dyn Future<Output = _> + Send + '_>>` explicitly. The `async-trait` macro
   crate is not used in first-party code.
4. **Cancellation:** `tokio_util::sync::CancellationToken` (added with T05/T08, which
   first need it).
5. **Schemas:** `schemars` 1.x generates JSON Schema from the Rust domain types (T02).
6. **Source-language gate:** `cargo run --locked -p xtask -- audit-sources` enforces R26
   against `source-policy.toml`. It fails closed on unknown file types, shebang scripts,
   executable non-Rust files, symlinks and non-Cargo CI `run:` steps. Fixture and vendor
   exceptions must be declared with a reason; vendor code also needs an inventory.
7. **Inference runtime candidate:** llama.cpp stable tag `v0.7.0`, commit
   `757cad021b357f417065ddbc41a6e8f87c1f8136`, MIT. The stable tag ships no binaries; the
   binary build to run is chosen and digest-verified by the operator-approved `prepare`
   step (T07/T08). The newest prerelease build observed is recorded in
   `vendor/manifests/toolchain.json` for reference only.
8. **Execution backend candidate (macOS host):** a disposable Linux VM through Lima
   2.2.1 (installed). Docker CLI is installed but its daemon was not running, and an
   ordinary container is not a VM-equivalent boundary. No image is pinned yet: T05 must
   pin the VM image by digest and pass its isolation tests before any verification runs.

## Consequences and open risks

- **Hardware:** the build machine has 8 GiB RAM and ~17 GiB free disk. That rules out
  the 32B-class models discussed in the design and leaves little room for a VM image plus
  model weights together. Model choice (T08) must be measured on this envelope; it is
  not assumed.
- The llama.cpp binary and VM image are not yet digest-pinned. T01's protocol and R26
  gates pass; the runtime/backend pins are carried as blockers into T05 and T08 rather
  than fabricated.
- Online dependency resolution (crates.io, GitHub release metadata) happened during this
  setup step. No repository source was transmitted. It is not part of any review.

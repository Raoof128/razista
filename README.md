# Razista

Foundations of a local, evidence-gated code review engine, written in Rust.

**Status: work in progress. Razista cannot review code yet.** Six of its 24 planned tasks are built. They are the contracts, storage and safety layers that a reviewer needs, not the reviewer itself.

## What it is meant to be

A code reviewer that runs entirely on the operator's machine and reports a behaviour as a bug only when a separate verification step has reproduced it. Model inference, code execution and reports stay local, and an MCP interface is planned so that coding assistants can call it.

The full specification and the 24-task implementation plan are in [Project-RAZISTA.md](Project-RAZISTA.md). The specification follows the publicly documented workflow of Anthropic's Ultra Review (several reviewers, then independent verification of their findings). Razista does not use that service and makes no claim about how it works internally.

## What exists today

Tasks T01 to T04, T06 and T07 of the plan:

| Built | Where |
|---|---|
| Domain contracts with 27 generated JSON Schemas, and report validation that enforces the evidence rules | `crates/razista-core`, `schemas/` |
| A caller-bound capability policy that fails closed | `crates/razista-core/src/policy.rs`, `profiles/`, [threat model](security/threat-model.md) |
| A durable store: jobs, append-only events, content-addressed evidence blobs, ownership checks, recovery | `crates/razista-store` |
| Immutable Git and worktree snapshots, taken through a hardened read-only git runner | `crates/razista-repo` |
| `razista prepare`, which assembles an offline bundle from local files, and `razista doctor`, which reports what the machine can do | `crates/razista-cli` |
| A source audit that enforces the Rust-only rule, and schema generation | `crates/xtask` |

## What does not exist

- The sandbox for running untrusted code (T05) and the local inference backend (T08). No model is run and no reviewed code is executed.
- Context retrieval, scanners, discovery, verification, challenge and reports (T09 to T16).
- The review commands and the daemon (T17).
- The MCP server (T18, T19). `crates/razista-mcp` holds a protocol probe test and a placeholder.
- Patch proposals, the benchmark harness, the adversarial suites, packaging and release qualification (T20 to T24).

There is no accuracy, speed or platform figure for Razista. The numbers in the specification are acceptance targets, not results.

The tool reports the same state. On the development machine, `razista doctor --offline` prints `not offline-certified`, lists every component except git as `missing` or `unverified`, and shows `no` for every language under parse, context, analyse, execute and verify.

## Build and test

You need Rust with rustup. `rust-toolchain.toml` pins toolchain 1.99.1, which rustup installs on the first build. Development and CI are on macOS; other platforms are untested.

```
cargo test --workspace --locked
cargo run --locked -p xtask -- audit-sources
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

CI runs these four commands on every push. Run locally on 17 November 2026 at commit `345f869` (macOS, Apple silicon): 188 tests passed and none failed, the source audit found 0 violations, and clippy and fmt were clean.

## Run it

```
cargo run --locked -p razista-cli -- doctor --offline
cargo run --locked -p razista-cli -- doctor --offline --json
```

`razista prepare --spec FILE --out DIR [--protect REPO_ROOT]...` builds an offline bundle from files you supply. It never downloads anything. `crates/razista-cli/tests/cli.rs` shows the spec format.

## Rules the code enforces today

- **Rust only.** `audit-sources` fails on any first-party source file that is not Rust and on any CI step that is not a single `cargo` command.
- **Nothing is fetched during a review.** A missing dependency is reported as a row or a gap, never downloaded.
- **Repository content is data.** Reviewed files, tool output and model output cannot grant a capability or change policy.
- **No unverified claim is published as a bug.** `validate_report` admits a finding to the reproduced section only with independent verification evidence bound to that finding.

## Repository map

| Path | Contents |
|---|---|
| `crates/` | The six workspace crates listed above |
| `schemas/` | Generated JSON Schemas (do not edit by hand) |
| `profiles/` | Trusted operator profiles |
| `docs/adr/` | Architecture decision records |
| `security/` | Threat model |
| `compat/`, `vendor/manifests/` | Client compatibility and toolchain records |
| [Project-RAZISTA.md](Project-RAZISTA.md) | Specification, plan and sources |
| [CHANGELOG.md](CHANGELOG.md) | What each task changed and the tests that were run |
| [AGENT.md](AGENT.md) | Rules for AI coding assistants working in this repository |

## Limits

- It is not usable as a reviewer.
- It is tested on macOS only.
- No licence has been chosen. `Cargo.toml` says `UNLICENSED`: the code is public to read and is not licensed for reuse.
- The code was written with AI coding assistants under the rules in [AGENT.md](AGENT.md). [CHANGELOG.md](CHANGELOG.md) records each task, the review findings that were fixed and the test results.

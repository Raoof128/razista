# Local Ultra Review MCP: complete agent-ready plan (Rust-only revision)

Prepared for Raouf. Research cut-off: 22 October 2026; **Rust-only constraint added 23 October 2026**.

This standalone document includes the full design, implementation tasks, agent hand-off and research references. All first-party software, evaluation and test code **must be Rust**; reviewed-code fixtures and audited third-party binaries are the explicitly delimited exceptions. Companion filenames refer to included sections or files in the ZIP package. Product implementation and benchmarking have not been performed.


---

# PACKAGE GUIDE

# Local Ultra Review MCP: agent-ready planning package

**Prepared for:** Raouf  
**Research cut-off:** 22 October 2026, Australia/Sydney  
**Plan revision:** 23 October 2026, Rust-only product implementation  
**Status:** Proposed design and implementation plan. No product implementation or model/sandbox benchmark has been performed by this package.

## Recommendation

Build a fully local review engine **implemented entirely in Rust for all first-party executable and test code**, with independent finding verification, then expose that engine through a Rust CLI and Rust MCP adapter. Preserve Claude Ultra Review's publicly described multi-reviewer and verification workflow. Do not depend on its cloud service or claim access to its private internals.

The specification contains **26 requirements**, **24 implementation tasks** and a **43-source research catalogue**. Sources include current official documentation, standards and selected original research. The bibliography is targeted, not an exhaustive academic literature review.

## Reading order

| File | Purpose |
|---|---|
| `DESIGN.md` | Product boundaries, architecture, trust model, workflow, evidence classes, MCP contracts, local runtime, evaluation and milestones. |
| `IMPLEMENTATION_PLAN.md` | Workspace layout, shared types, dependencies, exact task interfaces, named tests, commands and acceptance gates. |
| `AGENT_START.md` | Copyable implementation-agent hand-off with scope, invariants and execution discipline. |
| `SOURCES.md` | Australian Harvard-style references, source-to-decision mapping and evidence limitations. |
| `sources.json` | Machine-readable research catalogue for future traceability. |
| `MANIFEST.json` | Package inventory and SHA-256 checksums for the other component files. Not a security certification. |

For a single-document hand-off, use the accompanying `ULTRA_REVIEW_MCP_AGENT_READY_PLAN_RUST_2026-10-23.md`. It contains the design, implementation plan, agent brief and references. In that standalone copy, companion filenames refer to the corresponding included sections or to the ZIP's separate files.

## Mandatory Rust-only engineering constraint (R26)

The language of the **reviewer product itself** is **Rust only**: controller, MCP server, CLI, discovery/verification pipeline, orchestration, storage, sandbox/model/tool adapters, evaluation and benchmarking, adversarial/security test harnesses, release tooling and first-party automated tests must be implemented in Rust and built through Cargo. No Python, JavaScript/TypeScript, Go, Bash/PowerShell or other-language first-party business logic, daemon, helper script or test runner may be substituted. Run an automated repository source-language audit in CI and block violations.

This does **not** limit the languages of repositories being reviewed: Python and TypeScript/JavaScript remain first-wave **review targets**, with Rust, Go and further ecosystems as later review targets. Their source code, example fixtures and tests may naturally be written in those target languages. Pinned third-party programs, including llama.cpp, compilers, scanners and model runtimes, may be written in other languages; we invoke them through narrow, audited Rust adapters, and do not own or extend their source as first-party product code. Non-executable configuration, model prompts, schemas, documentation and workflow declarations may use their native formats.

## Core promises and non-promises

All review inference, code execution, context and reporting stay local in the strict profile. Networked preparation is a separate operator-controlled step. The offline boundary includes the client, model runtime and worker environment, not merely the MCP server process.

Only independently reproduced behavioural findings are labelled reproduced bugs. Deterministic diagnostics and unresolved concerns remain visible under their own labels. Reports always describe scope and omitted verification.

Architecture is specified; accuracy parity, speed, model fit and platform isolation still require implementation and measurement. Performance values in the design are acceptance targets, not achieved results. Example `ultra` commands describe the proposed product and are not currently installed commands.

## Initial delivery target

One safe snapshot, one local reviewer, one independent verifier, one rejected hard negative and one replayable local report. Establish this end-to-end path with network egress denied before expanding the specialist fleet.

---

# PART I: PRODUCT AND ARCHITECTURE SPECIFICATION

# Local Ultra Review MCP: architecture and product specification

**Research cut-off:** 22 October 2026 (Australia/Sydney).  
**Plan revision:** 23 October 2026. All first-party implementation code is required to be Rust.  
**Status:** Proposed design, ready for engineering review and staged implementation. No product code, local model benchmark, or security certification has been completed by this document.  
**Working name:** Ultra Review MCP. This is a descriptive working name, not an Anthropic product or a claim of affiliation.  
**Owner:** Raouf.  
**Companion documents:** `IMPLEMENTATION_PLAN.md`, `SOURCES.md`, `AGENT_START.md`.

## 1. Decision in one paragraph

Build a **fully local, evidence-gated review engine with an MCP interface**, not an MCP wrapper around Claude's cloud review. Preserve the publicly documented Ultra Review behaviour: multiple reviewers investigate a change, a separate verification stage checks findings, and the report concentrates on substantiated bugs. Replace cloud execution with managed local inference and isolated local execution. Use a deterministic Rust coordinator, a bounded fleet of logical agents, a content-addressed evidence store, and explicit capabilities. Ship a CLI first, then MCP adapters over the same engine. Treat matching Claude's accuracy as a hypothesis to test, not a feature already achieved.

### What "the same system" can honestly mean

Anthropic documents a cloud reviewer fleet and independent reproduction/verification of reported findings. It does not publish the complete internal prompts, scheduling algorithm, reviewer headcount, weights, or verification implementation. The compatibility target is therefore the **documented workflow and observable contract**, not an identical private implementation (Anthropic 2026a, [S01]).

The local `/code-review` command and hosted Ultra Review are different products. A command running in a local session is not evidence that its model inference is offline (Anthropic 2026b, [S02]). This design has its own inference backend and never calls `/code-review ultra` as a dependency.

## 2. Intent, assumptions and boundaries

### User requirements carried forward

1. Work on the user's own computer, not a hosted review service.
2. Retain the deep, multi-agent discovery and independent verification character of Ultra Review.
3. Be usable by coding agents through MCP.
4. Produce a comprehensive, implementable plan grounded in current official documentation and research.
5. **Implement the entire first-party review product in Rust.** This includes CLI, MCP, orchestration, reviewers, verification, security/evaluation tools and packaging helpers. Do not substitute Python/TypeScript or other languages for first-party services, scripts or tests.

### Explicit design assumptions

- Interpret local strictly: source code, model prompts, inference, test execution, indexes and reports remain on the machine during a review.
- **Rust-only first-party implementation (R26)** is a hard requirement, not a performance preference. Only data/configuration, reviewed-language fixtures, and audited third-party runtimes may use another language. Distinguish the reviewer implementation language from reviewed repository languages.
- No particular CPU, RAM capacity, GPU or current OS is assumed. Old hardware discussions do not establish the machine on which this will run.
- No existing project repository was supplied for this design. Paths below describe a proposed new workspace, not files inspected in an existing repository.
- Initial deeply supported languages are Python and TypeScript/JavaScript. Other languages have an explicit capability matrix, not a blanket promise.
- External dependency/model acquisition is a separate operator-approved preparation step. Review execution never downloads missing items.
- A person approves repository access and consequential writes. Review generation itself is read-only with respect to the user's original checkout.

### Non-goals for the first release

No cloud inference fallback, hosted dashboard, autonomous merging, automatic PR publishing, production penetration testing, universal theorem proving, mandatory vector database, custom foundational model training, or replacing every mature static analyser. No remote tunnel is part of the offline release.

## 3. What the research changes

| Evidence | Design consequence | Qualification |
|---|---|---|
| Ultra Review documents separate reproduction and verification [S01]. | Independent verification is a first-class stage, not a final prompt saying "check yourself". | The exact proprietary implementation is unknown. |
| GitHub's ReviewBench combines multiple sources of ground truth and distinguishes grounded from augmented evaluation [S28]. | Evaluate both previously labelled bugs and genuinely new valid findings. | Vendor-origin benchmark; do not treat it as the only judge. |
| c-CRAB evaluates review usefulness with executable checks [S29]. | Include behaviour-based, fail/pass evaluation rather than only textual similarity. | Many c-CRAB checks are structural; passing them is not automatically runtime reproduction. |
| SWE-PRBench reports degradation with additional context in its tested configurations [S30]. | Use targeted retrieval and test context ablations. | A preprint result about particular configurations, not proof that repository context is always harmful. |
| Multi-agent failure research identifies orchestration, alignment and verification failures [S31]. | Define roles, termination rules, budgets and structured hand-offs. | Multiple copies of one model can share the same blind spots. |
| Agentless demonstrates the value of a simpler staged baseline in software repair [S32]. | Keep a fixed single-model pipeline as a mandatory baseline. | Repair benchmark results do not establish review quality. |
| AgentDojo, CaMeL and AutoDojo motivate adversarial evaluation and control/data separation [S33-S35]. | Treat repository content as untrusted data and enforce permissions outside the model. | Borrowing a design principle does not inherit another system's security proof. |

These are selected relevant sources, not an exhaustive systematic review of all academic work. This research did not reproduce their experiments. See `SOURCES.md` for dates, provenance and limits.

## 4. Architecture options

| Option | Advantages | Disadvantages | Decision |
|---|---|---|---|
| Existing hosted-agent CLI wrapped in MCP | Quick prototype; familiar user experience. | Cloud dependencies, difficult privacy guarantees, nested agent loops, weak ownership of evidence. | Reject for the requested offline product. |
| Python-only local pipeline | Rapid prototyping and ecosystem integrations. | Violates the first-party Rust-only requirement, including evaluation/test code. | **Reject** even for the evaluation harness. |
| Rust end-to-end first-party implementation with audited third-party runtimes | Typed control plane, CLI/MCP, all orchestration, evaluation, security tests and release helpers in one Cargo workspace. | Third-party model/scanner executables still need audit, sandboxing and provenance. | **Mandatory design**. |

Rust is a **mandatory product implementation language**, not an accuracy feature. Model prefill/decode, context selection, tool latency and verification cost must be profiled before any performance claim. The official Rust MCP SDK supports the current protocol and older compatibility paths, making it preferable to a hand-written protocol stack [S11, S43].

## 5. System layout and trust boundaries

```text
Local CLI or explicitly authorised MCP client
                    |
      thin stdio/HTTP adapter; no review logic
                    |
       unprivileged per-user local service
       policy + job state + budget scheduler
          /           |            \
 snapshot/context   model broker    evidence/store
      workers           |               |
                   managed local     local reports
                   model runtime
                         |
                 structured proposals
                         |
             deterministic execution broker
                         |
          isolated, disposable verification workers
             base / reviewed / patched snapshots
```

### Components

**Controller:** owns the review state machine, validated configuration, job ownership and budgets. It cannot be instructed by repository text to widen permissions.

**Repository worker:** creates immutable snapshots, safe diffs, language manifests and scoped context. Repository parsing and third-party tools run outside the privileged control path.

**Model broker:** owns the model runtime connection, prompt templates, schema validation, token accounting and cancellation. It permits only installed, pinned local models in strict mode.

**Inference runtime:** one managed local server initially. It has no repository-wide filesystem access, no general tools, no internet access and no credentials. It receives bounded context through the broker.

**Execution broker:** accepts typed actions such as `run_preset(preset_id, snapshot_id, test_artifact_id)`. It does not expose an unrestricted host shell to models or MCP clients.

**Verification worker:** disposable environment with writable scratch space and immutable inputs. The supervisor owns collection of exit status, timeouts and artefact hashes. Text printed by code under test is not an authoritative verdict.

**Store:** local SQLite metadata, append-only event records and content-addressed blobs. No external database service is required. Retention, owner-only permissions and crash recovery are explicit features; the at-rest encryption boundary is stated below.

### Process model

Use one unprivileged service per operating-system user so concurrent CLI/MCP clients do not each load their own model or race on the database. A stdio adapter is a thin proxy. Use an OS-restricted local socket or named pipe internally. Optional loopback HTTP requires authentication and origin/host validation. Never run the whole service as administrator/root.

A compromised host administrator, compromised kernel or compromised trusted model/runtime binary is outside the containment guarantee. Residual risks must appear in the threat model.

### Rust-only implementation boundary (hard requirement)

The shipped software is a **Rust workspace**, not just a Rust MCP shim around another-language application. The same requirement applies to the reviewer fleet coordination, verifier, scheduler, source retrieval, domain schema, JSON/SARIF export, benchmarks, security harnesses, test automation, build tooling and packaging helpers. Use first-party `*.rs` source, `cargo test`, `cargo run`, and an `xtask` Rust binary for repository checks/build packaging. Prefer Rust crates and audited FFI/process boundaries. Do not create `eval/*.py`, `security/*.py`, `scripts/*.sh`, `scripts/*.js`, Node worker applications or Python orchestration services for product logic.

**Permitted exceptions are narrow and explicit:** (a) target-language code and test cases captured from or generated for reviewed repositories, stored as untrusted fixture data or executed only in a sandbox; (b) pinned, separately inventoried third-party binaries/libraries such as model runtimes, compilers and scanners, even if their upstream is C/C++/Python/JavaScript; (c) docs, prompts, JSON/TOML/YAML, schema files, CI declarative workflows, benchmark data and other non-executable inputs. An external tool is never permitted to bypass the Rust brokers or security boundaries. Vendor code must be separated from owned application code.

**Release enforcement:** a Rust `xtask audit-sources` command validates the owned-source inventory and rejects new first-party executable source or scripts in non-Rust languages, unless present under an explicitly catalogued fixture or external-vendor exception. T01 builds this gate, T21/T22 implement all evaluation/security runners in Rust, and T24 refuses qualification if the gate fails. This is a build and audit policy, not a proof that Rust eliminates memory-safety, logic or sandbox vulnerabilities.

## 6. Product requirements and immutable invariants

The following IDs are acceptance requirements, not aspirations.

| ID | Requirement |
|---|---|
| R01 | Strict reviews cannot use external model endpoints, remote search, telemetry or automatic downloads. |
| R02 | No silent fallback from local to cloud, from sandbox to host, or from Ultra to a weaker evidence mode. |
| R03 | Every job binds an immutable input snapshot, review scope, policy version, toolchain manifest and model configuration. |
| R04 | The original working tree remains byte-for-byte unchanged during review. |
| R05 | Untrusted repository text, instructions and tool output cannot grant capabilities or alter trusted policy. |
| R06 | Discovery and verification use separate contexts and separately recorded agent identities. |
| R07 | Published bug claims require independently established evidence of the appropriate kind. |
| R08 | Runtime reproduction, deterministic diagnostics and unverified hypotheses remain distinguishable. |
| R09 | Missing dependencies, unsupported languages and failed stages produce visible coverage gaps. |
| R10 | CPU, memory, output, tokens, disk, process count and time have hard limits. |
| R11 | Cancellation terminates managed work; a timed-out worker cannot remain running unnoticed. |
| R12 | Jobs survive client disconnects and recover consistently after service restarts. |
| R13 | Every tool/resource access checks caller, repository grant and job ownership. |
| R14 | Models cannot authorise their own writes, network access, installation or publication. |
| R15 | Cache reuse validates source dependencies, policy, model, prompts, toolchain and environment. |
| R16 | Terminal reports never equate "no verified findings" with "secure" or "fully covered". |
| R17 | The protocol boundary supports tested MCP 2026-07-28 and 2025-11-25 clients. |
| R18 | Patches are proposals until separately approved against the exact target snapshot. |
| R19 | All exported evidence is schema-valid, redacted as required and attributable to a run. |
| R20 | Language support is reported per capability: parse, context, analyse, execute and verify. |
| R21 | Dependencies, rule packs, models and benchmark data have recorded provenance and licence checks. |
| R22 | Runtime failures and schema failures are not counted as successful reviews. |
| R23 | Review quality is measured against static-only and matched-compute single-agent baselines. |
| R24 | Offline privacy is tested for the controller, inference runtime, scanners and execution workers. |
| R25 | Source-grounded evidence can be replayed; an LLM saying "verified" is never sufficient. |
| R26 | **Every first-party executable component, library, CLI/MCP service, reviewer/orchestrator, model/sandbox/tool adapter, benchmark/evaluation harness, adversarial/security-test harness, release/build helper and automated test is implemented in Rust and built/tested through Cargo.** Enforce this with an automated first-party source-language audit. Exceptions are non-executable config/docs/schema/prompt data, explicitly designated untrusted review/attack fixtures in their target languages, and pinned/audited external tools/runtimes. |

## 7. Review scope and snapshot semantics

### Supported scope modes

- `branch`: compare an explicitly resolved merge base with the effective reviewed state.
- `range`: compare two explicitly specified local revisions with documented two-endpoint semantics.
- `commit`: parent-to-commit, with an explicit merge-parent choice for merge commits.
- `staged`: review the index against HEAD, excluding unstaged work.
- `worktree`: review the effective tracked working tree against the selected baseline, including staged and unstaged content without double-counting.
- `paths`: restrict primary changed files while permitting bounded dependency reads; report reduced scope.
- `imported_pr`: review a locally prepared PR snapshot. Fetching remote PRs is not part of a strict review.

### Rules

Resolve the base before scheduling agents. If the intended default branch is unavailable locally, request an explicit available revision or return `BASE_UNAVAILABLE`. Do not guess `main`, silently fetch, or change the comparison after work starts. Git's diff forms have different meanings, so the engine must expose which comparison it used [S18].

Capture HEAD, base/merge-base, index content, effective worktree, file modes, deletions, renames, and optionally approved untracked files. Record each included file's digest. Use two-pass consistency checks or an equivalent snapshot mechanism to detect concurrent edits. If inputs change during capture, retry once, then return `SNAPSHOT_UNSTABLE` rather than mixing versions.

Use engine-generated Git configuration inside a read-only metadata worker. Disable hooks, external diff drivers, text conversion, credential helpers, network fetches and implicit submodule/LFS operations. Do not run Git commands against an untrusted checkout with the user's normal environment. Handle linked worktrees through explicitly authorised common-directory resolution.

Reject path traversal, symlink escapes, hostile `.git` indirection, device files, sockets and unsafe archive members. Preserve non-UTF-8 paths internally as byte-safe identifiers; provide escaped display names. Detect case-fold collisions. Do not silently drop files because their names are awkward.

**Default file limits:** 2 MiB per source-text file and 20,000 candidate tracked files per initial job. These are proposed guardrails, not capacity claims. Larger jobs require an explicit local budget profile. Binary files, unresolved LFS pointers, submodules and generated/vendor directories appear in a skipped-input manifest. Exclusions may not conceal an entire changed authentication/configuration file without a visible warning.

## 8. End-to-end Ultra pipeline

### Stage A: preflight and plan

Check repository grant, immutable scope, language capabilities, model installation, hardware budget, sandbox self-test, offline dependencies, rule-pack versions and available storage. Return a coverage/cost estimate in local resource terms, not a promise of wall-clock duration.

Produce `PreparedReview` containing the frozen snapshot, resolved scope, planned roles, disabled capabilities, budget and policy digest. A caller must start that exact preparation; changed inputs require another preparation.

### Stage B: deterministic baseline

Run approved parsers, compiler/type checks and security/secret/dependency scanners. Preserve structured results as **observations**, not automatically confirmed vulnerabilities. Scan corresponding base/head states where meaningful to separate newly introduced from existing diagnostics.

Do not allow a lint flood to displace verification of correctness/security candidates. Give deterministic findings their own queue and budget.

### Stage C: bounded independent discovery

Build role-specific context packets. Schedule logical reviewers as resources permit. Each reviewer can request additional context through bounded internal tools. Reviewers cannot see each other's initial findings before completing their first pass.

Each hypothesis must name: affected symbol/location, violated invariant, triggering conditions, plausible impact, evidence references, and a proposed way to falsify it. Reject vague style complaints from the bug queue. Style advice may be a separately enabled report category.

### Stage D: preliminary grouping

Group likely duplicates to avoid repeating identical verification work, while retaining each original hypothesis and provenance. Grouping is not confirmation. Never drop a different trigger or affected trust boundary simply because descriptions sound similar.

### Stage E: independent verification

A new verifier receives the frozen snapshot, trusted requirements, cited locations and a concise falsifiable claim. It does not receive the discoverer's conversational history or asserted confidence as proof. It rebuilds the necessary context and attempts to confirm or disprove the claim.

Where possible, produce a minimal regression test, counterexample, compiler diagnostic, bounded static-analysis result or controlled benchmark. Every test is untrusted code and runs through the execution broker. Respect a per-candidate attempt budget.

### Stage F: challenge and adjudication

For high-severity or uncertain findings, a challenger searches for counterexamples to the finding: intended behaviour, existing validation, unreachable paths, language semantics, caller preconditions or misleading tests. The challenge cannot grant new capabilities. An evidence validator, not an unconstrained synthesising model, determines report eligibility.

### Stage G: deduplicate, rank and report

Rank by demonstrated impact, reachability and evidence strength. A model's numerical self-confidence is never the sole ranking signal. Keep severity and certainty separate. Produce short actionable findings with expandable evidence, scope and coverage gaps.

### Stage H: optional patch proposal

Create a minimal patch against the reviewed snapshot in scratch space. Re-run the reproducer and relevant regression checks. The proposed patch must not delete tests, weaken security checks, expand permissions or change unrelated behaviour without an explicit explanation. It is not applied to the real checkout automatically.

## 9. Logical agent roster and bounded scheduling

| Role | Activation | Primary concerns |
|---|---|---|
| Correctness | Always | Control flow, boundary inputs, error propagation, arithmetic and data transformations. |
| Security | Always | Authorisation, injection, secret exposure, unsafe deserialisation, trust-boundary violations. |
| Contracts | Always | API compatibility, schema changes, caller/callee assumptions, serialisation and migration semantics. |
| State/concurrency | Conditional | Transactions, races, cancellation, retries, idempotency, partial failures. |
| Performance/resources | Conditional | Demonstrable complexity regressions, leaks, unbounded queues, resource exhaustion. |
| Tests/regressions | Always for supported languages | Missing regression coverage for changed behaviour; test/code disagreements. |
| Verifier | Per surviving hypothesis | Independent evidence construction and disconfirmation. |
| Challenger | High-impact or contested findings | Strongest plausible explanation that the finding is wrong. |

The coordinator is deterministic. A planning model may propose a bounded work partition, but it cannot change job policy or budgets.

**Initial scheduling defaults:** one active inference request; two CPU verification workers; up to six logical discovery roles; a maximum of two verification-repair attempts per candidate; a maximum of 24 candidates admitted to expensive verification. Raise limits only through a selected local profile after measurement. Hitting the candidate limit produces a visible partial-coverage status.

Use a shared model runtime instead of loading one full copy per agent. Model role independence means fresh contexts and independently constructed evidence, not mathematically independent errors. A second local model can be an evaluated challenger option later, not a default requirement.

## 10. Context engine

### Retrieval order

1. Changed hunks and explicit old/new boundaries.
2. Enclosing definitions, types and relevant imports.
3. Callers/callees and interfaces directly affected by the change.
4. Existing tests, schemas and error-handling contracts.
5. Additional material requested for a specific unresolved question.

Use Tree-sitter for syntactic structure and incremental parsing, not as a complete cross-language semantic/data-flow analyser [S17]. Prefer compiler/language-service facts when safely available. Run services that can load plugins or build projects in isolated workers.

Store file/symbol relationships in SQLite first. Add local embeddings only after an ablation demonstrates an accuracy or latency benefit. A vector database is not necessary to ship the first useful review.

Every packet carries source snapshot ID, file digest, line range, symbol anchor, trust label and truncation flags. An agent may not cite a source it has not actually received or retrieved. Context requests are logged and budgeted.

### Context budget policy

Start with a maximum 12,000 input tokens per reviewer packet, subject to the installed model's tested limits. Reserve output and additional retrieval capacity. Use a deterministic truncation strategy that keeps the changed code and contract before low-relevance neighbours. Report truncation rather than pretending to have analysed omitted material.

This is an initial design parameter, not an optimum derived from SWE-PRBench. Compare diff-only, targeted context and expanded context on held-out cases [S30].

### Cache validity

A reusable artefact key includes snapshot/dependency closure, tool and rule versions, policy, prompt version, model weights, tokenizer, quantisation, relevant generation settings and environment image digest. Changed imports, build files, feature flags, schemas or generated artefacts invalidate affected dependants. Findings never become "still verified" merely because the diff text is unchanged.

Caches are repository-scoped and permission-scoped. Shared caches across unrelated users or repositories are disabled. Trust labels survive serialisation and retrieval.

## 11. Evidence semantics: the core product contract

### Distinct evidence classes

| Class | What it establishes | Default presentation |
|---|---|---|
| `runtime_reproduced` | A supervisor-observed execution demonstrates a violated, sourced behavioural contract under recorded conditions. | Confirmed findings. |
| `deterministic_diagnostic` | A pinned analyser/compiler independently reproduces a specific diagnostic under recorded settings. | Separate verified-diagnostics section; never relabelled as runtime reproduction. |
| `bounded_static_proof` | A checkable argument establishes a violation under explicit assumptions and analysis bounds. | Separate verified-diagnostics section, with assumptions. |
| `unverified` | A plausible claim lacks sufficient evidence, or the environment prevents verification. | Open questions / coverage gaps, not confirmed bugs. |
| `disproved` | Evidence contradicts the hypothesis or shows intended behaviour. | Audit log; omitted from normal findings. |
| `inconclusive` | Conflicting, flaky, exhausted-budget or inadequate-oracle results. | Open questions, with reasons. |

The strict Ultra default presents only `runtime_reproduced` entries as reproduced bugs. Users can view verified diagnostics and high-priority unresolved questions without blurring those labels. A credible but unverified critical concern must not disappear; it appears as an explicitly unverified alert, not a proven vulnerability.

### Differential test matrix

For a suspected newly introduced bug, attempt:

| State | Expected result |
|---|---|
| Trusted base | Contract holds. |
| Reviewed state | Contract violation reproduces. |
| Proposed patch, when available | Contract holds and unrelated regression tests remain healthy. |

A base/head comparison strengthens causal attribution but is not always possible: a new API may not exist on the base. Then the claim needs a trusted specification or an independent oracle, and the report must not assert base-pass evidence that was not obtained. A pre-existing bug remains labelled pre-existing.

A test failing because dependencies cannot install, an import is wrong, a compiler is absent, or the worker times out is an environment failure, not proof of the suspected bug. Record failure stage and structured reason.

### Oracle requirements

The expected behaviour must come from approved requirements, established API semantics, trusted existing tests, independently checked invariants or a valid reference implementation. Generated tests cannot define product intent by themselves. Reject tautological tests, tests that merely search for the proposed fix's text, and tests that suppress errors to force the expected result.

Property-based tests are useful when an invariant can be stated independently; Hypothesis is an available Python implementation [S27]. Save seeds and minimise counterexamples. Race/performance findings require repeated measurements and explicit uncertainty, not one noisy run.

### Evidence bundle

Each verified item stores: job/snapshot IDs; hypothesis and verifier IDs; model/prompt identifiers; contract source; tool/image digests; exact typed execution request; test artefact digest; supervisor-recorded exit/outcome; redacted logs; observed and expected behaviour; base/head comparison; reproduction count; resource limits; unresolved assumptions; and relevant source spans.

Evidence integrity hashes show which artefacts were used. Optional signing later authenticates the evidence bundle and signer, not the truth of every conclusion or the absence of vulnerabilities.

## 12. Safety and privacy model

### Adversary

Assume a reviewed repository can contain malicious source, tests, build scripts, compiler plugins, package hooks, filenames, comments, Markdown, dependency metadata, terminal escape sequences and forged tool output. Assume a connected model can be prompt-injected and can emit invalid or malicious tool arguments.

### Controls

- Trusted policy is generated outside the reviewed changes. Repository review guidance is advisory data from an approved base snapshot; a PR cannot approve its own new rules.
- Repository instructions cannot override model configuration, egress, filesystem roots, tools, budgets or approval requirements.
- Use typed capability grants for repository read, scratch execution, report export and patch application. Bind grants to caller, job, snapshot, permitted action and expiry.
- No arbitrary host shell tool. No Docker/Podman socket, SSH agent, keychain, cloud credentials, host home directory or host package cache enters a verification worker.
- No untrusted repository code gets access to the model service, controller socket, audit database or approval keys.
- Sanitise ANSI/OSC terminal sequences, HTML and Markdown output. Reports do not auto-load remote images or scripts.
- Never trust `verified: true`, tool annotations, a test-generated JSON report, or a repository comment as an authorisation decision.
- A model can request approval; only an authenticated local human UI/CLI can mint it. A tool's boolean `approved=true` is not sufficient.

MCP security guidance treats tool execution, authentication and untrusted interactions as implementation responsibilities [S10]. CaMeL motivates separating trusted control from untrusted data, but this design does not claim CaMeL-equivalent formal guarantees [S34].

### Local inference does not automatically mean private end-to-end

A local MCP server can return findings to a cloud-connected host, which can then process that content remotely. Therefore:

- The certified offline path is the standalone local CLI or a verified local-model host.
- Claude Code and Codex compatibility may be tested with non-sensitive fixtures, but connecting their cloud-backed configurations is outside the offline guarantee [S36, S38].
- ChatGPT web integration requires a network-accessible route or supported tunnel. A tunnel still sends requests/results through a remote service; it is not an air gap [S37].
- The first release contains no tunnel and no remote-export feature. Future export would require a separate, explicit user decision and privacy profile.

### Strict offline enforcement

Application-level endpoint validation alone is insufficient: a loopback service can itself proxy to the cloud. Strict mode uses an engine-managed, pinned runtime with verified egress enforcement. It disables proxy environment inheritance, redirects, automatic model downloads, cloud features and telemetry. Network enforcement covers inference, scanning and controller child processes, not just tests.

An OS backend must prove its isolation with self-tests. Where native GPU serving cannot meet that policy, offer a fully local offline VM configuration, even if it is slower. Otherwise refuse strict mode. Never display an offline-certified badge for an unmanaged localhost endpoint.

For Ollama, `OLLAMA_NO_CLOUD=1` is a documented cloud-disable control, but does not by itself certify the complete process/network environment or eliminate updater behaviour [S12].

## 13. Execution isolation and platform strategy

### Required sandbox behaviour

Immutable base image; read-only input snapshot; per-job writable scratch; no external network; minimal environment; non-root workload; dropped capabilities; no privilege escalation; bounded process count, CPU, RAM, output and disk; supervised process-group teardown; and evidence collection outside the workload's writable namespace.

No GPU device is exposed to untrusted test code by default. Inference is a separate trusted local runtime. Some GPU-specific tests will remain unsupported until a separate isolation profile is evaluated.

### Platform rollout

| Platform | Initial execution approach | Release requirement |
|---|---|---|
| Linux | Disposable local VM, or an explicitly tested gVisor-backed profile. | Escape/egress/resource tests pass; no downgrade on unsupported syscall. |
| macOS | Controller and inference locally; Linux verification VM for supported Linux toolchains. A fully offline VM can host inference where native egress control is not certified. | VM filesystem/network isolation demonstrated; native Apple-platform tests are not claimed. |
| Windows | Local controller with a dedicated isolated Linux VM; native inference only after its egress policy passes. | No host-drive integration in the worker, no inherited credentials. |
| Native Swift/iOS/macOS or Windows-specific targets | Later dedicated OS-appropriate worker. | Until implemented, analysis-only capability and explicit verification gap. |

Ordinary containers are not a claim of VM-equivalent isolation. gVisor narrows host interaction but has documented compatibility and security limits; it does not make every workload supported [S23, S24]. Podman controls provide useful building blocks, not a complete guarantee [S25]. Do not treat an ordinary interactive WSL environment as a disposable security boundary.

### Offline dependency preparation

A separate `prepare` workflow obtains pinned model weights, signed runtime packages, rule bundles, advisory databases and language dependencies with operator approval. It runs without repository source mounted wherever possible. It produces an offline bundle manifest containing hashes, licences, origins and creation timestamps.

During review, only that bundle is available. Missing packages produce `DEPENDENCY_UNAVAILABLE`. Advisory scans state the database's age. Older than seven days triggers a default warning; it does not trigger network access. Private dependencies require an explicitly prepared local cache, never credentials mounted into the worker.

## 14. Local inference strategy

### Backend order

1. **llama.cpp server** as the first managed backend, subject to a pinned release and schema/tool-call contract tests. It documents constrained JSON and concurrent serving features [S13].
2. **MLX-LM** as an Apple-silicon optimisation path once measured; its model and prompt-cache handling must pass the same privacy and evidence rules [S14].
3. **vLLM** for suitable local GPU workstations after the first release; use the current structured-output interface, not obsolete example fields [S15].
4. **Ollama** as a convenience adapter only after managed-runtime, cloud-disable and network checks [S12].

Do not implement four backends before the first end-to-end review works. The `LocalModel` interface permits later adapters without changing the coordinator.

### Model selection

Choose an installed model using a frozen review-specific evaluation set and the actual hardware. Measure schema reliability, false-positive rate, candidate recall, verification success, memory and latency. A code-generation leaderboard is not sufficient.

Qwen3-Coder-Next is one documented open-weight candidate, not the selected winner. Its model card reports 80B total parameters with 3B activated; activated parameters must not be used as the memory requirement [S16]. Re-check model/runtime support and licence terms when generating the installation manifest.

For intuition only, a dense 32-billion-parameter model at nominal four-bit weight storage is approximately 16 GB of raw weights. Quantisation metadata, cache, runtime workspaces, OS and workers add overhead. Actual resident-memory measurements determine admission, not this arithmetic.

### Reliability contract

The model broker records weight/tokenizer digests, quantisation, runtime build, context limit, prompt version and generation settings. Validate all structured output independently. Permit one bounded schema-repair attempt; then record `MODEL_OUTPUT_INVALID`. Never repair by granting additional tools or silently switching to a remote model.

Do not persist private chain-of-thought. Store concise claim rationales, tool requests, evidence and outcomes. Temperature/seed settings do not guarantee bit-identical GPU execution; reproducibility claims must distinguish evidence replay from exact model-output replay.

## 15. Static tools and language coverage

| Capability | Initial tools or method | Important limit |
|---|---|---|
| Python diagnostics | Ruff, approved type checker, pytest in isolated presets. | Configuration/plugins are untrusted; no host execution. |
| JS/TS diagnostics | TypeScript compiler, trusted lint preset, supported test runner in a worker. | Do not load arbitrary host-side ESLint plugins. |
| Security patterns | Semgrep CE with pinned local rule files. | Explicitly disable metrics; no registry fetch during review [S19, S20]. |
| Dependencies | OSV-Scanner with preloaded offline databases. | Report advisory age and unsupported ecosystems [S21]. |
| Secrets | Pinned local scanner with redacted output. | Never print discovered secret values in reports or prompts. |
| Syntax/context | Tree-sitter plus safely obtained compiler facts. | Parsing is not semantic proof [S17]. |
| Advanced code queries | Optional CodeQL adapter. | Licence/entitlement gate; not a universally free private-repository dependency [S22]. |
| Export | JSON, Markdown and SARIF 2.1.0. | Custom evidence strength remains explicit; validate against schema [S26]. |

**Tier 1 reviewed-code targets:** Python and TypeScript/JavaScript with deterministic checks and executable verification. **These are target repository languages only; the Ultra Review engine and its associated first-party software are Rust.**

**Tier 2 reviewed-code targets:** Rust and Go, including build scripts/procedural macros and dependency preparation inside the same isolation model. Rust support as a *review target* is distinct from the mandatory Rust implementation language.

**Tier 3 planned:** Java/Kotlin, C/C++, C#, PHP, Ruby, Dart/Flutter and native Swift. Add each through language-specific contracts and benchmark fixtures. SQL, Terraform, YAML, Kubernetes, Dockerfiles and workflow definitions get dedicated analysers and capability labels, not generic "fully supported" badges.

No automatic fixes in analysis mode. Even tools that classify fixes as safe can change behaviour in project-specific contexts; Ruff exposes a distinction between safe and unsafe fixes [S39]. The engine's approval and regression rules still apply.

## 16. MCP interface and interoperability

### Version policy

Target **MCP 2026-07-28**, with a tested **2025-11-25 compatibility adapter**. The current revision changes session/initialisation semantics and deprecates new use of Roots, Sampling and Logging. Keep review state in explicit application handles, not protocol sessions [S04, S05].

Use configured repository grants rather than requiring Roots. Call the local model backend directly rather than depending on MCP Sampling. Log locally to stderr or local telemetry sinks. The SDK is a boundary adapter, not the workflow engine.

Long work uses the Tasks extension when negotiated; otherwise ordinary tools return durable job handles and polling endpoints. The extension's cancellation semantics do not replace our own process teardown guarantee [S06].

### Modern protocol conformance details

The 2026 adapter implements `server/discover`, validates per-request version/capability metadata, and emits the required `resultType`. Its cacheable list/resource results carry `ttlMs` and `cacheScope`; repository-dependent results use private caching and a conservative freshness interval. Fixed tool discovery ordering must not vary with connection history. Record the exact SDK support in protocol fixtures rather than adding a second ad hoc protocol implementation [S05, S07].

The HTTP adapter validates required method/name metadata headers and the selected version's transport rules. Modern request streams are not resumable via the removed legacy SSE event mechanism. Reissued start calls use application idempotency keys. Do not advertise subscriptions unless implemented and tested; polling is sufficient initially. Ordinary RPC cancellation and cancellation of an already durable review job are different operations. After durable acknowledgement, a client disconnect does not cancel the independent job; explicit job/task cancellation must trigger worker teardown. A response lost before acknowledgement must be recoverable through idempotent retry without duplicating work [S06, S09].


### Public tool set

| Tool | Input contract | Result | Side effect |
|---|---|---|---|
| `review_preflight` | `repo_id`, typed `scope`, `profile_id` | `PreparedReview` handle, snapshot/coverage summary | Creates local immutable preparation. |
| `review_start` | `preparation_id`, `idempotency_key` | Durable `job_id` | Starts bounded local work. |
| `review_status` | `job_id` | Stage, counters, budget, gaps and terminal state | None. |
| `review_findings` | `job_id`, cursor, evidence-class filter | Paginated structured findings | None. |
| `review_evidence` | `job_id`, `evidence_id`, bounded range | Redacted evidence slice | None. |
| `review_cancel` | `job_id` | Cancellation/teardown state | Stops managed work. |
| `review_verify` | `job_id`, `finding_id` | Child verification job | Runs fresh isolated verification. |
| `review_patch` | `job_id`, finding IDs | Patch proposal handle | Writes scratch proposal only. |
| `review_export` | `job_id`, `format`, `redaction_profile` | Authorised report-resource handle | Creates local report. |

Repository registration and patch application are local human CLI operations initially. There is no `run_shell`, `approve`, `install`, `publish_review`, `merge` or unrestricted file-read MCP tool.

Read-only annotations must match actual behaviour: starting work and generating artefacts are not read-only just because they do not modify the original repository. Annotations are advisory; the policy engine enforces permissions [S07].

### Resources and output budgets

Examples: `ultra://jobs/{id}/summary`, `ultra://jobs/{id}/findings`, `ultra://evidence/{id}`, `ultra://capabilities`. Resource IDs are opaque, caller-authorised handles, not arbitrary filesystem URLs.

Return at most 20 findings or 64 KiB per tool response by default. Large evidence is paginated. Every resource access rechecks authorisation, including after reconnection. Do not trust possession of a job ID as authority.

STDIO carries protocol messages only on stdout; diagnostics go to stderr [S08]. Loopback HTTP is optional, bound explicitly to loopback, with client tokens and origin/host validation. Support modern stateless HTTP separately from legacy compatibility; do not mix their session assumptions [S09].

## 17. Data contracts

The implementation uses Rust types with generated JSON Schemas. Required domain objects:

```text
RepoGrant { repo_id, canonical_identity, owner, allowed_scopes, created_at }
ReviewScope { mode, base, head, paths, include_untracked, merge_parent }
Snapshot { snapshot_id, repo_id, resolved_scope, manifest, consistency_evidence }
PreparedReview { preparation_id, snapshot_id, policy_hash, plan, expires_at }
ReviewRequest { preparation_id, idempotency_key, caller }
ReviewJob { job_id, request, stage, status, budget, gaps, created_at }
Hypothesis { id, role, claim, invariant, trigger, locations, context_refs }
Evidence { id, kind, subject_snapshot, producer, action, outcome, artifact_refs }
Verification { id, hypothesis_id, verifier_id, evidence_ids, outcome, assumptions }
Finding { id, claim, severity, evidence_class, introduced_by_change, locations,
          verification_ids, impact, reproduction_summary, limitations }
PatchProposal { id, target_snapshot, file_preconditions, diff_artifact,
                verification_job_ids, risks }
ReviewReport { job_id, status, scope, findings, diagnostics, open_questions,
               coverage, versions, resource_usage, evidence_manifest }
```

Use closed enums, bounded strings/arrays and `additionalProperties: false` at tool boundaries. Do not use loosely typed arbitrary JSON blobs for commands or approvals. Unknown fields produce a validation error rather than acquiring semantics later.

A validated report must not contain a confirmed finding with zero verification/evidence references. Findings reference real manifest paths and source ranges. Severity is a separate enum from evidence class. Unknown business intent produces an explicit unresolved assumption.

## 18. State, persistence and failure behaviour

Internal stages: `queued -> preflight -> snapshot -> indexing -> scanning -> discovering -> verifying -> adjudicating -> reporting`.

Terminal statuses: `completed`, `partial`, `failed`, `cancelled`. Waiting for a local human action is `input_required` and carries a bounded expiry. MCP task states map to these without erasing application-level `partial` information.

Write a durable job record before acknowledging `review_start`. An idempotency key is scoped to caller, repository and preparation. Reusing the key with different inputs is an error. A crashed worker can be retried only if its action is repeatable and its lease has been reclaimed. A restarted controller must not run the same patch-application action twice.

Terminal jobs are immutable; re-verification creates a child job. The store uses transactional metadata and atomic blob publication. Recovery reconciles orphan temporary blobs and expired worker leases. Keep events sufficient to explain decisions, without storing unnecessary source copies or secret-bearing logs indefinitely.

Store directories are owner-only (`0700` on Unix), regular sensitive files are `0600`, and Windows builds use an equivalent owner-restricted ACL. Protect local sockets and tokens with the same ownership boundary. Do not claim that ordinary SQLite or content hashes encrypt data. Initial at-rest confidentiality relies on the operator's encrypted local volume and OS account protection; `doctor` reports its verified or unverified status. Application-managed encrypted storage is a later separately reviewed capability. Strict offline operation does not by itself prove at-rest confidentiality.

Default retention: job metadata 30 days; raw logs/scratch 7 days; pinned evidence until the user releases it. These are configurable local defaults. Deletion removes associated model/context caches and exports where owned, and reports anything outside the engine's control. Do not promise forensic erasure from SSDs or backups.

### Required error distinctions

`BASE_UNAVAILABLE`, `SNAPSHOT_UNSTABLE`, `POLICY_DENIED`, `SANDBOX_UNAVAILABLE`, `EGRESS_POLICY_UNVERIFIED`, `DEPENDENCY_UNAVAILABLE`, `LANGUAGE_UNSUPPORTED`, `MODEL_NOT_INSTALLED`, `MODEL_OUTPUT_INVALID`, `OUT_OF_MEMORY`, `BUDGET_EXHAUSTED`, `WORKER_TIMEOUT`, `EVIDENCE_INVALID`, `STALE_PATCH`, `STORE_UNAVAILABLE`.

Errors have retryability and coverage impact. A failure after useful findings produces a partial report, not a clean pass.

## 19. Performance engineering without wishful thinking

Measure end-to-end latency as snapshot + retrieval + prefill + decode + scanner + verification + reporting time. Record p50/p95 and cold/warm runs separately.

Prioritise incremental parsing, narrow context, model residency, safe prompt caching, batching compatible requests, early duplicate grouping and risk-weighted verification. Avoid blanket context expansion, unlimited parallel agents, repeated full-repository scans and unmeasured model switching.

The scheduler reserves OS headroom, respects a configured memory ceiling, and adapts admission from measured resident memory rather than parameter count alone. On memory pressure it reduces concurrency or returns partial coverage; it does not silently quantise more aggressively or swap models. Quantisation changes require a new quality profile.

Separate scan and inference resource pools so compilers do not starve model serving. Cache dependency bundles read-only; each worker gets isolated writable layers. Never reuse an untrusted build output across unrelated jobs without provenance validation.

**Initial engineering targets, not measured results:** warm `review_status` p95 below 250 ms; job acknowledgement p95 below 1 s after durable persistence; no more than 10% latency regression on the fixed reference workload without an approved trade-off; incremental unchanged-input preparation materially faster than cold preparation. No end-to-end review-time promise is made before hardware/model measurements.

## 20. Evaluation design and release gates

### Datasets

- ReviewBench for broad review evaluation, with its public rubric and multi-source labels [S28].
- c-CRAB for review-to-fix usefulness, separating structural from behavioural tests [S29].
- SWE-PRBench for review precision/recall and context-strategy experiments [S30].
- A project-owned held-out corpus of real fixes, benign changes, plausible-but-correct hard negatives and security-boundary changes.
- An adversarial repository suite covering prompt injection, malicious build scripts, forged diagnostics, path escapes and cache poisoning.

Pin dataset versions and licences. Split by repository and time where feasible. Never expose gold fixes, review comments or held-out tests to discovery agents. Public benchmarks may be contaminated; publish results separately from private/fresh cases. Generated mutations supplement real bugs and do not replace them.

### Baselines and ablations

Static tools only; one local model with a fixed staged pipeline; multiple discovery roles without independent verification; full system with verification; alternative context budgets; different quantisations; and optional second-model challenger. Compare under equal hardware and both equal-token and equal-wall-time budgets. Otherwise a larger agent fleet can win merely by spending more.

Report per-language and per-severity precision/recall, confirmed-bug precision, false findings per clean change, candidate recall, verification retention, unique bugs per review, evidence replay rate, coverage gaps, p50/p95 latency, peak RAM/VRAM, tokens and cancellation reliability. Do not use raw comment count or subjective confidence as quality metrics.

Use at least three repeated model runs for stochastic comparisons. Report confidence intervals and paired/bootstrap comparisons by PR, not an independence assumption across highly related findings. Human adjudicators should be blind to system identity where feasible. Log disagreements and the rubric used to resolve them.

### Provisional release targets

| Area | Gate |
|---|---|
| Privacy | No unauthorised external communication observed across all certified offline-path tests; enforcement remains enabled under attack. |
| Host integrity | No original-checkout modification; no unauthorised host-file access in the adversarial suite. |
| Evidence | Every published reproduced finding has an independently generated valid evidence bundle and replay result. |
| Reporting precision | At least 90% adjudicated precision, with 95% confidence lower bound at least 85%, on at least 200 emitted confirmed findings. |
| Benign changes | At most 0.10 confirmed false findings per clean change on at least 100 clean/hard-negative changes; report the interval. |
| Added value | At least five percentage points recall improvement over the strongest matched-compute baseline while meeting precision gates, with uncertainty reported. |
| Protocol | Modern/legacy conformance and multi-client ownership tests pass for exact recorded client versions. |
| Reliability | Crash recovery, cancellation, quotas and stale-patch tests pass; no known high-severity unresolved product vulnerability. |
| Performance | Publish the measured hardware/model envelope. Meet control-plane targets on that reference machine. |

These are initial product acceptance targets, not results, and the recall target may require redesign rather than tuning a claim. A zero-observed-attack result in finite tests is not proof of universal security. Also measure benign task completion so the system cannot "win" by refusing everything. AutoDojo specifically motivates testing adaptive attacks rather than only a frozen attack list [S35].

## 21. User experience

The CLI shows scope, privacy status, current stage, active workers, consumed budget and coverage gaps. Findings are short by default: location, claim, trigger, impact and evidence label. Expandable detail includes reproducer, expected/observed result and limitations.

Use honest terminal language:

- `Completed: 3 reproduced bugs, 2 deterministic diagnostics, 1 unresolved question.`
- `Partial: verification unavailable for native iOS targets.`
- `No reproduced bugs found within the reviewed scope. 4 files were excluded.`

Never say "safe to merge" or "100% secure" based solely on an automated review.

Proposed CLI commands, to be implemented rather than copied as an existing product:

```sh
ultra doctor --offline
ultra repo register /absolute/path/to/project
ultra review --repo REPO_ID --base LOCAL_BASE --scope worktree --profile ultra
ultra status JOB_ID
ultra report JOB_ID --format markdown
ultra evidence replay JOB_ID FINDING_ID
ultra patch propose JOB_ID --finding FINDING_ID
ultra patch apply PROPOSAL_ID
ultra mcp --transport stdio
```

`patch apply` uses a user-controlled local authorisation mechanism outside model access, binds approval to exact file preconditions, refuses stale targets and never commits or pushes. A terminal prompt or `--yes` alone is not an isolation boundary. Platforms without the selected authorisation mechanism support proposal export only. Remote publication is deliberately absent.

## 22. Delivery sequence

**Milestone 0: contract and feasibility.** Lock **Rust-only (R26) language gates**, protocol support, one local model runtime and one execution backend. Demonstrate offline enforcement and safe snapshots before connecting a real model to tools.

**Milestone 1: minimal honest review.** Python/TypeScript **target repositories with Rust-built review components**, scope, static observations, one local discovery role, independent verifier, local report and replayable evidence.

**Milestone 2: Ultra workflow.** Bounded specialist fleet, challenger, deduplication, partial coverage, budgets and resume/cancel.

**Milestone 3: MCP product.** STDIO first, modern/legacy compatibility, durable tasks/fallback polling and tested local clients.

**Milestone 4: release qualification.** Benchmark harness, adversarial suite, platform certification, signed distribution and offline bundle preparation.

**Milestone 5: controlled expansion.** Additional model runtimes, language toolchains, native-platform verification, local UI and optional signed evidence. Add features only when measured value justifies maintenance and attack surface.

The implementation tasks and interfaces in `IMPLEMENTATION_PLAN.md` make these milestones executable. The first useful vertical slice is **snapshot -> local hypothesis -> independent reproducer -> local evidence report**, not eight agents with an unverified summary.

---

# PART II: IMPLEMENTATION PLAN

# Local Ultra Review MCP Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` or `superpowers:subagent-driven-development` when available. Otherwise follow the same task-by-task test-first workflow below. This is an implementation specification, not permission to skip repository discovery, security gates or user-controlled write approval.

**Goal:** Build a fully local multi-agent code reviewer that independently verifies findings and exposes the same engine through a CLI and MCP.

**Architecture:** **All first-party software is Rust**, not only the main service. An unprivileged Rust service owns policy, durable jobs and evidence. Isolated repository/execution workers and a managed local model runtime are reached only through typed brokers. MCP is a versioned interface adapter, not the orchestrator.

**Tech stack:** **Rust-only first-party Cargo workspace**, stable Rust, Tokio, official `rmcp`, SQLite, JSON Schema, Tree-sitter, a pinned third-party llama.cpp server, isolated local execution, **Rust-written evaluation, adversarial testing and packaging (`xtask`)**, pinned local third-party scanner bundles.

**Spec:** `DESIGN.md`. **Research:** `SOURCES.md`. **Research cut-off:** 22 October 2026.

## Global constraints

- R01-R26 in `DESIGN.md` are mandatory across all tasks.
- **R26 is a non-negotiable Rust-only first-party code constraint.** All app logic, interfaces, daemons, launchers, CLI/MCP, evaluators, benchmarking, security tests and packaging utilities are Rust. Reviewed-language fixtures and audited upstream tools are exceptions; `*.py`, `*.ts`, `*.js`, shell scripts etc. cannot implement first-party functionality.
- No cloud model, remote download, telemetry or remote tunnel during a strict review.
- No original-checkout writes during review; no arbitrary host shell exposed to agents.
- MCP 2026-07-28 and 2025-11-25 are separate, tested compatibility targets.
- One model adapter, two Tier 1 languages and one certified isolation backend precede expansion.
- Numbers in the design are proposed defaults/acceptance targets, not observed performance.
- Check exact runtime/SDK versions before implementation; pin them in manifests and lockfiles. Never copy a dated API example without checking the pinned dependency.
- Do not implement optional cloud export, automatic publishing, automatic merging or a hosted service.
- Stop a task at a failed security requirement. Report the blocker; do not silently remove the test.

## Review focus

1. Dirty repositories, concurrent edits and merge commits must not produce an incorrectly attributed snapshot. Covered by T06.
2. "Read-only" tools may load executable plugins/configuration or follow hostile paths. Covered by T05, T06 and T10.
3. A generated failing test can be wrong, environment-broken or unrelated to the claimed bug. Covered by T13 and T14.
4. Reconnection, cache reuse and multiple clients must not reveal another job's evidence or reuse stale authority. Covered by T04, T16, T18 and T19.
5. A local model endpoint, package manager or scanner may still contact the internet. Covered by T05, T07, T08 and T22.

## Workspace map

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
crates/
  ultra-core/
    src/{lib,types,errors,policy,config,budget}.rs
    tests/{contracts,policy,budgets}.rs
  ultra-store/
    src/{lib,db,cas,events,migrations}.rs
    tests/{persistence,recovery,ownership}.rs
  ultra-repo/
    src/{lib,snapshot,diff,paths,index,context,cache}.rs
    tests/{snapshots,paths,context,cache}.rs
  ultra-runtime/
    src/{lib,controller,scheduler,workers,model,sandbox,scanners,
         discovery,verification,adjudication,patches}.rs
    tests/{sandbox,model,scanners,jobs,discovery,verification,
           adjudication,fleet,patches}.rs
  ultra-report/
    src/{lib,findings,evidence,json,markdown,sarif,redaction}.rs
    tests/{eligibility,replay,exports,redaction}.rs
  ultra-mcp/
    src/{lib,tools,resources,stdio,http,compat,tasks,auth}.rs
    tests/{protocol_2026,protocol_2025,ownership,tasks,http}.rs
  ultra-cli/
    src/{main,daemon,doctor,prepare,commands,approval}.rs
    tests/{cli,approvals,vertical_slice}.rs
  ultra-eval/
    src/{lib,main,datasets,baselines,metrics,adjudication}.rs
    tests/{leakage,metrics,baselines}.rs
  ultra-adversarial/
    src/{lib,main,attacks,probes}.rs
    tests/{attacks,egress,host_integrity}.rs
  xtask/
    src/{main,audit_sources,package}.rs
    tests/source_policy.rs
schemas/                    generated contracts and pinned SARIF schema
prompts/v1/                 original reviewer/verifier/challenger templates
rules/                      trusted bundled rules and provenance manifests
profiles/                   offline and resource policies
fixtures/                   small synthetic repositories and attack fixtures
sandbox/                    image definitions and backend policy descriptions
models/                     manifests only; no large weights in Git
compat/                     exact client-version matrix and golden transcripts
eval/                       benchmark datasets, score/result artifacts and adjudication rubric (no executable scripts)
vendor/manifests/           offline tool/model/dependency bundle manifests
security/                   threat model, attack fixtures, coverage maps and test results (no executable scripts)
docs/                       user docs, ADRs, protocol and evidence formats
```

## Shared interface conventions

All public Rust domain functions use `ultra_core::Result<T>` with `ReviewError`. ID wrappers use UUIDv7 strings; IDs are not credentials. Hashes are SHA-256 over canonical, versioned representations. No schema accepts arbitrary command strings. Domain schema version starts at `1`.

T02 defines every shared type used below. Types with platform behaviour are opaque handles, not serialised operating-system objects. `CancellationToken` comes from the pinned Tokio utility library. Async backend traits use the same selected async-trait convention across the workspace; choose and record it in T01 rather than mixing conventions.

### Core type registry to define in T02

`RepoId`, `SnapshotId`, `PreparationId`, `JobId`, `FindingId`, `EvidenceId`, `ProposalId`, `CallerId`, `ArtifactId`, `Digest`, `RepoGrant`, `ReviewScope`, `ResolvedScope`, `SnapshotManifest`, `Snapshot`, `PreparedReview`, `ReviewRequest`, `ReviewJob`, `ReviewStatus`, `JobStage`, `CoverageGap`, `ResourceBudget`, `ResourceUsage`, `Policy`, `Action`, `ActionGrant`, `Decision`, `TrustLabel`, `HardwareProfile`, `BundleManifest`, `CapabilityReport`, `ContextQuery`, `ContextPacket`, `ModelRequest`, `ModelResponse`, `ModelCapabilities`, `RoleSpec`, `Hypothesis`, `SandboxRequest`, `ExecutionRecord`, `ScanPlan`, `Observation`, `VerificationRequest`, `Verification`, `Evidence`, `EvidenceClass`, `Finding`, `ReviewReport`, `ReportFormat`, `PatchProposal`, `PatchApproval`, `Event`, `Cursor`, `Page<T>`.

`Action` is a closed enum: `ReadSnapshot`, `ReadContext`, `RunPreset`, `GenerateReport`, `ProposePatch`, `ApplyPatch`. Network/installation are not review actions. `ApplyPatch` cannot be granted to any reviewer/model principal.

## Dependency map

```text
T01 -> T02 -> T03 -> T04
                 \-> T05 -> T06 -> T09
                       \-> T07 -> T08
T05 + T06 + T07 -> T10
T02 + T03 + T04 + T05 -> T11
T08 + T09 + T10 + T11 -> T12 -> T13 -> T14 -> T15
T09 + T11 + T12 + T13 + T15 -> T16
T07 + T11 + T15 -> T17 -> T18 -> T19
T13 + T15 + T17 -> T20
T15 + T16 + T17 -> T21
T03 + T05 + T08 + T18 + T20 -> T22
T17 + T18 + T22 -> T23
T19 + T20 + T21 + T22 + T23 -> T24
```

Parallel work is allowed only after shared interfaces are committed. Examples: report renderers can be developed against fixed report fixtures; language adapters can be developed against the established sandbox interface. Do not parallelise competing schema changes without a single owner.

## Common task execution cycle

Every task below has the following required steps:

- [ ] Create the named tests and fixtures first. Assert the specified behaviour, not merely that a function returns.
- [ ] Run the task's test command and confirm the intended failing assertion or missing implementation. Record the failure reason.
- [ ] Implement only the files/interfaces needed for the task.
- [ ] Re-run the named tests, then affected workspace tests. Review warnings and security regressions.
- [ ] Update the requirement trace/evidence record and commit the coherent change. Do not commit secrets, model weights or generated untrusted repositories.

A task is not done because its source compiles. Its listed acceptance behaviour must be demonstrated.

---

## T01. Establish workspace and dependency/protocol feasibility

**Files:** root Cargo files; `docs/adr/001-stack.md`; `vendor/manifests/toolchain.json`; `compat/client-matrix.json`; `crates/ultra-core/src/lib.rs`.

**Consumes:** `DESIGN.md`, official current SDK/runtime documentation. **Produces:** reproducible workspace, pinned dependencies and tested protocol capability record.

**Tests/steps:**
- [ ] Initialise the workspace only after checking whether the target directory already contains a project. Never replace an existing implementation blindly.
- [ ] Select the newest non-prerelease Rust/`rmcp` release combination that passes modern and legacy minimal tool round-trips. Record versions, source URLs, checksums and decision date.
- [ ] Build a minimal schema-validated fixture tool with both protocol versions. This is a conformance probe, not the review engine.
- [ ] Pin llama.cpp and one execution image/backend by immutable digest; record supported OS/architecture and licence metadata.
- [ ] Establish dependency audit, formatting, lint and test commands without automatic network updates during review execution.
- [ ] Create `crates/xtask/src/{main,audit_sources}.rs` and `crates/xtask/tests/source_policy.rs`. Implement `cargo run --locked -p xtask -- audit-sources` to reject first-party executable/script sources not in Rust. Maintain explicit allowlists for *data-only* target-language fixtures and inventoried third-party code; reject disguised executable helpers in owned code, including CI calls to non-Rust scripts. Add the source-policy gate to CI and release checks.

**Run:** `cargo check --workspace --locked`, `cargo test -p xtask --test source_policy`, `cargo run --locked -p xtask -- audit-sources`, and the minimal protocol fixture tests added under `compat/`.

**Gate:** first-party Rust-only source audit passes; both protocol targets work with the pinned SDK or a concrete adapter limitation is recorded and resolved before T18. No fabricated version support. **Commit:** `chore: pin local review toolchain and protocol probes`.

## T02. Define domain schemas and invariants

**Files:** `crates/ultra-core/src/{types,errors,config}.rs`; `crates/ultra-core/tests/contracts.rs`; `schemas/`.

**Consumes:** requirement IDs and type registry. **Produces:** all shared types, generated schemas, `validate_report(report: &ReviewReport) -> Result<()>` and `validate_request(request: &ReviewRequest) -> Result<()>`.

**Tests:** `confirmed_finding_requires_evidence`, `severity_is_not_certainty`, `unknown_command_field_is_rejected`, `source_span_must_reference_snapshot`, `invalid_enum_is_rejected`, `bounded_strings_and_arrays`.

**Assertions:** a reproduced finding with no verifier/evidence is rejected; an arbitrary shell field cannot deserialize; a source span outside the manifest is invalid; unverified findings cannot populate the reproduced-bugs list.

**Run:** `cargo test -p ultra-core --test contracts`.

**Gate:** generated schemas and Rust validation agree on positive and negative fixtures. **Commit:** `feat: define review contracts and evidence invariants`.

## T03. Enforce policy, caller identity and repository grants

**Files:** `crates/ultra-core/src/policy.rs`; `crates/ultra-core/tests/policy.rs`; `profiles/offline-strict.toml`; `security/threat-model.md`.

**Produces:** `authorize(policy: &Policy, caller: &CallerId, action: &Action, grant: &ActionGrant) -> Result<Decision>`; trusted configuration precedence and immutable policy digest.

**Tests:** `repository_text_cannot_grant_capabilities`, `model_cannot_apply_patch`, `cross_repo_grant_is_denied`, `expired_grant_is_denied`, `scope_cannot_expand_after_approval`, `unknown_profile_fails_closed`.

**Assertions:** changing `REVIEW.md` in a reviewed branch cannot enable network/write access; an authenticated client still cannot use another repository's grant; model identity can never mint approval.

**Run:** `cargo test -p ultra-core --test policy`.

**Gate:** all denied actions have structured, audited reasons. **Commit:** `feat: enforce caller-bound repository capabilities`.

## T04. Implement durable jobs, evidence blobs and ownership

**Files:** `crates/ultra-store/src/{db,cas,events,migrations}.rs`; `crates/ultra-store/tests/{persistence,recovery,ownership}.rs`.

**Produces:** `Store::create_job(request: ReviewRequest) -> Result<ReviewJob>`, `Store::append_event(job: JobId, event: Event) -> Result<()>`, `Store::put_artifact(bytes: &[u8], trust: TrustLabel) -> Result<ArtifactId>`, `Store::get_job(caller: &CallerId, id: JobId) -> Result<ReviewJob>`.

**Tests:** `acknowledged_job_survives_restart`, `idempotency_conflict_is_rejected`, `partial_blob_is_never_visible`, `terminal_state_is_immutable`, `cross_caller_read_is_denied`, `migration_preserves_evidence_links`, `store_permissions_are_owner_only`, `retention_removes_owned_cache_blobs`, `pinned_evidence_survives_retention`, `unencrypted_volume_status_is_not_misrepresented`.

**Assertions:** kill between metadata/blob steps; on restart the job is recoverable and orphan blobs do not become valid evidence. Repeated identical start returns the same job; changed preparation with the same key fails.

**Run:** `cargo test -p ultra-store`.

**Gate:** no success acknowledgement precedes durable persistence. **Commit:** `feat: persist owned jobs and content-addressed evidence`.

## T05. Implement one certified local execution boundary

**Files:** `crates/ultra-runtime/src/sandbox.rs`; `crates/ultra-runtime/tests/sandbox.rs`; `sandbox/`; `security/isolation-tests.md`.

**Produces:** `Sandbox::self_test() -> Result<CapabilityReport>` and `Sandbox::execute(request: SandboxRequest, cancel: CancellationToken) -> Result<ExecutionRecord>`.

**Tests:** `network_egress_is_denied`, `host_home_is_not_mounted`, `control_socket_is_unreachable`, `filesystem_escape_is_denied`, `fork_bomb_hits_pid_limit`, `output_flood_is_bounded`, `timeout_kills_descendants`, `sandbox_failure_never_runs_on_host`.

**Assertions:** synthetic canary files are inaccessible; the worker cannot reach external addresses, host loopback or controller/model sockets; cancelled descendants are absent; any unsupported isolation setting produces an error rather than a weaker backend.

**Run:** `cargo test -p ultra-runtime --test sandbox -- --test-threads=1` on the recorded certified backend.

**Gate:** tests execute in the actual backend, not only a mocked subprocess runner. Mark other platforms unsupported until their tests pass. **Commit:** `feat: isolate local verification workers`.

## T06. Build immutable Git and worktree snapshots

**Files:** `crates/ultra-repo/src/{snapshot,diff,paths}.rs`; `crates/ultra-repo/tests/{snapshots,paths}.rs`; synthetic repositories under `fixtures/repositories/`.

**Produces:** `Snapshotter::capture(grant: &RepoGrant, scope: &ReviewScope) -> Result<Snapshot>` and `Snapshotter::diff(snapshot: &Snapshot) -> Result<ContextPacket>`.

**Tests:** `staged_and_unstaged_are_not_double_counted`, `missing_base_never_fetches`, `merge_commit_requires_parent`, `concurrent_edit_cannot_mix_snapshots`, `rename_and_delete_preserve_identity`, `git_filter_and_hook_never_execute`, `symlink_escape_is_rejected`, `non_utf8_path_round_trips`, `case_collision_is_reported`, `linked_worktree_requires_authorised_common_dir`.

**Assertions:** snapshot digest changes on content/mode changes; source tree before/after hashes are equal; external Git helpers leave no canary marker; unstable capture retries at most once.

**Run:** `cargo test -p ultra-repo --test snapshots` and `cargo test -p ultra-repo --test paths`.

**Gate:** scope manifests explain every included and skipped path. **Commit:** `feat: freeze safe review snapshots and exact diff scope`.

## T07. Add offline bundles and capability diagnosis

**Files:** `crates/ultra-cli/src/{prepare,doctor}.rs`; `crates/ultra-core/src/config.rs`; `vendor/manifests/`; `profiles/`.

**Produces:** `validate_bundle(manifest: &BundleManifest) -> Result<CapabilityReport>` and `Doctor::inspect() -> Result<CapabilityReport>`.

**Tests:** `missing_weight_never_downloads`, `stale_advisory_db_is_reported`, `checksum_mismatch_blocks_use`, `unsupported_language_is_explicit`, `unknown_runtime_is_not_offline_certified`, `prepare_has_no_repository_source_access`.

**Assertions:** absent dependency is a coverage gap, not internet access; seven-day advisory freshness warning is deterministic under a fake clock; downloaded bundles have provenance and licence records before use.

**Run:** `cargo test -p ultra-cli --test cli doctor_` plus bundle validation fixtures.

**Gate:** `ultra doctor --offline` reports model, sandbox, compiler, package, scanner and egress readiness separately. **Commit:** `feat: validate offline review prerequisites`.

## T08. Integrate one managed local inference backend

**Files:** `crates/ultra-runtime/src/model.rs`; `crates/ultra-runtime/tests/model.rs`; `models/manifest.schema.json`; local model configuration.

**Produces:** `LocalModel::capabilities() -> ModelCapabilities` and `LocalModel::generate(request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse>`.

**Tests:** `remote_endpoint_is_rejected`, `redirect_is_rejected`, `proxy_env_is_not_inherited`, `runtime_cannot_auto_download`, `invalid_json_has_one_repair_attempt`, `tool_call_outside_schema_is_denied`, `cancel_releases_inference_slot`, `model_hash_change_invalidates_profile`.

**Assertions:** use a tiny deterministic fake backend for unit tests and a pinned real local model for contract tests; both paths record usage/version metadata. Never let a failed real-model test pass via a mocked fallback.

**Run:** `cargo test -p ultra-runtime --test model`; explicitly tagged offline-model contract test on prepared hardware.

**Gate:** actual runtime egress controls pass, not merely an endpoint string check. **Commit:** `feat: broker managed local model inference`.

## T09. Build source-grounded incremental context

**Files:** `crates/ultra-repo/src/{index,context,cache}.rs`; `crates/ultra-repo/tests/{context,cache}.rs`.

**Produces:** `ContextIndex::build(snapshot: &Snapshot) -> Result<()>` and `ContextIndex::query(query: ContextQuery) -> Result<ContextPacket>`.

**Tests:** `context_citations_have_real_spans`, `changed_hunks_survive_truncation`, `dependency_edit_invalidates_caller_context`, `parser_timeout_is_coverage_gap`, `cross_repo_cache_is_denied`, `untrusted_labels_survive_cache`.

**Assertions:** initial packets respect the 12,000-token limit and report omissions; compiler-derived relationships are labelled separately from syntactic guesses; hostile parser inputs run in a bounded worker.

**Run:** `cargo test -p ultra-repo --test context` and `cargo test -p ultra-repo --test cache`.

**Gate:** targeted context works without embeddings. **Commit:** `feat: retrieve bounded source-grounded review context`.

## T10. Integrate deterministic scanners and Tier 1 language presets

**Files:** `crates/ultra-runtime/src/scanners.rs`; `crates/ultra-runtime/tests/scanners.rs`; `rules/`; `sandbox/presets/`.

**Produces:** `ScannerBroker::run(plan: ScanPlan, snapshot: &Snapshot) -> Result<Vec<Observation>>`.

**Tests:** `semgrep_uses_only_local_rules`, `secret_values_are_redacted`, `scanner_exit_code_is_not_verdict`, `eslint_plugin_is_not_loaded_on_host`, `pytest_conftest_runs_only_in_worker`, `base_head_diagnostics_are_distinguished`, `codeql_requires_entitlement`, `missing_dependency_is_not_bug`.

**Assertions:** Semgrep metrics are explicitly off; every scanner output is parsed under bounds; no executable is selected from the reviewed repository's PATH; detected secrets never enter model context as values.

**Run:** `cargo test -p ultra-runtime --test scanners` and prepared Python/TypeScript worker fixtures.

**Gate:** deterministic diagnostics remain observations pending adjudication. **Commit:** `feat: add offline Python and TypeScript analysis presets`.

## T11. Implement bounded durable job orchestration

**Files:** `crates/ultra-runtime/src/{controller,scheduler,workers}.rs`; `crates/ultra-runtime/tests/jobs.rs`; `crates/ultra-core/src/budget.rs`.

**Produces:** `Controller::prepare(repo: RepoId, scope: ReviewScope, caller: CallerId) -> Result<PreparedReview>`, `Controller::start(request: ReviewRequest) -> Result<JobId>`, `Controller::status(caller: &CallerId, job: JobId) -> Result<ReviewStatus>`, `Controller::cancel(caller: &CallerId, job: JobId) -> Result<ReviewStatus>`.

**Tests:** `budgets_are_reserved_before_work`, `crash_reclaims_expired_worker_lease`, `reconnect_does_not_duplicate_job`, `partial_stage_failure_remains_visible`, `cancel_kills_workers_and_model_request`, `resume_preserves_policy_and_snapshot`.

**Assertions:** default one inference/two verification-worker limits hold; cancellation races are deterministic; terminal statuses cannot be rewritten by late worker results.

**Run:** `cargo test -p ultra-runtime --test jobs` and `cargo test -p ultra-core --test budgets`.

**Gate:** long-running work does not depend on a particular MCP connection staying open. **Commit:** `feat: orchestrate durable resource-bounded review jobs`.

## T12. Implement the first discovery role and bounded internal tools

**Files:** `crates/ultra-runtime/src/discovery.rs`; `crates/ultra-runtime/tests/discovery.rs`; `prompts/v1/correctness.md`; `prompts/v1/security.md`.

**Produces:** `Discovery::run(role: RoleSpec, snapshot: &Snapshot, context: ContextPacket) -> Result<Vec<Hypothesis>>`.

**Tests:** `hypothesis_requires_invariant_and_trigger`, `invented_source_reference_is_rejected`, `repository_instruction_cannot_override_role`, `unbounded_context_request_is_denied`, `style_only_claim_does_not_enter_bug_queue`.

**Assertions:** hypotheses cite retrieved artefacts; model actions are limited to typed context requests; execution claims are not accepted from prose. Start with correctness, then security using the same interface.

**Run:** `cargo test -p ultra-runtime --test discovery`.

**Gate:** the single-role baseline is preserved for evaluation. **Commit:** `feat: discover falsifiable source-grounded bug hypotheses`.

## T13. Build independent verification and regression evidence

**Files:** `crates/ultra-runtime/src/verification.rs`; `crates/ultra-runtime/tests/verification.rs`; `prompts/v1/verifier.md`; regression fixtures.

**Produces:** `Verifier::verify(request: VerificationRequest, cancel: CancellationToken) -> Result<Verification>`.

**Tests:** `verifier_has_fresh_context`, `environment_failure_cannot_confirm_bug`, `tautological_test_is_rejected`, `base_pass_head_fail_is_recorded`, `new_api_without_base_needs_contract`, `preexisting_bug_is_not_new`, `fake_success_log_cannot_forge_supervisor_result`, `attempt_budget_stops_after_two_repairs`, `flaky_result_is_inconclusive`.

**Assertions:** outcome depends on supervisor evidence and a validated oracle; reviewer self-confidence is ignored; test source and execution records are hashed; no generated test executes outside a worker.

**Run:** `cargo test -p ultra-runtime --test verification` and the real worker differential-test fixtures.

**Gate:** one end-to-end fixture produces a valid reproduced finding and one plausible hard negative is disproved. **Commit:** `feat: independently reproduce and reject review hypotheses`.

## T14. Implement challenger and evidence eligibility

**Files:** `crates/ultra-runtime/src/adjudication.rs`; `crates/ultra-runtime/tests/adjudication.rs`; `prompts/v1/challenger.md`; `crates/ultra-report/src/findings.rs`.

**Produces:** `Adjudicator::decide(hypothesis: &Hypothesis, verification: &Verification, observations: &[Observation]) -> Result<Option<Finding>>`.

**Tests:** `reachable_trigger_is_required_for_impact`, `existing_validation_disproves_claim`, `high_severity_uncertainty_is_visible`, `diagnostic_is_not_runtime_reproduction`, `majority_vote_cannot_override_missing_evidence`, `challenge_cannot_change_permissions`.

**Assertions:** a majority of agreeing reviewers cannot publish an unsupported claim; unresolved serious concerns remain explicitly unverified; intended behaviour is not redefined by generated tests.

**Run:** `cargo test -p ultra-runtime --test adjudication` and `cargo test -p ultra-report --test eligibility`.

**Gate:** only the approved evidence classes enter their corresponding report sections. **Commit:** `feat: adjudicate findings through evidence gates`.

## T15. Build reports, deduplication and evidence replay

**Files:** `crates/ultra-report/src/{findings,evidence,json,markdown,sarif,redaction}.rs`; `crates/ultra-report/tests/{replay,exports,redaction}.rs`.

**Produces:** `ReportBuilder::build(job: &ReviewJob, findings: Vec<Finding>) -> Result<ReviewReport>`, `EvidenceReplayer::replay(evidence: &Evidence) -> Result<ExecutionRecord>` and `export_report(report: &ReviewReport, format: ReportFormat) -> Result<ArtifactId>`; use the `ReportFormat` contract defined in T02.

**Tests:** `same_root_cause_groups_without_losing_provenance`, `different_triggers_are_not_collapsed`, `report_never_claims_secure`, `sarif_validates`, `logs_cannot_inject_terminal_or_html`, `evidence_replay_detects_tampering`, `partial_report_preserves_gaps`.

**Assertions:** evidence remains linkable after deduplication; source spans map to reviewed bytes; all exports have consistent counts and labels; replay refuses mismatched toolchain/model-independent execution preconditions.

**Run:** `cargo test -p ultra-report`.

**Gate:** JSON, Markdown and SARIF fixtures agree on severity, location and verification status. **Commit:** `feat: export honest review reports and replayable evidence`.

## T16. Add the bounded specialist fleet and cache optimisation

**Files:** `crates/ultra-runtime/src/{scheduler,discovery}.rs`; `crates/ultra-runtime/tests/fleet.rs`; remaining `prompts/v1/` roles; cache tests.

**Produces:** `FleetPlanner::plan(snapshot: &Snapshot, capabilities: &CapabilityReport, budget: &ResourceBudget) -> Result<Vec<RoleSpec>>`.

**Tests:** `conditional_roles_only_activate_when_relevant`, `reviewers_do_not_see_initial_peer_findings`, `one_model_runtime_serves_multiple_roles`, `memory_pressure_reduces_admission_not_policy`, `candidate_cap_sets_partial_coverage`, `policy_or_model_change_invalidates_cache`.

**Assertions:** at most six discovery roles and 24 expensive candidates under the default profile; role scheduling cannot bypass verifier independence; completed deterministic work is safely reused only under identical keys.

**Run:** `cargo test -p ultra-runtime --test fleet` and `cargo test -p ultra-repo --test cache`.

**Gate:** compare the fleet to the preserved single-agent baseline; do not call it better before T21. **Commit:** `feat: schedule bounded multi-role local reviews`.

## T17. Ship the local CLI and per-user daemon vertical slice

**Files:** `crates/ultra-cli/src/{main,daemon,commands,doctor}.rs`; `crates/ultra-cli/tests/{cli,vertical_slice}.rs`; `docs/cli.md`.

**Produces:** proposed commands from `DESIGN.md`; authenticated local IPC to the existing controller. No business logic duplicated in CLI handlers.

**Tests:** `two_clients_share_one_daemon`, `review_does_not_change_checkout`, `offline_vertical_slice_reproduces_bug`, `client_disconnect_does_not_lose_job`, `cancel_has_visible_terminal_result`, `missing_prerequisite_is_actionable`.

**Assertions:** on prepared hardware, review one Python and one TypeScript synthetic repository offline; one true bug and one hard negative produce the expected evidence classes; cold and warm measurements are recorded, not invented.

**Run:** `cargo test -p ultra-cli --test vertical_slice` plus an explicitly recorded real-model offline smoke run.

**Gate:** a user can complete a review without an MCP client or internet connection. **Commit:** `feat: expose the local review engine through CLI`.

## T18. Add modern and legacy STDIO MCP adapters

**Files:** `crates/ultra-mcp/src/{tools,resources,stdio,compat,auth}.rs`; `crates/ultra-mcp/tests/{protocol_2026,protocol_2025,ownership}.rs`; `compat/`.

**Produces:** all nine public tools and authorised resources; adapter methods delegate to controller/report APIs. Use the pinned SDK's exact supported signatures.

**Tests:** `server_discover_reports_supported_versions`, `modern_request_metadata_round_trip`, `modern_results_include_result_type`, `private_resources_never_use_public_cache_scope`, `cacheable_results_have_ttl`, `discovery_order_is_connection_independent`, `legacy_initialisation_round_trip`, `stdout_contains_only_protocol_messages`, `tool_annotations_match_side_effects`, `resource_read_rechecks_ownership`, `malformed_tool_input_is_execution_error`, `evidence_pagination_respects_64k_limit`, `findings_page_defaults_to_20`.

**Assertions:** do not require deprecated Roots/Sampling from new clients; reject unknown repository IDs and arbitrary paths; state persists through explicit job handles rather than a protocol session.

**Run:** `cargo test -p ultra-mcp --test protocol_2026`, `cargo test -p ultra-mcp --test protocol_2025`, and ownership tests.

**Gate:** recorded real client versions pass against non-sensitive fixtures. **Commit:** `feat: expose versioned local review MCP tools`.

## T19. Add long-running task adaptation and optional loopback HTTP

**Files:** `crates/ultra-mcp/src/{tasks,http}.rs`; `crates/ultra-mcp/tests/{tasks,http}.rs`; compatibility docs.

**Produces:** negotiated Tasks extension adapter, normal-tool polling fallback, authenticated loopback HTTP server.

**Tests:** `task_handle_is_durable_before_response`, `no_extension_uses_normal_job_handle`, `cancel_reaches_underlying_workers`, `modern_http_has_no_session_dependency`, `legacy_client_uses_legacy_path`, `foreign_origin_is_denied`, `missing_token_is_denied`, `loopback_is_not_wildcard_bind`, `job_id_alone_is_not_authority`.

**Assertions:** extension support is negotiated, not assumed; client disconnect after durable acknowledgement does not terminate a review unless job/task cancellation is requested; protocol cancellation acknowledgment does not falsely imply worker teardown completion. Ordinary HTTP request cancellation is handled separately. Validate version-specific metadata headers; do not advertise unimplemented subscriptions. Add `lost_start_response_retries_idempotently`, `http_metadata_headers_are_checked`, and `unsupported_subscriptions_not_advertised` tests.

**Run:** `cargo test -p ultra-mcp --test tasks` and `cargo test -p ultra-mcp --test http`.

**Gate:** no remote bind, tunnel or public endpoint in the offline profile. **Commit:** `feat: support durable MCP review jobs and secured loopback transport`.

## T20. Add tested patch proposals and separate application authority

**Files:** `crates/ultra-runtime/src/patches.rs`; `crates/ultra-runtime/tests/patches.rs`; `crates/ultra-cli/src/approval.rs`; `crates/ultra-cli/tests/approvals.rs`.

**Produces:** `PatchService::propose(job: JobId, findings: &[FindingId]) -> Result<PatchProposal>` and `PatchService::apply(proposal: &PatchProposal, approval: &PatchApproval) -> Result<()>`.

**Tests:** `proposal_only_writes_scratch`, `patch_reproducer_passes_without_weakening_assertions`, `stale_target_is_rejected`, `approval_is_bound_to_patch_hash`, `model_boolean_cannot_approve`, `expired_approval_is_denied`, `partial_apply_rolls_back`, `patch_never_commits_or_pushes`.

**Assertions:** use a local user-controlled authority outside model-worker access. A TTY, `--yes` flag or caller-provided boolean is not sufficient security evidence. If the platform lacks the selected human-authorisation mechanism, support proposal export only and refuse application.

**Run:** `cargo test -p ultra-runtime --test patches` and `cargo test -p ultra-cli --test approvals`.

**Gate:** apply requires per-file snapshot preconditions and atomic/rollback behaviour. **Commit:** `feat: propose verified patches with separate application approval`.

## T21. Implement benchmark and ablation harness

**Files:** `crates/ultra-eval/src/{lib,main,datasets,metrics,baselines,adjudication}.rs`; `crates/ultra-eval/tests/{leakage,metrics,baselines}.rs`; `eval/{adjudication.md,datasets.lock.json}`; `eval/results/`; benchmark-result schema. **No first-party Python or other-language benchmark code.**

**Produces:** reproducible dataset preparation, hardware/model manifest, baseline runs and scored reports. The evaluator is a separate **Rust binary/crate** within the same Cargo workspace and uses engine data contracts without granting extra review permissions.

**Tests:** `gold_labels_are_not_in_agent_context`, `split_is_repository_disjoint`, `duplicate_prs_are_not_double_counted`, `timeouts_count_as_failures_or_gaps`, `precision_denominator_is_emitted_findings`, `new_valid_findings_are_not_auto_false_positives`, `confidence_intervals_cluster_by_pr`.

**Assertions:** distinguish c-CRAB behavioural/structural checks; record model-judge and human disagreements; at least three repeated runs for stochastic ablations; equal-compute comparisons are explicit.

**Run:** `cargo test -p ultra-eval --locked`, followed by `cargo run --locked -p ultra-eval -- --dataset eval/datasets.lock.json --profile frozen-baselines` on prepared benchmark fixtures.

**Gate:** Rust evaluation harness and tests pass; produce actual results for static-only, single-agent and full system; meet the design's thresholds or label the release experimental. **Commit:** `test: evaluate review quality and compute trade-offs`.

## T22. Build adaptive adversarial and privacy regression suites

**Files:** `crates/ultra-adversarial/src/{lib,main,attacks,probes}.rs`; `crates/ultra-adversarial/tests/{attacks,egress,host_integrity}.rs`; `security/{attacks,coverage-map.md}`. Attack payloads under `security/attacks/` are untrusted fixture data, not first-party executable harness code.

**Produces:** replayable adversarial fixtures and externally observed security outcomes.

**Test families:** instructions in comments/Markdown; forged `REVIEW.md`; malicious package/install hooks; output-log instructions; external URL/data exfiltration; ANSI/OSC injection; symlink/zip traversal; `.git` alternates; DNS/redirect/proxy tricks; cross-job resource reads; poisoned context cache; fake test verdicts; race-to-apply patches; fork/output/disk bombs; worker cancellation races; malicious scanner config; hostile model output.

**Assertions:** observe network/host canaries outside the worker; distinguish blocked attack, successful benign review and blanket refusal. Include adaptive mutations that were not used while implementing policy. No production targets or real credentials are needed.

**Run:** `cargo test -p ultra-adversarial --locked` plus `cargo run --locked -p ultra-adversarial -- --suite security/attacks --offline --isolated` on a prepared certified backend.

**Gate:** Rust adversarial test harness passes with no known high-severity product escape/exfiltration/write bug. Publish test scope and limitations; zero observed successes is not universal immunity. **Commit:** `test: harden local review trust boundaries`.

## T23. Package and certify supported platforms

**Files:** `crates/xtask/src/{main,package,audit_sources}.rs`; `crates/xtask/tests/source_policy.rs`; `docs/{install,offline-bundles,platform-support,troubleshooting}.md`; platform policy profiles; software bill of materials; release manifests. All owned packaging logic must be Rust (`xtask`), not shell/Python/Node scripts.

**Produces:** signed or checksum-verifiable per-platform packages, clean-machine installation procedure and exact capability matrix.

**Tests:** `clean_machine_offline_install`, `tampered_package_is_rejected`, `upgrade_preserves_jobs`, `rollback_preserves_schema_compatibility`, `uninstall_documents_retained_data`, `unsupported_native_target_is_not_verified`, `no_automatic_updater_traffic`.

**Assertions:** run only supported platform tests as passing; others remain explicitly unqualified. Record CPU/GPU/RAM, OS, worker environment, model and runtime digests. Inventory licences for code, rules, models and datasets separately.

**Run:** `cargo run --locked -p xtask -- audit-sources`, `cargo run --locked -p xtask -- package --offline`, plus documented package verification and isolated clean-machine smoke tests per platform.

**Gate:** Rust `xtask` packaging and source audit pass; do not advertise macOS/Windows native execution parity merely because the CLI starts there. **Commit:** `build: package certified offline review environments`.

## T24. Run final release qualification and publish evidence

**Files:** `docs/release/qualification.md`; `eval/results/`; `security/results/`; final traceability matrix and compatibility manifest.

**Produces:** a decision: release, experimental release with explicit limits, or blocked release.

**Verification commands:**

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p ultra-eval --locked
cargo test -p ultra-adversarial --locked
cargo run --locked -p xtask -- audit-sources
```

These commands will be run against the future implementation. They have not been run by this planning task.

- [ ] Confirm **R26 Rust-only source audit** passes across all first-party executable code, owned test code, evaluation, security and release tooling; document any fixture/vendor exceptions.
- [ ] Run offline end-to-end reviews of both Tier 1 language fixtures with the pinned real model.
- [ ] Run all certified isolation/egress tests and multi-client protocol tests.
- [ ] Replay every confirmed finding in the release fixture corpus.
- [ ] Validate all report schemas, links, counts, evidence classes and coverage gaps.
- [ ] Run the held-out benchmark and compare to both required baselines.
- [ ] Inspect dependencies, model/rule provenance, secret handling, licence gates and distribution signatures.
- [ ] Review any failed/flaky/skipped tests. Do not rewrite their labels to make a green release.
- [ ] Record known limits, residual risks, unsupported environments and actual measured resource use.

**Gate:** all mandatory design requirements have evidence. A failed gate blocks the corresponding claim, not just the test. **Commit:** `release: record evidence-backed local ultra review qualification`.

## Requirement-to-task traceability

| Requirements | Owning tasks |
|---|---|
| R01, R02, R24 | T03, T05, T07, T08, T22, T23 |
| R03, R04 | T02, T06, T17, T20 |
| R05, R13, R14 | T03, T18, T19, T20, T22 |
| R06, R07, R08, R25 | T12, T13, T14, T15 |
| R09, R16, R20, R22 | T07, T10, T11, T15, T17 |
| R10, R11, R12 | T04, T05, T08, T11, T19 |
| R15 | T09, T16, T22 |
| R17 | T01, T18, T19, T24 |
| R18 | T20 |
| R19 | T02, T15, T18 |
| R21 | T01, T07, T10, T21, T23 |
| R23 | T21, T24 |
| R26 | T01, T21, T22, T23, T24 (and every implementation task) |

## Definition of done for the first useful milestone

A clean local installation can capture a dirty repository without changing it, run one local discovery role, independently reproduce a seeded real bug in an isolated worker, reject a plausible hard negative, and export an evidence-backed report while network egress is denied. This must work through the CLI before the fleet, dashboard or additional runtime adapters are allowed to expand scope.

## Agent hand-off rules

Do not execute the entire roadmap in one uncontrolled change. Read the design, map the actual workspace, state which task is next, and implement the smallest dependency-complete slice. At each completion, report files changed, tests executed, outcomes, unresolved risks and next task. Record deviations as ADRs. Do not fabricate benchmark scores or claim Claude equivalence from architectural similarity.

---

# PART III: AGENT HAND-OFF

# Implementation-agent hand-off

**Project:** Local Ultra Review MCP  
**Design date:** 22 October 2026  
**Rust-only revision:** 23 October 2026  
**Status:** Planning package, not an implemented product.

Copy the following brief into the coding-agent session that has access to the target workspace.

---

## Mission

Implement a fully local, evidence-backed code-review engine, with **every first-party component written in Rust**, exposed through a Rust CLI and Rust MCP server. Preserve the publicly documented Ultra Review pattern of multiple reviewers and independent verification. Do not claim to clone Anthropic's unpublished implementation or match its accuracy before comparative evaluation.

Read `DESIGN.md`, `IMPLEMENTATION_PLAN.md` and `SOURCES.md` in full before making product changes. Treat R01-R26 as project-wide requirements, with R26 the Rust-only first-party language gate. The implementation plan contains the task order, file boundaries, shared interfaces, acceptance tests and release gates. Resolve any contradiction before implementing the affected interface.

## First action

Inspect the actual workspace, existing instructions, version control state and installed toolchain without changing them. Do not assume that a new repository is needed. Identify the operating system, CPU architecture, RAM and accelerator using read-only discovery where permitted. Do not assume hardware from old conversations.

Map the workspace to T01. Establish the Rust-only source-policy test before implementing evaluation or security runners. Record which official dependency versions are being pinned and why they satisfy the required protocol and runtime capabilities. Online dependency research/preparation is a separate, expressly authorised activity, never a hidden part of a strict review. Do not transmit repository source while resolving dependencies.

## Mandatory Rust-only implementation language (R26)

**Do not build a Python/TypeScript/Go application with a Rust MCP wrapper.** The entire first-party implementation must use Rust: controller, domain types, scheduling, memory/context retrieval, local model adapters, verification, sandbox brokering, report generation, CLI, MCP, benchmarks, security harnesses, packaging helpers and automated product tests. Use a Cargo workspace; `xtask` provides source-language policy and release packaging. **Never add first-party `*.py`, `*.ts`, `*.js`, shell/PowerShell scripts or another-language executable/test code.** Run `cargo run --locked -p xtask -- audit-sources` as a non-optional build/CI/release gate.

Python and TypeScript/JavaScript in the specification are *languages of code being reviewed*, not implementation languages. Target-language example projects and tests are untrusted fixtures; pinned third-party scanners, model engines and compilers may use their upstream language but are invoked through audited Rust adapters. Markdown, JSON, YAML and similar declarative data formats do not violate Rust-only. Do not broaden these exceptions to conceal first-party helpers.

## Non-negotiable boundaries

- Strict mode uses local inference, local storage and isolated local execution. No cloud fallback, remote search, telemetry, tunnel or automatic download during a review.
- The original checkout is read-only during review. Freeze the exact base/head/index/worktree scope and bind every finding to that snapshot.
- Repository files, build configuration, tool output and model-generated tests are untrusted data. They cannot grant capabilities, alter policy or approve writes.
- Models receive typed tools, not an unrestricted host shell. A missing sandbox is a blocker, not permission to run tests on the host.
- A discovery model cannot mark its own claim verified. Only independently collected evidence with a valid behavioural oracle can justify `runtime_reproduced`.
- Keep diagnostics, static proofs, reproduced bugs, unresolved concerns and coverage gaps distinct. A failed import, missing dependency or timeout is not a bug reproducer.
- A connected cloud-model client is outside the end-to-end offline guarantee even when its MCP server is local. The local CLI must work independently.
- Patches are proposals until an authenticated local human authority approves the exact proposal and file preconditions. No MCP `approve` tool, automatic commit, push, publication or merge.
- Claim compatibility only for tested protocol/client versions. Modern MCP and the legacy adapter do not share session assumptions.
- Security tests, benchmark results and performance numbers must be measured, not invented. A finite attack suite does not prove universal security.

## Execution discipline

Work task-by-task through the dependency graph. Each task begins with its named failing tests. Implement the smallest coherent change, run the specified tests plus affected regressions, inspect failures, and commit only the verified scope when repository policy permits commits.

Use the named shared types instead of inventing competing schemas. Document justified changes in an architectural decision record and update downstream interfaces and tests together. Do not quietly weaken a requirement to satisfy a test.

The first useful vertical slice is: safe immutable snapshot, one local discovery role, independent sandboxed reproduction, one rejected hard negative, and an evidence-backed local report. Prove this with real local inference and denied network egress before expanding the agent fleet or adding UI work.

At each task boundary report: task ID, changed files, exact verification commands, actual outcomes, outstanding risks and the next dependency-ready task. On failure, retain the evidence and identify the specific blocker. Never mark skipped platform tests as passing.

## Completion standard

Use the implementation plan's release qualification gates, including the mandatory Rust-only source audit. Deliver code, tests, offline preparation manifests, compatibility records, benchmark methodology/results and documented limits. Architectural similarity alone is not evidence of Claude-equivalent performance.

---

This brief authorises no remote publication, external account modification or cloud inference. Product implementation begins in the target workspace, not in this planning package.

---

# PART IV: SOURCE REFERENCES

# Research sources and evidence boundaries

**Accessed:** 22 October 2026, Australia/Sydney.  
**Method:** Targeted primary-source research covering the public Claude workflow, current MCP specification, local runtimes, isolation, static analysis and review/security evaluation. Selected academic abstracts and relevant HTML methods were examined. This is not an exhaustive systematic literature review, and the papers' experiments were not reproduced.

Live documentation entries use 2026 as the **documentation snapshot year**, not an assertion that every page was originally published in 2026. Dated manuscripts, benchmark announcements and the SARIF standard use their publication/submission year. References follow an Australian Harvard-style author/year pattern, with stable source IDs for implementation traceability.

## Evidence hierarchy

Official specifications determine protocol behaviour. Official product documentation determines documented product behaviour, not hidden internals. Original model/runtime documentation determines supported interfaces, not measured performance on Raouf's hardware. Research findings are bounded by the authors' datasets, methodology and version. Proposed product defaults and release gates are engineering decisions, not claims already demonstrated by these sources.

## Important distinctions

- The managed Code Review service, local `/code-review` and hosted Ultra Review must not be conflated.
- Workflow similarity does not establish accuracy parity with Claude, nor access to its proprietary internals.
- An MCP server on a laptop is not automatically an offline AI system.
- A test that examines source text is not the same as a runtime bug reproducer.
- A signature authenticates an artefact or signer; it does not prove that software has no bugs.
- A finite security test suite cannot establish universal immunity, particularly against adaptive attacks.

## Source catalogue

### S01

Anthropic (2026a), *Find bugs with ultrareview*, official product documentation, viewed 22 October 2026.

Available at: <https://code.claude.com/docs/en/ultrareview>.

**Use and limitation:** Public workflow baseline: cloud reviewer fleet and independent reproduction/verification. Does not disclose a complete reproducible internal design.

### S02

Anthropic (2026b), *Review locally with /code-review*, official product documentation, viewed 22 October 2026.

Available at: <https://code.claude.com/docs/en/code-review-local>.

**Use and limitation:** Distinguishes the local-session review command from hosted Ultra Review. Local execution wording must not be mistaken for local model inference.

### S03

Anthropic (2026c), *Code Review*, official product documentation, viewed 22 October 2026.

Available at: <https://code.claude.com/docs/en/code-review>.

**Use and limitation:** Managed GitHub review product. Do not transfer its detailed behaviour or limits to Ultra Review without separate evidence.

### S04

Model Context Protocol contributors (2026a), *Specification, revision 2026-07-28*, official protocol specification, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/specification/2026-07-28>.

**Use and limitation:** Source of truth for the current protocol target and application-level security responsibilities.

### S05

Model Context Protocol contributors (2026b), *Key changes, revision 2026-07-28*, official protocol specification, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/specification/2026-07-28/changelog>.

**Use and limitation:** Stateless/request metadata changes, removed protocol sessions and deprecated Roots/Sampling/Logging. Drives the adapter design.

### S06

Model Context Protocol contributors (2026c), *Tasks*, official extension documentation, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/extensions/tasks/overview>.

**Use and limitation:** Durable task handles, capability negotiation, polling and cancellation. Extension behaviour is not identical to the older 2025 experimental task API.

### S07

Model Context Protocol contributors (2026d), *Tools*, official protocol specification, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/specification/2026-07-28/server/tools>.

**Use and limitation:** Tool schemas, result structure and tool safety. Annotations do not replace server-side authority checks.

### S08

Model Context Protocol contributors (2026e), *stdio transport*, official protocol specification, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio>.

**Use and limitation:** STDIO framing and separation of protocol output from diagnostics.

### S09

Model Context Protocol contributors (2026f), *Streamable HTTP transport*, official protocol specification, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http>.

**Use and limitation:** Current HTTP transport and backwards-compatibility differences; supports a separately tested modern/legacy adapter.

### S10

Model Context Protocol contributors (2026g), *Security best practices*, official security guidance, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/security_best_practices>.

**Use and limitation:** MCP-specific attack surfaces, authorisation and safe implementation practices. Not a security certification for this proposed product.

### S11

Model Context Protocol Rust SDK contributors (2026), *Official Rust SDK for the Model Context Protocol*, primary project documentation, viewed 22 October 2026.

Available at: <https://github.com/modelcontextprotocol/rust-sdk>.

**Use and limitation:** SDK protocol support and adapter implementation reference. Pin a released version and test it; repository HEAD is not a dependency lock.

### S12

Ollama (2026), *FAQ*, official runtime documentation, viewed 22 October 2026.

Available at: <https://docs.ollama.com/faq>.

**Use and limitation:** Documents cloud-disable configuration and runtime behaviour. Cloud-disable is only one part of offline enforcement.

### S13

ggml-org contributors (2026), *llama.cpp server README*, primary runtime documentation, viewed 22 October 2026.

Available at: <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md>.

**Use and limitation:** Local serving, schema-constrained output and batching/concurrency capabilities. Exact support depends on pinned build and model.

### S14

MLX-LM contributors (2026), *MLX-LM: Run LLMs with MLX*, primary runtime documentation, viewed 22 October 2026.

Available at: <https://github.com/ml-explore/mlx-lm>.

**Use and limitation:** Candidate Apple-silicon backend and prompt-cache functionality. No local performance measurements were made in this research.

### S15

vLLM contributors (2026), *Structured outputs*, official runtime documentation, viewed 22 October 2026.

Available at: <https://docs.vllm.ai/en/latest/features/structured_outputs/>.

**Use and limitation:** Current structured-output interface and deprecated-field warning. Avoid copying obsolete guided-output examples.

### S16

Qwen team (2026), *Qwen3-Coder-Next model card*, original model card, viewed 22 October 2026.

Available at: <https://huggingface.co/Qwen/Qwen3-Coder-Next>.

**Use and limitation:** Candidate local weights and distinction between total and activated parameters. Vendor benchmark claims do not establish review quality on this system.

### S17

Tree-sitter contributors (2026), *Tree-sitter: Introduction*, official parser documentation, viewed 22 October 2026.

Available at: <https://tree-sitter.github.io/tree-sitter/>.

**Use and limitation:** Incremental syntactic parsing. Not evidence of complete semantic or cross-file data-flow analysis.

### S18

Git project (2026), *git-diff documentation*, official git documentation, viewed 22 October 2026.

Available at: <https://git-scm.com/docs/git-diff>.

**Use and limitation:** Diff scope semantics and external-helper controls. Snapshot safety still requires additional application design and testing.

### S19

Semgrep (2026a), *Semgrep metrics*, official scanner documentation, viewed 22 October 2026.

Available at: <https://docs.semgrep.dev/metrics-1>.

**Use and limitation:** Metrics behaviour differs between local configuration and registry use; strict mode explicitly disables telemetry.

### S20

Semgrep (2026b), *CLI reference*, official scanner documentation, viewed 22 October 2026.

Available at: <https://docs.semgrep.dev/cli-reference>.

**Use and limitation:** Scanner invocation/output/exit-code reference. Verify against the exact installed release.

### S21

Google OSV-Scanner contributors (2026), *Offline mode*, official scanner documentation, viewed 22 October 2026.

Available at: <https://google.github.io/osv-scanner/usage/offline-mode/>.

**Use and limitation:** Offline vulnerability matching requires a preloaded database. Freshness and missing-data gaps must remain visible.

### S22

GitHub (2026a), *CodeQL CLI*, official product and licence-eligibility documentation, viewed 22 October 2026.

Available at: <https://docs.github.com/en/code-security/concepts/code-scanning/codeql/codeql-cli>.

**Use and limitation:** CodeQL is not an unconditional free dependency for every private-repository use case. Check applicable entitlement and terms at installation.

### S23

gVisor contributors (2026a), *Security model*, official sandbox architecture documentation, viewed 22 October 2026.

Available at: <https://gvisor.dev/docs/architecture_guide/security/>.

**Use and limitation:** Explains the isolation model and residual assumptions; does not establish that every proposed platform configuration is secure.

### S24

gVisor contributors (2026b), *Networking*, official sandbox documentation, viewed 22 October 2026.

Available at: <https://gvisor.dev/docs/user_guide/networking/>.

**Use and limitation:** Network isolation configuration. Host services and application control channels still require explicit separation.

### S25

Podman contributors (2026), *podman-run manual*, official container documentation, viewed 22 October 2026.

Available at: <https://docs.podman.io/en/latest/markdown/podman-run.1.html>.

**Use and limitation:** Resource, network, capability and filesystem controls; an ordinary container is not a VM-equivalent guarantee.

### S26

OASIS Open; Fanning, M.C. and Golding, L.J. (eds) (2023), *Static Analysis Results Interchange Format (SARIF), version 2.1.0, incorporating approved errata*, published technical standard, viewed 22 October 2026.

Available at: <https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html>.

**Use and limitation:** Interoperable report schema. Evidence strength must remain explicit in properties rather than being implied by the export format.

### S27

Hypothesis contributors (2026), *Hypothesis documentation*, official testing-library documentation, viewed 22 October 2026.

Available at: <https://hypothesis.readthedocs.io/en/latest/>.

**Use and limitation:** Property-based testing implementation for Python. The product must supply a valid invariant; the framework does not invent ground truth.

### S28

Zhou, M. and Carderera de Diego, A. (2026), *ReviewBench: An open benchmark for AI code review*, primary benchmark announcement and methodology, viewed 22 October 2026.

Available at: <https://github.blog/ai-and-ml/github-copilot/reviewbench-an-open-benchmark-for-ai-code-review/>.

**Use and limitation:** Published 5 October 2026. Multi-source ground truth and grounded/augmented evaluation. Vendor-origin evidence, to be combined with independent and held-out evaluation.

### S29

Zhang, Y., Pan, Z., Yusuf, I.N.B., Ruan, H., Shariffdeen, R. and Roychoudhury, A. (2026), *Code Review Agent Benchmark*, primary research manuscript, arxiv:2603.23448v3, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2603.23448>.

**Use and limitation:** Submitted 24 March; version 3 revised 7 April 2026. Abstract and HTML methods examined. c-CRAB tests include both structural and behavioural oracles; review-to-fix evaluation depends partly on the fixing agent.

### S30

Kumar, D. (2026), *SWE-PRBench: Benchmarking AI Code Review Quality Against Pull Request Feedback*, primary research manuscript, arxiv:2603.26130, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2603.26130>.

**Use and limitation:** Submitted 27 March 2026. Abstract and HTML methods/limitations examined. Context degradation is a result for the tested configurations, not a universal law.

### S31

Cemri, M. et al. (2025), *Why Do Multi-Agent LLM Systems Fail?*, primary research manuscript, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2503.13657>.

**Use and limitation:** Role coordination, specification and verification failure modes motivate bounded orchestration. Not a direct benchmark of the proposed local reviewer.

### S32

Xia, C.S., Deng, Y., Dunn, S. and Zhang, L. (2024), *Agentless: Demystifying LLM-based Software Engineering Agents*, primary research manuscript, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2407.01489>.

**Use and limitation:** Supports testing a simpler staged baseline. Historical repair performance is not used as a current review-performance claim.

### S33

Debenedetti, E., Zhang, J., Balunović, M., Beurer-Kellner, L., Fischer, M. and Tramèr, F. (2024), *AgentDojo: A Dynamic Environment to Evaluate Prompt Injection Attacks and Defenses for LLM Agents*, primary research manuscript and benchmark, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2406.13352>.

**Use and limitation:** Motivates measuring both benign utility and attack outcomes when agents consume untrusted tool output.

### S34

Debenedetti, E. et al. (2025), *Defeating Prompt Injections by Design*, primary research manuscript, version 2, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2503.18813>.

**Use and limitation:** CaMeL control/data separation and capability-based enforcement inform the threat model. No claim that this proposed engine inherits its proof or measured results.

### S35

Ma, X., Li, T., Xiao, C., Yu, Z., Zhang, N. and Vorobeychik, Y. (2026), *AutoDojo: A Generative Benchmark for Evaluating Prompt Injection Defenses in LLM Agents*, primary research manuscript, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2606.15057>.

**Use and limitation:** Adaptive evaluation motivates attacks that are not frozen to the same fixtures used to develop the defence.

### S36

OpenAI (2026a), *Model Context Protocol: Connect ChatGPT and Codex to your tools and information*, official client documentation, viewed 22 October 2026.

Available at: <https://developers.openai.com/codex/mcp>.

**Use and limitation:** Client integration reference. Local tool execution is a different property from the privacy of the host model.

### S37

OpenAI (2026b), *Secure MCP Tunnel*, official integration documentation, viewed 22 October 2026.

Available at: <https://developers.openai.com/api/docs/guides/secure-mcp-tunnels>.

**Use and limitation:** An outbound route to OpenAI can connect private MCP servers, but is not offline. Included to explain why it is excluded from the strict release.

### S38

Anthropic (2026d), *Connect Claude Code to tools via MCP*, official client documentation, viewed 22 October 2026.

Available at: <https://code.claude.com/docs/en/mcp>.

**Use and limitation:** Claude Code compatibility reference. Test exact client versions against non-sensitive fixtures.

### S39

Astral (2026), *The Ruff linter*, official analyser documentation, viewed 22 October 2026.

Available at: <https://docs.astral.sh/ruff/linter/>.

**Use and limitation:** Fix safety labels and behaviour. Project-level patch approval and regression validation remain necessary.

### S40

PR-Agent contributors (2026), *PR Agent: The original open-source PR reviewer*, primary project documentation, viewed 22 October 2026.

Available at: <https://github.com/The-PR-Agent/pr-agent>.

**Use and limitation:** Existing open-source reviewer baseline. The former qodo-ai repository URL redirects here; do not conflate the community project with Qodo commercial products.

### S41

Kekatos, N. et al. (2026), *Formal Runtime Verification for Tool-Using LLM Agents: An Offline Same-Benchmark Study on AgentDojo and STAC*, primary research manuscript, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2610.09793>.

**Use and limitation:** Submitted 7 October 2026; abstract reviewed. Additional reading on trustworthy event/provenance fields. Its offline trace replay does not establish live enforcement effectiveness for this product.

### S42

Liu, Z. (2026), *Passing the Test You Trained On: Re-evaluating Prompt-Injection Detectors for LLM Agents*, primary research manuscript, viewed 22 October 2026.

Available at: <https://arxiv.org/abs/2610.03448>.

**Use and limitation:** Submitted 2 October 2026; abstract reviewed. Additional warning about benchmark transfer and training overlap. Not a reason to rely on a single injection classifier.

### S43

Model Context Protocol contributors (2026h), *SDKs*, official sdk catalogue, viewed 22 October 2026.

Available at: <https://modelcontextprotocol.io/docs/2026-07-28/sdk>.

**Use and limitation:** Confirms official Rust SDK status. The actual release used by implementation must still be pinned and conformance-tested.

## Source-to-decision map

| Decision | Sources |
|---|---|
| Public Ultra baseline and product distinctions | S01-S03 |
| Current protocol, task handles and compatibility | S04-S11, S43 |
| Local inference, schemas and model memory considerations | S12-S16 |
| Safe context and Git comparison | S17-S18 |
| Offline analysis and tool eligibility | S19-S22, S27, S39 |
| Local execution boundary | S23-S25 |
| Portable results | S26 |
| Review quality and simpler baselines | S28-S32, S40 |
| Prompt-injection threat model and adaptive evaluation | S33-S35 |
| Host/cloud-client privacy distinction | S36-S38 |
| Further provenance and detector-evaluation research | S41-S42 |

## Research limitations to preserve during implementation

No new benchmark was executed. No exact machine configuration was established. No existing Ultra Review MCP codebase was supplied or audited. No proprietary Claude implementation was inspected. SDK, runtime and model compatibility must be verified against immutable versions selected during implementation. Several sources are living documents, so their future contents may differ from this research snapshot.

The implementation should archive legally redistributable source metadata, schema versions and dependency manifests. It should not copy proprietary product prompts, redistribute model weights without checking their terms, or treat a vendor's marketing comparison as independent validation.

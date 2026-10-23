# Threat model

Status: initial (T03). Updated as each boundary lands. This document states
what the engine is designed to resist and what it does not claim. A passing
test suite is evidence about the tested cases only.

## Assets

1. The user's original checkout (must stay byte-for-byte unchanged during review, R04).
2. Source code, prompts, context and reports (must not leave the machine in strict mode, R01/R24).
3. Host secrets: home directory, SSH agent, keychain, cloud credentials, package caches.
4. The engine's own authority: policy, grants, approval keys, controller socket, audit store.
5. The integrity of findings: a published "reproduced bug" must be backed by independent evidence (R07, R25).

## Adversaries

- **The reviewed repository.** May contain malicious source, tests, build scripts,
  compiler plugins, package hooks, hostile filenames, prompt-injection text in
  comments and Markdown (including a forged `REVIEW.md`), forged tool output,
  terminal escape sequences, symlinks and `.git` indirection.
- **A prompt-injected or simply wrong model.** May emit invalid or malicious tool
  arguments, cite sources it never saw, claim verification it did not perform, or
  request capabilities.
- **A connected MCP client.** May be cloud-backed; may present another job's
  identifier; may reconnect and reuse stale handles.
- **Third-party tools** (scanners, compilers, runtimes) that try to reach the network
  or execute repository-controlled configuration.

## Out of scope

A compromised host administrator, kernel, or trusted model/runtime binary. Physical
access. Forensic erasure from SSDs or backups. Confidentiality at rest beyond the
operator's encrypted volume and OS account protection (design §18).

## Controls in place (T03)

| Threat | Control | Test |
|---|---|---|
| Repository text grants itself capabilities | `Policy` is built only from an operator profile and registered principals. `RepositoryGuidance` is labelled `untrusted` and has no path into policy or authorisation. | `repository_text_cannot_grant_capabilities` |
| Profile smuggles network access | `network_egress` has one value, `denied`; unknown keys are refused. | `repository_text_cannot_grant_capabilities`, `shipped_strict_profile_is_offline_with_design_defaults` |
| Unknown/traversing profile id falls back to defaults | Profile ids are `Ident`s; a missing profile is `POLICY_DENIED`, never a default. | `unknown_profile_fails_closed`, `profile_id_must_match_its_file` |
| Model applies a patch or approves itself | `PrincipalKind::Model` may only `read_context`; MCP clients may never `apply_patch`; grants must be issued by a `local_human` principal. | `model_cannot_apply_patch`, `model_cannot_mint_a_grant`, `models_are_limited_to_context_reads` |
| Client uses another caller's or repository's grant | Grants bind caller, repository, job, snapshot and scope digest; each is checked. | `another_callers_grant_is_denied`, `cross_repo_grant_is_denied`, `scope_cannot_expand_after_approval` |
| Stale authority after a policy change | Grants carry the policy digest; any profile/principal change invalidates them. | `policy_change_invalidates_existing_grants` |
| Expired authority | Expiry is exclusive and checked on every call. | `expired_grant_is_denied` |
| Unbound grant used as a wildcard | `run_preset`, `propose_patch` and `apply_patch` require grants bound to job, snapshot and scope digest. | `consequential_actions_require_fully_bound_grants` |
| Silent denial | Every decision is a serialisable record naming caller, action, grant, policy digest, time and reason. | `every_denial_is_structured_and_auditable` |
| Arbitrary commands via contracts | No contract has a command field; unknown fields are rejected by serde and by the generated JSON Schemas. | `unknown_command_field_is_rejected` (T02) |
| Terminal escape injection via report text | `Text` refuses C0/C1 controls other than tab/newline at the boundary. | `bounded_strings_and_arrays` (T02) |
| Unverified claims published as bugs | `validate_report` admits a finding only with a verification of its own hypothesis, by a different agent, of the same class, backed by evidence of the matching kind from a non-discoverer, not cited by any other finding and not contradicted by a disproof. `validate_report_against` ties cited sources to the snapshot manifest. | T02 contract tests incl. `verification_cannot_be_borrowed_by_another_finding`, `report_sources_must_match_the_snapshot` |
| Non-Rust code smuggled into the build | `audit-sources` rejects shebang/executable files (including `.rs`), `.cargo/config*`, non-Cargo or chained CI commands, custom shells and non-allowlisted actions; CI YAML is parsed, not pattern-matched. | `crates/xtask/tests/source_policy.rs` |

## Controls not yet in place

These are open and block the corresponding claims until their tasks land:

- Network egress enforcement for inference, scanners and workers (T05, T07, T08, T22).
  Until then **no offline guarantee is made**.
- Execution isolation (T05). Until then no repository code or generated test runs.
- Safe snapshots: hooks, filters, symlinks, `.git` indirection, concurrent edits (T06).
- Job ownership in the store and cross-caller reads (T04, T18, T19).
- Cache poisoning across jobs (T09, T16).
- Local human approval mechanism for patch application (T20).
- Adaptive adversarial suite (T22).

## Residual risks

- Model role independence means fresh contexts and separately recorded identities,
  not statistically independent errors: copies of one model share blind spots.
- A local MCP server can return findings to a cloud-connected host. The offline
  guarantee covers the standalone CLI and verified local-model hosts only.
- Principal kinds are asserted by the registering component; T17/T18 must bind them
  to real authentication (OS user for the CLI, per-client tokens for MCP).

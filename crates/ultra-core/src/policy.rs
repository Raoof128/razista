//! Caller-bound capability policy (R05, R13, R14).
//!
//! Authority comes from three trusted inputs only: the operator profile, the
//! registered principals, and a grant minted by a local human. Repository
//! text, tool output and model output are data; nothing here accepts them.
//! Every decision, allow or deny, is a structured record suitable for the
//! audit log, and a denial always names its reason.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::Profile;
use crate::errors::{ErrorCode, Result, ReviewError};
use crate::primitives::{
    BoundedVec, CallerId, Digest, GrantId, JobId, RepoId, SnapshotId, SourceText, Timestamp,
};
use crate::types::TrustLabel;

/// Version of the policy digest's canonical form.
pub const POLICY_VERSION: u32 = 1;

/// The closed set of review actions. Network access and installation are not
/// review actions and cannot be granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    ReadSnapshot,
    ReadContext,
    RunPreset,
    GenerateReport,
    ProposePatch,
    ApplyPatch,
}

/// What an action touches. Grants are checked against every bound field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionTarget {
    pub repo_id: RepoId,
    pub job_id: Option<JobId>,
    pub snapshot_id: Option<SnapshotId>,
    /// Digest of the approved `ResolvedScope`.
    pub scope_digest: Option<Digest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub kind: ActionKind,
    pub target: ActionTarget,
}

/// A capability bound to one caller, repository, optional job/snapshot/scope,
/// a set of actions, the policy it was issued under, and an expiry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionGrant {
    pub grant_id: GrantId,
    pub caller: CallerId,
    pub repo_id: RepoId,
    pub job_id: Option<JobId>,
    pub snapshot_id: Option<SnapshotId>,
    pub scope_digest: Option<Digest>,
    pub actions: BoundedVec<ActionKind, 6>,
    pub policy_digest: Digest,
    pub issued_by: CallerId,
    /// Exclusive: the grant is invalid at and after this instant.
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A person at the local CLI, authenticated by the OS account.
    LocalHuman,
    /// A connected MCP client. May be cloud-backed, so never an approver.
    McpClient,
    /// A model role inside the engine.
    Model,
    /// An execution worker. Requests nothing; it only runs what it is given.
    Worker,
}

impl PrincipalKind {
    /// The ceiling on what a kind of principal may ever do, regardless of grant.
    pub fn may(self, action: ActionKind) -> bool {
        use ActionKind::*;
        match self {
            Self::LocalHuman => true,
            Self::McpClient => action != ApplyPatch,
            Self::Model => action == ReadContext,
            Self::Worker => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub caller: CallerId,
    pub kind: PrincipalKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    UnknownCaller,
    GrantCallerMismatch,
    IssuerNotLocalHuman,
    PolicyChanged,
    GrantExpired,
    PrincipalKindForbidden,
    ActionNotGranted,
    RepositoryMismatch,
    JobMismatch,
    SnapshotMismatch,
    ScopeChanged,
    GrantNotBound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Verdict {
    Allow,
    Deny { reason: DenyReason },
}

/// One audited authorisation decision.
///
/// A denial is a value, not an error: callers must act on it, usually with
/// [`Decision::require`]. Dropping it unread is a bug.
#[must_use = "a Decision may be a denial; call require() or inspect verdict"]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub verdict: Verdict,
    pub caller: CallerId,
    pub action: ActionKind,
    pub grant_id: GrantId,
    pub policy_digest: Digest,
    pub at: Timestamp,
}

impl Decision {
    /// Turn a denial into a `POLICY_DENIED` error.
    pub fn require(&self) -> Result<()> {
        match &self.verdict {
            Verdict::Allow => Ok(()),
            Verdict::Deny { reason } => Err(ReviewError::new(
                ErrorCode::PolicyDenied,
                format!(
                    "{:?} denied for caller {}: {reason:?}",
                    self.action, self.caller
                ),
            )),
        }
    }
}

/// The trusted policy in force. Built only from a validated profile and the
/// locally registered principals; it has no constructor that takes
/// repository content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Policy {
    version: u32,
    profile: Profile,
    /// Sorted by caller so the digest is independent of registration order.
    principals: Vec<Principal>,
}

impl Policy {
    pub fn new(profile: Profile, mut principals: Vec<Principal>) -> Result<Self> {
        profile.validate()?;
        let mut seen = HashSet::new();
        for p in &principals {
            if !seen.insert(p.caller) {
                return Err(ReviewError::invalid(format!(
                    "caller {} is registered more than once",
                    p.caller
                )));
            }
        }
        principals.sort_by_key(|p| p.caller);
        Ok(Self {
            version: POLICY_VERSION,
            profile,
            principals,
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn principals(&self) -> &[Principal] {
        &self.principals
    }

    pub fn principal(&self, caller: &CallerId) -> Option<&Principal> {
        self.principals
            .binary_search_by_key(caller, |p| p.caller)
            .ok()
            .map(|i| &self.principals[i])
    }

    /// SHA-256 over the canonical JSON form. Any change to profile or
    /// principals changes the digest and so invalidates outstanding grants.
    pub fn digest(&self) -> Digest {
        // Serialising a struct of plain data cannot fail.
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        Digest::of(&bytes)
    }
}

/// Review guidance found in a repository (e.g. `REVIEW.md`). Always untrusted
/// advisory data from an approved base snapshot; it can inform prompts but
/// has no path into [`Policy`] or [`authorize`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryGuidance {
    from_snapshot: SnapshotId,
    text: SourceText,
}

impl RepositoryGuidance {
    pub fn advisory(from_snapshot: SnapshotId, text: SourceText) -> Self {
        Self {
            from_snapshot,
            text,
        }
    }

    pub fn trust(&self) -> TrustLabel {
        TrustLabel::Untrusted
    }

    pub fn from_snapshot(&self) -> SnapshotId {
        self.from_snapshot
    }

    pub fn text(&self) -> &SourceText {
        &self.text
    }
}

/// Authorise `action` for `caller` under `grant`, at the current time.
///
/// `grant` must be one the engine loaded from its own store by `GrantId`.
/// Never pass a grant deserialised from a client request: `issued_by` is only
/// as trustworthy as the place the grant came from (enforced from T04 on).
pub fn authorize(
    policy: &Policy,
    caller: &CallerId,
    action: &Action,
    grant: &ActionGrant,
) -> Result<Decision> {
    authorize_at(policy, caller, action, grant, Timestamp::now())
}

/// Authorise at an explicit instant. Checks run in a fixed order so the
/// reported reason is deterministic.
pub fn authorize_at(
    policy: &Policy,
    caller: &CallerId,
    action: &Action,
    grant: &ActionGrant,
    now: Timestamp,
) -> Result<Decision> {
    let verdict = match evaluate(policy, caller, action, grant, now) {
        None => Verdict::Allow,
        Some(reason) => Verdict::Deny { reason },
    };
    Ok(Decision {
        verdict,
        caller: *caller,
        action: action.kind,
        grant_id: grant.grant_id,
        policy_digest: policy.digest(),
        at: now,
    })
}

fn evaluate(
    policy: &Policy,
    caller: &CallerId,
    action: &Action,
    grant: &ActionGrant,
    now: Timestamp,
) -> Option<DenyReason> {
    use DenyReason::*;

    let Some(principal) = policy.principal(caller) else {
        return Some(UnknownCaller);
    };
    if grant.caller != *caller {
        return Some(GrantCallerMismatch);
    }
    let issuer_is_human = policy
        .principal(&grant.issued_by)
        .is_some_and(|p| p.kind == PrincipalKind::LocalHuman);
    if !issuer_is_human {
        return Some(IssuerNotLocalHuman);
    }
    if grant.policy_digest != policy.digest() {
        return Some(PolicyChanged);
    }
    if now >= grant.expires_at {
        return Some(GrantExpired);
    }
    if !principal.kind.may(action.kind) {
        return Some(PrincipalKindForbidden);
    }
    if !grant.actions.iter().any(|k| *k == action.kind) {
        return Some(ActionNotGranted);
    }

    let consequential = matches!(
        action.kind,
        ActionKind::RunPreset | ActionKind::ProposePatch | ActionKind::ApplyPatch
    );
    if consequential
        && (grant.job_id.is_none() || grant.snapshot_id.is_none() || grant.scope_digest.is_none())
    {
        return Some(GrantNotBound);
    }

    let target = &action.target;
    if target.repo_id != grant.repo_id {
        return Some(RepositoryMismatch);
    }
    if grant.job_id.is_some() && target.job_id != grant.job_id {
        return Some(JobMismatch);
    }
    if grant.snapshot_id.is_some() && target.snapshot_id != grant.snapshot_id {
        return Some(SnapshotMismatch);
    }
    if grant.scope_digest.is_some() && target.scope_digest != grant.scope_digest {
        return Some(ScopeChanged);
    }
    None
}

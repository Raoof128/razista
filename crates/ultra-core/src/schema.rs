//! JSON Schema generation for the public contracts. The committed copies under
//! `schemas/` are regenerated with `cargo run --locked -p xtask -- schemas` and
//! checked by `tests/contracts.rs`.

use schemars::{Schema, schema_for};

use crate::config::Profile;
use crate::policy::{Action, ActionGrant, Decision};

use crate::types::{
    BundleManifest, CapabilityReport, ContextPacket, Event, Evidence, ExecutionRecord, Finding,
    Hypothesis, ModelRequest, ModelResponse, Observation, PatchApproval, PatchProposal,
    PreparedReview, ReviewJob, ReviewReport, ReviewRequest, ReviewScope, ReviewStatus,
    SandboxRequest, Snapshot, Verification, VerificationRequest,
};

/// Every published contract, by stable name.
pub fn all() -> Vec<(&'static str, Schema)> {
    vec![
        ("Action", schema_for!(Action)),
        ("ActionGrant", schema_for!(ActionGrant)),
        ("BundleManifest", schema_for!(BundleManifest)),
        ("CapabilityReport", schema_for!(CapabilityReport)),
        ("ContextPacket", schema_for!(ContextPacket)),
        ("Decision", schema_for!(Decision)),
        ("Event", schema_for!(Event)),
        ("Evidence", schema_for!(Evidence)),
        ("ExecutionRecord", schema_for!(ExecutionRecord)),
        ("Finding", schema_for!(Finding)),
        ("Hypothesis", schema_for!(Hypothesis)),
        ("ModelRequest", schema_for!(ModelRequest)),
        ("ModelResponse", schema_for!(ModelResponse)),
        ("Observation", schema_for!(Observation)),
        ("PatchApproval", schema_for!(PatchApproval)),
        ("PatchProposal", schema_for!(PatchProposal)),
        ("PreparedReview", schema_for!(PreparedReview)),
        ("Profile", schema_for!(Profile)),
        ("ReviewJob", schema_for!(ReviewJob)),
        ("ReviewReport", schema_for!(ReviewReport)),
        ("ReviewRequest", schema_for!(ReviewRequest)),
        ("ReviewScope", schema_for!(ReviewScope)),
        ("ReviewStatus", schema_for!(ReviewStatus)),
        ("SandboxRequest", schema_for!(SandboxRequest)),
        ("Snapshot", schema_for!(Snapshot)),
        ("Verification", schema_for!(Verification)),
        ("VerificationRequest", schema_for!(VerificationRequest)),
    ]
}

pub fn schema_for_name(name: &str) -> Option<Schema> {
    all().into_iter().find(|(n, _)| *n == name).map(|(_, s)| s)
}

/// Stable on-disk rendering: pretty JSON with a trailing newline.
pub fn render(schema: &Schema) -> String {
    let mut out = serde_json::to_string_pretty(schema).unwrap_or_default();
    out.push('\n');
    out
}

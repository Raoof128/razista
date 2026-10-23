//! Trusted configuration.
//!
//! Precedence: built-in defaults, then an operator profile from the engine's
//! own profiles directory. Reviewed repository content is never a
//! configuration source. Profiles fail closed: an unknown id, an unknown key
//! or a zero limit is an error, never a fallback to something weaker.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, Result, ReviewError};
use crate::primitives::{Ident, Summary};
use crate::types::ResourceBudget;

/// Network egress for review work. The strict release has exactly one
/// setting, so "allowed" cannot even be written in a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NetworkEgress {
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub profile_id: Ident,
    pub description: Summary,
    pub offline_strict: bool,
    pub network_egress: NetworkEgress,
    pub budget: ResourceBudget,
}

impl Profile {
    pub fn network_egress_allowed(&self) -> bool {
        match self.network_egress {
            NetworkEgress::Denied => false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let b = &self.budget;
        let all_nonzero = [
            b.wall_time_ms,
            b.cpu_millis,
            b.memory_bytes,
            b.disk_bytes,
            b.output_bytes,
            u64::from(b.processes),
            b.tokens,
            u64::from(b.max_inference_concurrency),
            u64::from(b.max_verification_workers),
            u64::from(b.max_discovery_roles),
            u64::from(b.max_repair_attempts),
            u64::from(b.max_verification_candidates),
            u64::from(b.max_packet_tokens),
        ]
        .iter()
        .all(|&v| v > 0);
        if !all_nonzero {
            return Err(ReviewError::invalid(format!(
                "profile {}: every budget limit must be greater than zero",
                self.profile_id
            )));
        }
        // Six discovery roles plus verifier and challenger exist.
        if b.max_discovery_roles > 6 {
            return Err(ReviewError::invalid(format!(
                "profile {}: max_discovery_roles cannot exceed the 6 discovery roles",
                self.profile_id
            )));
        }
        Ok(())
    }
}

/// Load `<dir>/<id>.toml`. The id must be an [`Ident`], so it cannot traverse.
pub fn load_profile(dir: &Path, id: &str) -> Result<Profile> {
    let ident = Ident::new(id)?;
    let path = dir.join(format!("{}.toml", ident.as_str()));
    let text = std::fs::read_to_string(&path).map_err(|e| {
        ReviewError::new(
            ErrorCode::PolicyDenied,
            format!("unknown or unreadable profile {id:?}: {e}"),
        )
    })?;
    let profile: Profile = toml::from_str(&text)
        .map_err(|e| ReviewError::invalid(format!("profile {id:?} is invalid: {e}")))?;
    if profile.profile_id != ident {
        return Err(ReviewError::invalid(format!(
            "profile file {id:?} declares id {:?}",
            profile.profile_id.as_str()
        )));
    }
    profile.validate()?;
    Ok(profile)
}

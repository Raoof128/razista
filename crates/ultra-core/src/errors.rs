//! Structured review errors (design §18 "Required error distinctions").

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type Result<T, E = ReviewError> = std::result::Result<T, E>;

/// Stable error codes. Every code declares whether a retry may succeed and
/// what it does to review coverage, so a failure is never reported as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BaseUnavailable,
    SnapshotUnstable,
    PolicyDenied,
    SandboxUnavailable,
    EgressPolicyUnverified,
    DependencyUnavailable,
    LanguageUnsupported,
    ModelNotInstalled,
    ModelOutputInvalid,
    OutOfMemory,
    BudgetExhausted,
    WorkerTimeout,
    EvidenceInvalid,
    StalePatch,
    StoreUnavailable,
    /// Input failed contract validation (bounds, enums, unknown fields, config).
    InvalidInput,
}

/// What a failure does to the review's coverage claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoverageImpact {
    /// The job cannot start or produce any report.
    Blocking,
    /// Work continues; the affected scope is recorded as a coverage gap.
    PartialCoverage,
    /// The rejected item is dropped; other coverage is unaffected.
    None,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 16] = [
        Self::BaseUnavailable,
        Self::SnapshotUnstable,
        Self::PolicyDenied,
        Self::SandboxUnavailable,
        Self::EgressPolicyUnverified,
        Self::DependencyUnavailable,
        Self::LanguageUnsupported,
        Self::ModelNotInstalled,
        Self::ModelOutputInvalid,
        Self::OutOfMemory,
        Self::BudgetExhausted,
        Self::WorkerTimeout,
        Self::EvidenceInvalid,
        Self::StalePatch,
        Self::StoreUnavailable,
        Self::InvalidInput,
    ];

    /// Whether repeating the same request unchanged can succeed.
    pub fn retryable(self) -> bool {
        match self {
            Self::SnapshotUnstable
            | Self::OutOfMemory
            | Self::WorkerTimeout
            | Self::StoreUnavailable => true,
            Self::BaseUnavailable
            | Self::PolicyDenied
            | Self::SandboxUnavailable
            | Self::EgressPolicyUnverified
            | Self::DependencyUnavailable
            | Self::LanguageUnsupported
            | Self::ModelNotInstalled
            | Self::ModelOutputInvalid
            | Self::BudgetExhausted
            | Self::EvidenceInvalid
            | Self::StalePatch
            | Self::InvalidInput => false,
        }
    }

    pub fn coverage_impact(self) -> CoverageImpact {
        match self {
            Self::BaseUnavailable
            | Self::SnapshotUnstable
            | Self::PolicyDenied
            | Self::SandboxUnavailable
            | Self::EgressPolicyUnverified
            | Self::ModelNotInstalled
            | Self::StoreUnavailable
            | Self::InvalidInput => CoverageImpact::Blocking,
            Self::DependencyUnavailable
            | Self::LanguageUnsupported
            | Self::ModelOutputInvalid
            | Self::OutOfMemory
            | Self::BudgetExhausted
            | Self::WorkerTimeout => CoverageImpact::PartialCoverage,
            Self::EvidenceInvalid | Self::StalePatch => CoverageImpact::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ReviewError {
    code: ErrorCode,
    message: String,
}

impl ReviewError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn evidence(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::EvidenceInvalid, message)
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

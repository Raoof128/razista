//! Domain contracts, errors, configuration and policy for the local review
//! engine. Every other crate speaks these types.

pub mod config;
pub mod errors;
pub mod policy;
pub mod primitives;
pub mod schema;
pub mod types;
pub mod validate;

pub use errors::{CoverageImpact, ErrorCode, Result, ReviewError};
pub use primitives::*;
pub use types::*;
pub use validate::{validate_report, validate_report_against, validate_request};

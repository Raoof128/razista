//! Immutable repository snapshots (T06).
//!
//! [`Snapshotter::capture`] freezes an exact review scope (branch, range,
//! commit, staged, worktree or paths) into a [`razista_core::Snapshot`]: a
//! manifest of every included file with its digest and change kind, every
//! skipped path with a reason, and visible gaps. The bytes go into the store as
//! untrusted artefacts. The user's checkout is never written to, git never
//! runs a hook, filter or helper, and nothing is fetched.
//!
//! Unix only for now: Windows path and ACL handling is a T23 platform task.

mod diff;
mod git;
mod paths;
mod snapshot;

pub use git::GitBinary;
pub use snapshot::{CaptureOptions, Snapshotter};

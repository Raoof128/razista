//! Durable local state for the review engine (T04).
//!
//! One SQLite database (`store.db`, WAL, `synchronous=FULL`) holds metadata;
//! evidence bytes live in a content-addressed blob directory. Every write that
//! a caller is told succeeded has been committed first. Every read checks the
//! owning caller: an id is a name, not a credential.
//!
//! Layout under the store root (all owner-only):
//!
//! ```text
//! store.db           metadata, events, links
//! blobs/ab/<hex>     immutable artefact bytes, named by SHA-256
//! tmp/               partially written blobs; emptied on every open
//! ```

mod cas;
mod db;
mod events;
mod migrations;

pub use db::{
    AtRestProtection, Clock, RecoveryReport, RetentionPolicy, RetentionReport, Store, StoreProbe,
    SystemClock,
};
pub use migrations::{MIGRATIONS, Migration};

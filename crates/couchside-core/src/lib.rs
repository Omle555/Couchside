//! Shared types, configuration and pure helpers used by every Couchside crate.

pub mod config;
pub mod ids;
pub mod kinds;
pub mod naming;
pub mod net;
pub mod paths;
pub mod standard_names;

pub use config::Config;
pub use ids::{LibraryId, MediaId, ShowId, SubtitleId, UserId};
pub use kinds::{LibraryKind, MediaKind};

/// Current time as whole seconds since the Unix epoch.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

//! Strongly typed database identifiers.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(feature = "sqlx", derive(sqlx::Type), sqlx(transparent))]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(
    /// Identifies a media library (a root folder of movies or shows).
    LibraryId
);
id_type!(
    /// Identifies a playable media item (a movie or an episode).
    MediaId
);
id_type!(
    /// Identifies a TV show.
    ShowId
);
id_type!(
    /// Identifies an external subtitle file.
    SubtitleId
);
id_type!(
    /// Identifies a user account.
    UserId
);

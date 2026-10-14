//! Enumerations stored in the database and exposed through the API.

use serde::{Deserialize, Serialize};

/// What a library contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type), sqlx(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
pub enum LibraryKind {
    /// One movie per file (optionally in its own folder).
    Movies,
    /// `Show/Season NN/Show - SxxEyy.ext` layout.
    Shows,
}

/// What a playable media item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type), sqlx(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    /// A feature film.
    Movie,
    /// A single TV episode.
    Episode,
}

/// What a stream inside a media file carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type), sqlx(rename_all = "lowercase"))]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    /// Pictures.
    Video,
    /// Sound.
    Audio,
    /// Subtitles (text or image).
    Subtitle,
}

//! Rows returned from the database. These are internal types: the API maps
//! them to its own DTOs before sending anything to clients.

use couchside_core::{
    LibraryId, LibraryKind, MediaId, MediaKind, ShowId, StreamKind, SubtitleId, UserId,
};
use sqlx::FromRow;

/// A configured media library.
#[derive(Debug, Clone, FromRow)]
pub struct Library {
    /// Library id.
    pub id: LibraryId,
    /// Display name.
    pub name: String,
    /// Movies or shows.
    pub kind: LibraryKind,
    /// Root folder on disk.
    pub path: String,
    /// Creation time (Unix seconds).
    pub created_at: i64,
}

/// A user account.
#[derive(Debug, Clone, FromRow)]
pub struct User {
    /// User id.
    pub id: UserId,
    /// Login name (case-insensitive unique).
    pub username: String,
    /// Argon2id PHC string.
    pub password_hash: String,
    /// Administrators can manage libraries and users.
    pub is_admin: bool,
    /// Creation time (Unix seconds).
    pub created_at: i64,
}

/// A TV show.
#[derive(Debug, Clone, FromRow)]
pub struct Show {
    /// Show id.
    pub id: ShowId,
    /// Owning library.
    pub library_id: LibraryId,
    /// Show folder on disk.
    pub folder_path: String,
    /// Display title.
    pub title: String,
    /// Sort key.
    pub sort_title: String,
    /// First-air year.
    pub year: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// Cached poster file name in the image cache.
    pub poster_file: Option<String>,
    /// Cached backdrop file name in the image cache.
    pub backdrop_file: Option<String>,
    /// TMDB id once matched.
    pub tmdb_id: Option<i64>,
    /// Time first seen (Unix seconds).
    pub added_at: i64,
    /// Number of episode files.
    pub episode_count: i64,
}

/// A playable movie or episode file.
#[derive(Debug, Clone, FromRow)]
pub struct MediaItem {
    /// Item id.
    pub id: MediaId,
    /// Owning library.
    pub library_id: LibraryId,
    /// Movie or episode.
    pub kind: MediaKind,
    /// File path on disk.
    pub path: String,
    /// File size in bytes.
    pub file_size: i64,
    /// Display title.
    pub title: String,
    /// Sort key.
    pub sort_title: String,
    /// Release year.
    pub year: Option<i64>,
    /// Parent show for episodes.
    pub show_id: Option<ShowId>,
    /// Season number for episodes.
    pub season: Option<i64>,
    /// Episode number for episodes.
    pub episode: Option<i64>,
    /// Last episode number for multi-episode files.
    pub episode_end: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// Cached poster file name.
    pub poster_file: Option<String>,
    /// Cached backdrop / still file name.
    pub backdrop_file: Option<String>,
    /// TMDB id once matched.
    pub tmdb_id: Option<i64>,
    /// Duration from ffprobe.
    pub runtime_seconds: Option<f64>,
    /// Container format from ffprobe.
    pub container: Option<String>,
    /// First video stream codec.
    pub video_codec: Option<String>,
    /// First audio stream codec.
    pub audio_codec: Option<String>,
    /// Video width in pixels.
    pub width: Option<i64>,
    /// Video height in pixels.
    pub height: Option<i64>,
    /// Time first seen (Unix seconds).
    pub added_at: i64,
    /// Original-language title.
    pub original_title: Option<String>,
    /// Tagline.
    pub tagline: Option<String>,
    /// Genre names as a JSON array.
    pub genres: Option<String>,
    /// TMDB average rating out of 10.
    pub rating: Option<f64>,
    /// Release date, `YYYY-MM-DD`.
    pub release_date: Option<String>,
    /// Age rating, e.g. `M`.
    pub certification: Option<String>,
    /// IMDb id.
    pub imdb_id: Option<String>,
    /// Top cast as a JSON array.
    pub cast_names: Option<String>,
    /// Directors as a JSON array.
    pub directors: Option<String>,
    /// The TMDB match was chosen by hand.
    pub match_locked: bool,
}

/// What the metadata fetcher needs to find a movie on TMDB.
#[derive(Debug, Clone, FromRow)]
pub struct MovieLookup {
    /// Item id.
    pub id: MediaId,
    /// Title parsed from the file name.
    pub parsed_title: String,
    /// Year parsed from the file name.
    pub parsed_year: Option<i64>,
    /// TMDB id tagged in the file name.
    pub hint_tmdb_id: Option<i64>,
    /// IMDb id tagged in the file name.
    pub hint_imdb_id: Option<String>,
    /// Current TMDB match.
    pub tmdb_id: Option<i64>,
    /// The current match was chosen by hand and must be kept.
    pub match_locked: bool,
}

/// An external subtitle file next to a media item.
#[derive(Debug, Clone, FromRow)]
pub struct Subtitle {
    /// Subtitle id.
    pub id: SubtitleId,
    /// Media item it belongs to.
    pub media_id: MediaId,
    /// File path on disk.
    pub path: String,
    /// Language tag from the file name (`en`, `English`...).
    pub language: Option<String>,
    /// File extension (`srt`, `vtt`, `ass`, `ssa`).
    pub format: String,
}

/// A user's playback position for one item.
#[derive(Debug, Clone, FromRow)]
pub struct Progress {
    /// Media item.
    pub media_id: MediaId,
    /// Resume position in seconds.
    pub position_seconds: f64,
    /// Marked as watched.
    pub played: bool,
    /// Last update (Unix seconds).
    pub updated_at: i64,
}

/// One video, audio or subtitle stream inside a media file, as read by ffprobe.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct MediaStream {
    /// ffprobe's stream index (what `-map 0:N` refers to).
    pub stream_index: i64,
    /// Video, audio or subtitle.
    pub kind: StreamKind,
    /// ffprobe codec name, e.g. `h264`, `hevc`, `ac3`, `subrip`, `hdmv_pgs_subtitle`.
    pub codec: Option<String>,
    /// Codec profile, e.g. `High`, `Main 10`, `DTS-HD MA`.
    pub profile: Option<String>,
    /// Video pixel format, e.g. `yuv420p10le`.
    pub pix_fmt: Option<String>,
    /// Bits per colour sample for video (8, 10, 12).
    pub bit_depth: Option<i64>,
    /// Video transfer characteristics; `smpte2084` (HDR10) and `arib-std-b67` (HLG) mean HDR.
    pub color_transfer: Option<String>,
    /// Audio channel count.
    pub channels: Option<i64>,
    /// Language tag, e.g. `eng`.
    pub language: Option<String>,
    /// Stream title, e.g. `Director's commentary`.
    pub title: Option<String>,
    /// Marked as the default stream of its kind.
    pub is_default: bool,
    /// Marked as forced (subtitles for foreign-language parts).
    pub is_forced: bool,
}

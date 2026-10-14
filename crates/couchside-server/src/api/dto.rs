//! Request and response bodies. Database rows are always mapped to these
//! before being sent, so internal fields (paths, hashes) never leak.

use couchside_core::{LibraryId, LibraryKind, MediaId, MediaKind, ShowId, SubtitleId, UserId};
use couchside_db::{Library, MediaItem, Progress, Show, Subtitle, User};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// `POST /auth/login` and `POST /setup` body.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Login name.
    pub username: String,
    /// Plain-text password (sent over the local network only).
    pub password: String,
}

/// A user as seen by clients.
#[derive(Debug, Serialize)]
pub struct UserDto {
    /// User id.
    pub id: UserId,
    /// Login name.
    pub username: String,
    /// Administrator flag.
    pub is_admin: bool,
}

impl From<&User> for UserDto {
    fn from(u: &User) -> Self {
        Self {
            id: u.id,
            username: u.username.clone(),
            is_admin: u.is_admin,
        }
    }
}

/// `POST /users` body.
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    /// Login name.
    pub username: String,
    /// Password.
    pub password: String,
    /// Make the user an administrator.
    #[serde(default)]
    pub is_admin: bool,
}

/// `PUT /users/{id}/password` body.
#[derive(Debug, Deserialize)]
pub struct PasswordRequest {
    /// New password.
    pub password: String,
}

/// A library as seen by clients.
#[derive(Debug, Serialize)]
pub struct LibraryDto {
    /// Library id.
    pub id: LibraryId,
    /// Display name.
    pub name: String,
    /// Movies or shows.
    pub kind: LibraryKind,
    /// Root folder (shown to signed-in users so they can recognise it).
    pub path: String,
}

impl From<Library> for LibraryDto {
    fn from(l: Library) -> Self {
        Self {
            id: l.id,
            name: l.name,
            kind: l.kind,
            path: l.path,
        }
    }
}

/// `POST /libraries` body.
#[derive(Debug, Deserialize)]
pub struct CreateLibraryRequest {
    /// Display name.
    pub name: String,
    /// Movies or shows.
    pub kind: LibraryKind,
    /// Root folder, e.g. `D:\Movies` or `\\NAS\Video\TV`.
    pub path: String,
}

/// Playback state for the current user.
#[derive(Debug, Serialize)]
pub struct ProgressDto {
    /// Resume position in seconds.
    pub position_seconds: f64,
    /// Watched flag.
    pub played: bool,
}

impl From<&Progress> for ProgressDto {
    fn from(p: &Progress) -> Self {
        Self {
            position_seconds: p.position_seconds,
            played: p.played,
        }
    }
}

/// `PUT /items/{id}/progress` body.
#[derive(Debug, Deserialize)]
pub struct ProgressRequest {
    /// Current position in seconds.
    pub position_seconds: f64,
    /// Explicitly mark watched/unwatched. If omitted, it is inferred from the position.
    pub played: Option<bool>,
}

/// A movie or episode in lists and detail views.
#[derive(Debug, Serialize)]
pub struct MediaItemDto {
    /// Item id.
    pub id: MediaId,
    /// Movie or episode.
    pub kind: MediaKind,
    /// Display title.
    pub title: String,
    /// Release year.
    pub year: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// A poster image is available at `/items/{id}/image/poster`.
    pub has_poster: bool,
    /// A backdrop/still is available at `/items/{id}/image/backdrop`.
    pub has_backdrop: bool,
    /// Parent show.
    pub show_id: Option<ShowId>,
    /// Season number.
    pub season: Option<i64>,
    /// Episode number.
    pub episode: Option<i64>,
    /// Last episode for multi-episode files.
    pub episode_end: Option<i64>,
    /// Duration in seconds.
    pub runtime_seconds: Option<f64>,
    /// Video codec.
    pub video_codec: Option<String>,
    /// Audio codec.
    pub audio_codec: Option<String>,
    /// Container format.
    pub container: Option<String>,
    /// Video width.
    pub width: Option<i64>,
    /// Video height.
    pub height: Option<i64>,
    /// When it was added (Unix seconds).
    pub added_at: i64,
    /// The current user's progress.
    pub progress: Option<ProgressDto>,
    /// TMDB id of the matched movie.
    pub tmdb_id: Option<i64>,
    /// IMDb id.
    pub imdb_id: Option<String>,
    /// Original-language title.
    pub original_title: Option<String>,
    /// Tagline.
    pub tagline: Option<String>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Average rating out of 10.
    pub rating: Option<f64>,
    /// Release date, `YYYY-MM-DD`.
    pub release_date: Option<String>,
    /// Age rating for the configured region.
    pub certification: Option<String>,
    /// Top cast.
    pub cast: Vec<String>,
    /// Directors.
    pub directors: Vec<String>,
    /// The TMDB match was chosen by hand.
    pub match_locked: bool,
}

/// Parses a JSON array column, tolerating missing or malformed data.
fn json_list(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|r| serde_json::from_str(r).ok())
        .unwrap_or_default()
}

impl MediaItemDto {
    /// Maps a row plus the user's progress.
    pub fn new(item: MediaItem, progress: Option<&Progress>) -> Self {
        Self {
            id: item.id,
            kind: item.kind,
            title: item.title,
            year: item.year,
            overview: item.overview,
            // Episodes fall back to their show's poster.
            has_poster: item.poster_file.is_some() || item.show_id.is_some(),
            has_backdrop: item.backdrop_file.is_some(),
            show_id: item.show_id,
            season: item.season,
            episode: item.episode,
            episode_end: item.episode_end,
            runtime_seconds: item.runtime_seconds,
            video_codec: item.video_codec,
            audio_codec: item.audio_codec,
            container: item.container,
            width: item.width,
            height: item.height,
            added_at: item.added_at,
            progress: progress.map(ProgressDto::from),
            tmdb_id: item.tmdb_id,
            imdb_id: item.imdb_id,
            original_title: item.original_title,
            tagline: item.tagline,
            genres: json_list(item.genres.as_deref()),
            rating: item.rating,
            release_date: item.release_date,
            certification: item.certification,
            cast: json_list(item.cast_names.as_deref()),
            directors: json_list(item.directors.as_deref()),
            match_locked: item.match_locked,
        }
    }

    /// Maps a list using a progress lookup table.
    pub fn list(items: Vec<MediaItem>, progress: &HashMap<MediaId, Progress>) -> Vec<Self> {
        items
            .into_iter()
            .map(|item| {
                let p = progress.get(&item.id);
                Self::new(item, p)
            })
            .collect()
    }
}

/// A show in lists.
#[derive(Debug, Serialize)]
pub struct ShowDto {
    /// Show id.
    pub id: ShowId,
    /// Display title.
    pub title: String,
    /// First-air year.
    pub year: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// Poster available.
    pub has_poster: bool,
    /// Backdrop available.
    pub has_backdrop: bool,
    /// Number of episode files.
    pub episode_count: i64,
}

impl From<Show> for ShowDto {
    fn from(s: Show) -> Self {
        Self {
            id: s.id,
            title: s.title,
            year: s.year,
            overview: s.overview,
            has_poster: s.poster_file.is_some(),
            has_backdrop: s.backdrop_file.is_some(),
            episode_count: s.episode_count,
        }
    }
}

/// One season in a show detail view.
#[derive(Debug, Serialize)]
pub struct SeasonDto {
    /// Season number (0 = specials).
    pub season: i64,
    /// Episodes in order.
    pub episodes: Vec<MediaItemDto>,
}

/// `GET /shows/{id}` response.
#[derive(Debug, Serialize)]
pub struct ShowDetailDto {
    /// The show.
    pub show: ShowDto,
    /// Seasons in order.
    pub seasons: Vec<SeasonDto>,
}

/// An external subtitle track.
#[derive(Debug, Serialize)]
pub struct SubtitleDto {
    /// Subtitle id (fetch as WebVTT from `/subtitles/{id}`).
    pub id: SubtitleId,
    /// Language tag, if known.
    pub language: Option<String>,
    /// Original file format.
    pub format: String,
    /// Label for menus.
    pub label: String,
}

impl From<Subtitle> for SubtitleDto {
    fn from(s: Subtitle) -> Self {
        let label = match &s.language {
            Some(lang) => format!("{lang} ({})", s.format.to_uppercase()),
            None => s.format.to_uppercase(),
        };
        Self {
            id: s.id,
            language: s.language,
            format: s.format,
            label,
        }
    }
}

/// `GET /items/{id}` response.
#[derive(Debug, Serialize)]
pub struct ItemDetailDto {
    /// The item.
    pub item: MediaItemDto,
    /// Parent show title for episodes.
    pub show_title: Option<String>,
    /// File name only (no folders).
    pub file_name: String,
    /// File size in bytes.
    pub file_size: i64,
    /// Likely playable directly in a browser.
    pub browser_playable: bool,
    /// External subtitle tracks.
    pub subtitles: Vec<SubtitleDto>,
}

/// File name portion of a stored path.
pub fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A paged list.
#[derive(Debug, Serialize)]
pub struct Page<T> {
    /// Items on this page.
    pub items: Vec<T>,
    /// Total matching items.
    pub total: i64,
    /// Page size used.
    pub limit: i64,
    /// Offset used.
    pub offset: i64,
}

/// `GET /home` response.
#[derive(Debug, Serialize)]
pub struct HomeDto {
    /// Partly watched items.
    pub continue_watching: Vec<MediaItemDto>,
    /// Newest movies.
    pub recent_movies: Vec<MediaItemDto>,
    /// Newest episodes.
    pub recent_episodes: Vec<MediaItemDto>,
}

/// `GET /search` response.
#[derive(Debug, Serialize)]
pub struct SearchDto {
    /// Matching movies.
    pub movies: Vec<MediaItemDto>,
    /// Matching shows.
    pub shows: Vec<ShowDto>,
}

/// List query parameters.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// Restrict to one library.
    pub library_id: Option<i64>,
    /// `title` (default), `added` or `year`.
    pub sort: Option<String>,
    /// Page size.
    pub limit: Option<i64>,
    /// Offset.
    pub offset: Option<i64>,
}

/// Search query parameters.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    /// Text to look for in titles.
    pub q: String,
}

/// `GET/PUT /settings/metadata` response.
#[derive(Debug, Serialize)]
pub struct MetadataSettingsDto {
    /// A TMDB key is configured.
    pub has_key: bool,
    /// `"app"` (entered in Settings) or `"config"` (config.toml).
    pub key_source: Option<&'static str>,
    /// Last characters of the key, for recognition.
    pub key_hint: Option<String>,
    /// Metadata language, e.g. `en-AU`.
    pub language: String,
}

/// `PUT /settings/metadata` body. Omitted fields are unchanged; an empty key removes it.
#[derive(Debug, Deserialize)]
pub struct MetadataSettingsRequest {
    /// New TMDB key ("" = remove the key saved in Settings).
    pub tmdb_api_key: Option<String>,
    /// New language.
    pub language: Option<String>,
}

/// `GET`/`PUT /settings/library`: what Couchside may do to files in your libraries.
#[derive(Debug, Serialize, Deserialize)]
pub struct LibrarySettingsDto {
    /// "Clean up" may rename movie files (always after showing the new name).
    pub allow_file_renames: bool,
}

/// `GET /items/{id}/identify` query.
#[derive(Debug, Deserialize)]
pub struct IdentifyQuery {
    /// Title to search for (defaults to the title parsed from the file name).
    pub q: Option<String>,
    /// Year to search for (defaults to the parsed year).
    pub year: Option<i64>,
}

/// One TMDB search result in the Identify dialog.
#[derive(Debug, Serialize)]
pub struct CandidateDto {
    /// TMDB id.
    pub tmdb_id: i64,
    /// Title.
    pub title: String,
    /// Original-language title.
    pub original_title: Option<String>,
    /// Release year.
    pub year: Option<i64>,
    /// Shortened plot summary.
    pub overview: Option<String>,
    /// TMDB poster path, for `/tmdb/image?path=`.
    pub poster_path: Option<String>,
    /// Match score (>= 0.6 is a confident match).
    pub score: f64,
    /// This is the current match.
    pub current: bool,
}

/// `POST /items/{id}/identify` body.
#[derive(Debug, Deserialize)]
pub struct IdentifyRequest {
    /// The chosen TMDB movie id.
    pub tmdb_id: i64,
}

/// `POST /items/{id}/title-scan` query.
#[derive(Debug, Deserialize)]
pub struct TitleScanQuery {
    /// `start` (default) for the opening, `end` for the end credits.
    pub part: Option<String>,
}

/// One frame from a title scan.
#[derive(Debug, Serialize)]
pub struct TitleScanFrameDto {
    /// Image index for `/items/{id}/title-scan/frames/{index}`.
    pub index: usize,
    /// Position in the video, in seconds.
    pub time_seconds: f64,
    /// Lines of text read on the frame.
    pub lines: Vec<String>,
}

/// `POST /items/{id}/title-scan` response.
#[derive(Debug, Serialize)]
pub struct TitleScanDto {
    /// Frames to check, in video order.
    pub frames: Vec<TitleScanFrameDto>,
    /// On-screen text that looks like a title, best first. Never applied automatically.
    pub suggestions: Vec<String>,
    /// Whether Windows text recognition ran.
    pub ocr_available: bool,
    /// How many frames were read from the video.
    pub frames_read: usize,
}

/// `GET /tmdb/image` query.
#[derive(Debug, Deserialize)]
pub struct TmdbImageQuery {
    /// TMDB image path such as `/abc.jpg`.
    pub path: String,
}

/// One rename shown before "Clean up" runs, relative to the library folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenameStepDto {
    /// Current name.
    pub from: String,
    /// New name.
    pub to: String,
}

/// `GET /items/{id}/cleanup`: what "Clean up" would do.
#[derive(Debug, Serialize)]
pub struct CleanupPreviewDto {
    /// `allow_file_renames` is switched on in config.toml.
    pub renames_enabled: bool,
    /// The video's current name, relative to its library folder.
    pub current_name: Option<String>,
    /// The video's new name; `None` if it already has its standard name or can't be renamed.
    pub new_name: Option<String>,
    /// Other files and folders renamed with it (subtitles, artwork, its folder).
    pub other_steps: Vec<RenameStepDto>,
    /// Why the file won't be renamed, when renaming is on but not possible.
    pub rename_note: Option<String>,
    /// Images and subtitles that would be moved out of the library (relative names).
    pub remove_files: Vec<String>,
}

/// `POST /items/{id}/cleanup` body.
#[derive(Debug, Deserialize)]
pub struct CleanupRequest {
    /// The new name shown in the preview. Set only when the user agreed to rename; the file is
    /// renamed only if the new name still matches it.
    pub rename_to: Option<String>,
    /// Images and subtitles from the preview the user agreed to remove. Only files the
    /// preview would also list are removed.
    pub remove_files: Option<Vec<String>>,
}

/// `POST /items/{id}/cleanup` response.
#[derive(Debug, Serialize)]
pub struct CleanupResultDto {
    /// The file was renamed.
    pub renamed: bool,
    /// What happened, for a short message.
    pub message: String,
}

/// `GET /items/{id}/delete`: what deleting a movie removes, relative to its library folder.
/// The same shape is sent back to `POST /items/{id}/delete` to confirm.
#[derive(Debug, Serialize)]
pub struct DeletePreviewDto {
    /// The movie's own folder, removed with everything in it.
    pub folder: Option<String>,
    /// Or, in a folder shared with other movies, just the movie and the files named after it.
    pub files: Vec<String>,
}

/// `POST /items/{id}/delete` body: the preview the user confirmed.
#[derive(Debug, Deserialize)]
pub struct DeleteMovieRequest {
    /// Folder from the preview.
    pub folder: Option<String>,
    /// Files from the preview.
    #[serde(default)]
    pub files: Vec<String>,
}

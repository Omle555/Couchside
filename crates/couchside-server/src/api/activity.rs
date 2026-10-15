//! `GET /activity`: library totals and what is playing now, for Home Assistant and dashboards.

use super::auth::AdminUser;
use super::ApiError;
use crate::app::AppState;
use axum::extract::State;
use axum::Json;
use couchside_core::{MediaId, MediaKind};
use couchside_db::NowPlaying;
use serde::Serialize;

/// Players save their position every 10 seconds while playing; anything saved more recently
/// than this counts as playing now.
const PLAYING_WINDOW_SECS: i64 = 30;

/// Response of `GET /activity`.
#[derive(Debug, Serialize)]
pub struct ActivityDto {
    /// This PC's name, the same as the `server` entry of the network announcement.
    pub server: Option<String>,
    /// Couchside version.
    pub version: &'static str,
    /// Movies in all libraries.
    pub movies: i64,
    /// TV shows in all libraries.
    pub shows: i64,
    /// TV episodes in all libraries.
    pub episodes: i64,
    /// A library scan or metadata refresh is running.
    pub scanning: bool,
    /// Repackaged or converted streams currently open.
    pub converting: usize,
    /// What people are watching right now, newest first.
    pub now_playing: Vec<NowPlayingDto>,
}

/// One thing being watched.
#[derive(Debug, Serialize)]
pub struct NowPlayingDto {
    /// Who is watching.
    pub user: String,
    /// The item.
    pub item_id: MediaId,
    /// `movie` or `episode`.
    pub kind: MediaKind,
    /// Movie or episode title.
    pub title: String,
    /// Show title, for episodes.
    pub show_title: Option<String>,
    /// Season number, for episodes.
    pub season: Option<i64>,
    /// Episode number, for episodes.
    pub episode: Option<i64>,
    /// Position in seconds when last saved.
    pub position_seconds: f64,
    /// Length in seconds, when known.
    pub runtime_seconds: Option<f64>,
}

impl From<NowPlaying> for NowPlayingDto {
    fn from(p: NowPlaying) -> Self {
        Self {
            user: p.username,
            item_id: p.media_id,
            kind: p.kind,
            title: p.title,
            show_title: p.show_title,
            season: p.season,
            episode: p.episode,
            position_seconds: p.position_seconds,
            runtime_seconds: p.runtime_seconds,
        }
    }
}

/// `GET /activity` (admin)
pub async fn get(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Result<Json<ActivityDto>, ApiError> {
    let counts = state.db.library_counts().await?;
    let now_playing = state.db.now_playing(PLAYING_WINDOW_SECS).await?;
    Ok(Json(ActivityDto {
        server: crate::lan::computer_name(),
        version: env!("CARGO_PKG_VERSION"),
        movies: counts.movies,
        shows: counts.shows,
        episodes: counts.episodes,
        scanning: state.manager.status().running,
        converting: state.transcoder.open_sessions(),
        now_playing: now_playing.into_iter().map(NowPlayingDto::from).collect(),
    }))
}

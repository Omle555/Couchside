//! Browsing endpoints: home, movies, shows, items, search, progress.

use super::auth::AuthUser;
use super::dto::{
    file_name_of, HomeDto, ItemDetailDto, ListQuery, MediaItemDto, Page, ProgressRequest,
    SearchDto, SearchQuery, SeasonDto, ShowDetailDto, ShowDto, SubtitleDto,
};
use super::{page, ApiError};
use crate::app::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use solidfin_core::{LibraryId, MediaId, MediaKind, ShowId, UserId};
use solidfin_db::{MediaItem, MovieSort};

const HOME_ROW: i64 = 24;

/// Containers and codecs that Chrome/Edge/Firefox can usually play without transcoding.
fn browser_playable(item: &MediaItem) -> bool {
    let ext = std::path::Path::new(&item.path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let container_ok = matches!(ext.as_str(), "mp4" | "m4v" | "webm" | "mkv" | "mov");
    let video_ok = item
        .video_codec
        .as_deref()
        .is_none_or(|c| matches!(c, "h264" | "vp8" | "vp9" | "av1" | "hevc"));
    let audio_ok = item
        .audio_codec
        .as_deref()
        .is_none_or(|c| matches!(c, "aac" | "mp3" | "opus" | "vorbis" | "flac"));
    container_ok && video_ok && audio_ok
}

async fn with_progress(
    state: &AppState,
    user: UserId,
    items: Vec<MediaItem>,
) -> Result<Vec<MediaItemDto>, ApiError> {
    let ids: Vec<MediaId> = items.iter().map(|i| i.id).collect();
    let progress = state.db.progress_map(user, &ids).await?;
    Ok(MediaItemDto::list(items, &progress))
}

/// `GET /home`
pub async fn home(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<HomeDto>, ApiError> {
    let continue_watching = state.db.continue_watching(user.id, HOME_ROW).await?;
    let recent_movies = state.db.recently_added(MediaKind::Movie, HOME_ROW).await?;
    let recent_episodes = state
        .db
        .recently_added(MediaKind::Episode, HOME_ROW)
        .await?;
    Ok(Json(HomeDto {
        continue_watching: with_progress(&state, user.id, continue_watching).await?,
        recent_movies: with_progress(&state, user.id, recent_movies).await?,
        recent_episodes: with_progress(&state, user.id, recent_episodes).await?,
    }))
}

/// `GET /movies?library_id=&sort=title|added|year&limit=&offset=`
pub async fn movies(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<MediaItemDto>>, ApiError> {
    let (limit, offset) = page(q.limit, q.offset);
    let sort = match q.sort.as_deref() {
        Some("added") => MovieSort::Added,
        Some("year") => MovieSort::Year,
        _ => MovieSort::Title,
    };
    let (items, total) = state
        .db
        .list_movies(q.library_id.map(LibraryId), sort, limit, offset)
        .await?;
    Ok(Json(Page {
        items: with_progress(&state, user.id, items).await?,
        total,
        limit,
        offset,
    }))
}

/// `GET /shows?library_id=&limit=&offset=`
pub async fn shows(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<ShowDto>>, ApiError> {
    let (limit, offset) = page(q.limit, q.offset);
    let (shows, total) = state
        .db
        .list_shows(q.library_id.map(LibraryId), limit, offset)
        .await?;
    Ok(Json(Page {
        items: shows.into_iter().map(ShowDto::from).collect(),
        total,
        limit,
        offset,
    }))
}

/// `GET /shows/{id}` — show with its episodes grouped by season.
pub async fn show(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ShowDetailDto>, ApiError> {
    let show = state
        .db
        .get_show(ShowId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let episodes = state.db.show_episodes(show.id).await?;
    let episodes = with_progress(&state, user.id, episodes).await?;

    let mut seasons: Vec<SeasonDto> = Vec::new();
    for ep in episodes {
        let number = ep.season.unwrap_or(0);
        match seasons.last_mut() {
            Some(season) if season.season == number => season.episodes.push(ep),
            _ => seasons.push(SeasonDto {
                season: number,
                episodes: vec![ep],
            }),
        }
    }
    // Specials (season 0) go last.
    seasons.sort_by_key(|s| if s.season == 0 { i64::MAX } else { s.season });
    Ok(Json(ShowDetailDto {
        show: ShowDto::from(show),
        seasons,
    }))
}

/// `GET /items/{id}`
pub async fn item(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ItemDetailDto>, ApiError> {
    let item = state
        .db
        .get_item(MediaId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let progress = state.db.get_progress(user.id, item.id).await?;
    let subtitles = state.db.subtitles_for(item.id).await?;
    let show_title = match item.show_id {
        Some(show_id) => state.db.get_show(show_id).await?.map(|s| s.title),
        None => None,
    };
    Ok(Json(ItemDetailDto {
        show_title,
        file_name: file_name_of(&item.path),
        file_size: item.file_size,
        browser_playable: browser_playable(&item),
        subtitles: subtitles.into_iter().map(SubtitleDto::from).collect(),
        item: MediaItemDto::new(item, progress.as_ref()),
    }))
}

/// `GET /search?q=`
pub async fn search(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(q): Query<SearchQuery>,
) -> Result<Json<SearchDto>, ApiError> {
    if q.q.trim().is_empty() {
        return Ok(Json(SearchDto {
            movies: Vec::new(),
            shows: Vec::new(),
        }));
    }
    let (movies, shows) = state.db.search(&q.q, 50).await?;
    Ok(Json(SearchDto {
        movies: with_progress(&state, user.id, movies).await?,
        shows: shows.into_iter().map(ShowDto::from).collect(),
    }))
}

/// `PUT /items/{id}/progress` — saves the resume position. Items are marked
/// watched automatically once 90% has been played.
pub async fn set_progress(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<ProgressRequest>,
) -> Result<StatusCode, ApiError> {
    if !req.position_seconds.is_finite() {
        return Err(ApiError::BadRequest("Invalid position.".into()));
    }
    let item = state
        .db
        .get_item(MediaId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let near_end = item
        .runtime_seconds
        .is_some_and(|runtime| runtime > 0.0 && req.position_seconds >= runtime * 0.9);
    let played = req.played.unwrap_or(near_end);
    let position = if played { 0.0 } else { req.position_seconds };
    state
        .db
        .set_progress(user.id, item.id, position, played)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

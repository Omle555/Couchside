//! TMDB settings, manual "Identify" for movies (with its title scan), and the thumbnail proxy.

use super::auth::{AdminUser, AuthUser};
use super::dto::{
    CandidateDto, IdentifyQuery, IdentifyRequest, MetadataSettingsDto, MetadataSettingsRequest,
    TitleScanDto, TitleScanFrameDto, TitleScanQuery, TmdbImageQuery,
};
use crate::title_scan::ScanPart;
use super::stream::safe_media_path;
use super::ApiError;
use crate::app::{effective_metadata_settings, AppState, KeySource};
use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use solidfin_core::MediaId;
use solidfin_db::{METADATA_LANGUAGE, TMDB_API_KEY};
use solidfin_metadata::{is_tmdb_image_path, Tmdb};

/// Most results shown in the Identify dialog.
const MAX_CANDIDATES: usize = 12;
/// Overview length shown in the Identify dialog.
const OVERVIEW_CHARS: usize = 220;

/// `en`, `en-AU`, `pt-BR`...
fn valid_language(lang: &str) -> bool {
    let (base, region) = match lang.split_once('-') {
        Some((b, r)) => (b, Some(r)),
        None => (lang, None),
    };
    base.len() == 2
        && base.chars().all(|c| c.is_ascii_lowercase())
        && region.is_none_or(|r| r.len() == 2 && r.chars().all(|c| c.is_ascii_uppercase()))
}

fn key_hint(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••{tail}")
}

async fn current_settings(state: &AppState) -> Result<MetadataSettingsDto, ApiError> {
    let (key, language) = effective_metadata_settings(&state.db, &state.config).await?;
    Ok(MetadataSettingsDto {
        has_key: key.is_some(),
        key_source: key.as_ref().map(|(_, source)| match source {
            KeySource::App => "app",
            KeySource::Config => "config",
        }),
        key_hint: key.as_ref().map(|(k, _)| key_hint(k)),
        language,
    })
}

/// `GET /settings/metadata`
pub async fn get_settings(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<MetadataSettingsDto>, ApiError> {
    Ok(Json(current_settings(&state).await?))
}

/// `PUT /settings/metadata` — saves the TMDB key (after checking it works) and language,
/// then starts fetching metadata.
pub async fn put_settings(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(req): Json<MetadataSettingsRequest>,
) -> Result<Json<MetadataSettingsDto>, ApiError> {
    let (_, old_language) = effective_metadata_settings(&state.db, &state.config).await?;

    let new_language = match req.language.as_deref().map(str::trim) {
        Some(lang) if !valid_language(lang) => {
            return Err(ApiError::BadRequest(
                "Language should look like en-US or en-AU.".into(),
            ))
        }
        Some(lang) => lang.to_string(),
        None => old_language.clone(),
    };

    if let Some(key) = req.tmdb_api_key.as_deref().map(str::trim) {
        if key.is_empty() {
            state.db.set_setting(TMDB_API_KEY, None).await?;
        } else {
            let candidate = Tmdb::new(key, &new_language, state.image_dir.to_path_buf())?;
            candidate.validate().await?;
            state.db.set_setting(TMDB_API_KEY, Some(key)).await?;
            tracing::info!("TMDB API key saved");
        }
    }
    if req.language.is_some() {
        state
            .db
            .set_setting(METADATA_LANGUAGE, Some(&new_language))
            .await?;
    }

    let (key, language) = effective_metadata_settings(&state.db, &state.config).await?;
    let client = match &key {
        Some((k, _)) => Some(Tmdb::new(k, &language, state.image_dir.to_path_buf())?),
        None => None,
    };
    let has_client = client.is_some();
    state.manager.set_tmdb(client);

    if language != old_language {
        // Titles and descriptions are language specific: fetch everything again.
        state.db.reset_metadata().await?;
    }
    if has_client {
        state.manager.trigger();
    }
    Ok(Json(current_settings(&state).await?))
}

fn shorten(text: Option<String>) -> Option<String> {
    text.map(|t| {
        if t.chars().count() > OVERVIEW_CHARS {
            let cut: String = t.chars().take(OVERVIEW_CHARS).collect();
            format!("{}…", cut.trim_end())
        } else {
            t
        }
    })
}

/// `GET /items/{id}/identify?q=&year=` — TMDB search results for the Identify dialog.
pub async fn identify_search(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
    Query(q): Query<IdentifyQuery>,
) -> Result<Json<Vec<CandidateDto>>, ApiError> {
    let lookup = state
        .db
        .movie_lookup(MediaId(id))
        .await?
        .ok_or_else(|| ApiError::BadRequest("Only movies can be identified.".into()))?;
    let tmdb = state.manager.tmdb().ok_or_else(|| {
        ApiError::BadRequest("Add a TMDB API key under Settings → Metadata first.".into())
    })?;

    let query =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or_else(|| lookup.parsed_title.clone(), str::to_string);
    let year = if q.q.is_some() {
        q.year
    } else {
        q.year.or(lookup.parsed_year)
    };

    let mut candidates = tmdb.search_movies(&query, year).await?;
    if candidates.is_empty() && year.is_some() {
        candidates = tmdb.search_movies(&query, None).await?;
    }
    Ok(Json(
        candidates
            .into_iter()
            .take(MAX_CANDIDATES)
            .map(|c| CandidateDto {
                current: lookup.tmdb_id == Some(c.tmdb_id),
                tmdb_id: c.tmdb_id,
                title: c.title,
                original_title: c.original_title,
                year: c.year,
                overview: shorten(c.overview),
                poster_path: c.poster_path,
                score: (c.score * 100.0).round() / 100.0,
            })
            .collect(),
    ))
}

/// `POST /items/{id}/identify` — use the chosen TMDB movie and keep it.
pub async fn identify_apply(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
    Json(req): Json<IdentifyRequest>,
) -> Result<StatusCode, ApiError> {
    if req.tmdb_id <= 0 {
        return Err(ApiError::BadRequest("Invalid TMDB id.".into()));
    }
    state
        .manager
        .identify_movie(MediaId(id), req.tmdb_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /items/{id}/title-scan?part=start|end` — reads the text on frames from the first
/// minutes or the end credits of a movie so the user can pick the title to search for.
/// Nothing is matched automatically.
pub async fn title_scan(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
    Query(query): Query<TitleScanQuery>,
) -> Result<Json<TitleScanDto>, ApiError> {
    let part = match query.part.as_deref() {
        None | Some("start") => ScanPart::Opening,
        Some("end") => ScanPart::Ending,
        Some(_) => return Err(ApiError::BadRequest("Unknown part of the video.".into())),
    };
    let id = MediaId(id);
    state
        .db
        .movie_lookup(id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("Only movies can be identified.".into()))?;
    let item = state.db.get_item(id).await?.ok_or(ApiError::NotFound)?;
    let path = safe_media_path(&state, &item.path).await?;
    let result = state
        .title_scanner
        .scan(id, &path, part, item.runtime_seconds)
        .await?;
    Ok(Json(TitleScanDto {
        frames: result
            .frames
            .into_iter()
            .enumerate()
            .map(|(index, f)| TitleScanFrameDto {
                index,
                time_seconds: (f.time_seconds * 10.0).round() / 10.0,
                lines: f.lines,
            })
            .collect(),
        suggestions: result.suggestions,
        ocr_available: result.ocr_available,
        frames_read: result.frames_read,
    }))
}

/// `GET /items/{id}/title-scan/frames/{index}` — a frame from the item's last title scan.
pub async fn title_scan_frame(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((id, index)): Path<(i64, usize)>,
) -> Result<Response, ApiError> {
    let jpeg = state
        .title_scanner
        .frame_image(MediaId(id), index)
        .ok_or(ApiError::NotFound)?;
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
            (CACHE_CONTROL, HeaderValue::from_static("private, no-store")),
        ],
        jpeg,
    )
        .into_response())
}

/// `GET /tmdb/image?path=/abc.jpg` — small TMDB poster for the Identify dialog,
/// fetched by the server so browsers never contact TMDB directly.
pub async fn tmdb_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<TmdbImageQuery>,
) -> Result<Response, ApiError> {
    if !is_tmdb_image_path(&q.path) {
        return Err(ApiError::NotFound);
    }
    let tmdb = state.manager.tmdb().ok_or(ApiError::NotFound)?;
    let bytes = tmdb.fetch_image(&q.path, "w185").await?;
    let content_type = if q.path.ends_with(".png") {
        "image/png"
    } else if q.path.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    };
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400"),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert!(valid_language("en"));
        assert!(valid_language("en-AU"));
        assert!(!valid_language("EN-au"));
        assert!(!valid_language("english"));
        assert!(!valid_language("en-AUS"));
    }

    #[test]
    fn hints() {
        assert_eq!(key_hint("abcdef123456"), "••••3456");
        assert_eq!(key_hint("ab"), "••••ab");
    }

    #[test]
    fn shortens_overview() {
        let long = "word ".repeat(100);
        let short = shorten(Some(long)).expect("some");
        assert!(short.chars().count() <= OVERVIEW_CHARS + 1);
        assert!(short.ends_with('…'));
    }
}

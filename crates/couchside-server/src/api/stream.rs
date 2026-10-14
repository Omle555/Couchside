//! File streaming (with HTTP Range support), artwork and subtitles.

use super::auth::AuthUser;
use super::ApiError;
use crate::app::AppState;
use crate::range::{parse_range, ByteRange};
use crate::subtitles::to_vtt;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use couchside_core::paths::resolve_within;
use couchside_core::{MediaId, ShowId, SubtitleId};
use std::io::SeekFrom;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const STREAM_BUFFER: usize = 256 * 1024;

/// MIME type for a media file extension.
fn video_content_type(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        // Chrome and Edge play most H.264/AAC MKV files when told they are WebM.
        "mkv" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "ts" | "m2ts" => "video/mp2t",
        "mpg" | "mpeg" => "video/mpeg",
        "flv" => "video/x-flv",
        "ogv" => "video/ogg",
        _ => "application/octet-stream",
    }
}

fn image_content_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    }
}

/// Resolves a stored media path and checks it is still inside a library folder.
pub(super) async fn safe_media_path(state: &AppState, stored: &str) -> Result<PathBuf, ApiError> {
    let roots: Vec<PathBuf> = state
        .db
        .list_libraries()
        .await?
        .into_iter()
        .map(|l| PathBuf::from(l.path))
        .collect();
    let candidate = PathBuf::from(stored);
    tokio::task::spawn_blocking(move || {
        resolve_within(&candidate, roots.iter().map(PathBuf::as_path))
    })
    .await?
    .ok_or(ApiError::NotFound)
}

/// Serves a file honouring `Range` requests so players can seek.
pub(super) async fn serve_file(
    path: &std::path::Path,
    headers: &HeaderMap,
    content_type: &'static str,
) -> Result<Response, ApiError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => ApiError::NotFound,
            _ => ApiError::internal(format!("open {}: {err}", path.display())),
        })?;
    let len = file.metadata().await.map_err(ApiError::internal)?.len();
    let range = parse_range(headers.get(RANGE).and_then(|v| v.to_str().ok()), len);

    let mut response = match range {
        ByteRange::Full => {
            let body = Body::from_stream(ReaderStream::with_capacity(file, STREAM_BUFFER));
            let mut r = Response::new(body);
            r.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from(len));
            r
        }
        ByteRange::Partial { start, end } => {
            file.seek(SeekFrom::Start(start))
                .await
                .map_err(ApiError::internal)?;
            let count = end - start + 1;
            let body =
                Body::from_stream(ReaderStream::with_capacity(file.take(count), STREAM_BUFFER));
            let mut r = Response::new(body);
            *r.status_mut() = StatusCode::PARTIAL_CONTENT;
            r.headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from(count));
            let content_range = HeaderValue::from_str(&format!("bytes {start}-{end}/{len}"))
                .map_err(ApiError::internal)?;
            r.headers_mut().insert(CONTENT_RANGE, content_range);
            r
        }
        ByteRange::Unsatisfiable => {
            let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            let content_range =
                HeaderValue::from_str(&format!("bytes */{len}")).map_err(ApiError::internal)?;
            r.headers_mut().insert(CONTENT_RANGE, content_range);
            return Ok(r);
        }
    };
    let h = response.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    Ok(response)
}

/// `GET /items/{id}/stream` — direct play of the original file.
pub async fn stream_item(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let item = state
        .db
        .get_item(MediaId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let path = safe_media_path(&state, &item.path).await?;
    serve_file(&path, &headers, video_content_type(&path)).await
}

/// Serves a cached artwork file by its stored name.
async fn serve_image(
    state: &AppState,
    file_name: Option<String>,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let name = file_name.ok_or(ApiError::NotFound)?;
    // Names are generated by Couchside, but never allow separators or `..`.
    let plain = std::path::Path::new(&name)
        .file_name()
        .and_then(|n| n.to_str())
        == Some(name.as_str());
    if !plain || name.contains("..") {
        return Err(ApiError::NotFound);
    }
    let path = state.image_dir.join(&name);
    let mut response = serve_file(&path, headers, image_content_type(&name)).await?;
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    Ok(response)
}

/// `GET /items/{id}/image/{poster|backdrop}` — episodes without a poster use their show's.
pub async fn item_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Path((id, kind)): Path<(i64, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let item = state
        .db
        .get_item(MediaId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let file = match kind.as_str() {
        "poster" => match (item.poster_file, item.show_id) {
            (Some(file), _) => Some(file),
            (None, Some(show_id)) => state
                .db
                .get_show(show_id)
                .await?
                .and_then(|s| s.poster_file),
            (None, None) => None,
        },
        "backdrop" => item.backdrop_file,
        _ => return Err(ApiError::NotFound),
    };
    serve_image(&state, file, &headers).await
}

/// `GET /shows/{id}/image/{poster|backdrop}`
pub async fn show_image(
    State(state): State<AppState>,
    _user: AuthUser,
    Path((id, kind)): Path<(i64, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let show = state
        .db
        .get_show(ShowId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let file = match kind.as_str() {
        "poster" => show.poster_file,
        "backdrop" => show.backdrop_file,
        _ => return Err(ApiError::NotFound),
    };
    serve_image(&state, file, &headers).await
}

/// `GET /subtitles/{id}` — an external subtitle converted to WebVTT.
pub async fn subtitle(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let sub = state
        .db
        .get_subtitle(SubtitleId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let path = safe_media_path(&state, &sub.path).await?;
    let bytes = tokio::fs::read(&path).await.map_err(ApiError::internal)?;
    let text = String::from_utf8_lossy(&bytes);
    let vtt = to_vtt(&sub.format, &text).ok_or(ApiError::NotFound)?;
    Ok((
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("text/vtt; charset=utf-8"),
            ),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=300"),
            ),
        ],
        vtt,
    )
        .into_response())
}

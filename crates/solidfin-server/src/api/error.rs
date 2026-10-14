//! API error type: consistent JSON bodies, no internal details leaked.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use solidfin_db::DbError;

/// Errors returned by API handlers.
#[derive(Debug)]
pub enum ApiError {
    /// 400 with a message for the user.
    BadRequest(String),
    /// 401: not logged in or session expired.
    Unauthorized,
    /// 403: logged in but not allowed.
    Forbidden,
    /// 403: the client is not on the local network.
    OutsideLocalNetwork,
    /// 404.
    NotFound,
    /// 409 with a message for the user.
    Conflict(String),
    /// 500: details are logged, never sent.
    Internal(String),
}

impl ApiError {
    /// Logs `err` and returns a generic 500.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self::Internal(err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Please sign in.".into(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "Only administrators can do that.".into(),
            ),
            Self::OutsideLocalNetwork => (
                StatusCode::FORBIDDEN,
                "outside_local_network",
                "Solidfin only accepts connections from devices on its home network.".into(),
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Not found.".into()),
            Self::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg),
            Self::Internal(detail) => {
                tracing::error!(error = %detail, "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Something went wrong. See the Solidfin log for details.".into(),
                )
            }
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}

impl From<DbError> for ApiError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::Conflict(msg) => Self::Conflict(msg),
            other => Self::internal(other),
        }
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(err: tokio::task::JoinError) -> Self {
        Self::internal(err)
    }
}

impl From<crate::manager::MetadataTaskError> for ApiError {
    fn from(err: crate::manager::MetadataTaskError) -> Self {
        use crate::manager::MetadataTaskError as E;
        match err {
            E::Db(db) => db.into(),
            E::Metadata(meta) => meta.into(),
            other => Self::BadRequest(capitalize(&other.to_string())),
        }
    }
}

impl From<solidfin_metadata::MetadataError> for ApiError {
    fn from(err: solidfin_metadata::MetadataError) -> Self {
        use solidfin_metadata::MetadataError as E;
        match err {
            E::Unauthorized => Self::BadRequest(err.to_string()),
            E::Http(_) | E::RateLimited => {
                Self::BadRequest(format!("Couldn't get data from TMDB right now ({err})."))
            }
            E::Io(_) => Self::internal(err),
        }
    }
}

impl From<crate::title_scan::TitleScanError> for ApiError {
    fn from(err: crate::title_scan::TitleScanError) -> Self {
        use crate::title_scan::TitleScanError as E;
        use solidfin_library::FrameError as F;
        match err {
            E::Busy => Self::Conflict(capitalize(&err.to_string())),
            E::NoFfmpeg | E::NoFrames | E::NoRuntime | E::Frames(F::Timeout) => {
                Self::BadRequest(capitalize(&err.to_string()))
            }
            E::Frames(F::Failed(_)) => {
                tracing::warn!(error = %err, "title scan could not read the video");
                Self::BadRequest("ffmpeg couldn't read this video file.".into())
            }
            E::Frames(F::Spawn(_)) | E::Join(_) => Self::internal(err),
        }
    }
}

pub(super) fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>() + ".",
        None => String::new(),
    }
}

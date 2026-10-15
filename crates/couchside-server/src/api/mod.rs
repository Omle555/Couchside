//! HTTP API (`/api/v1`) and the embedded web UI.

mod activity;
mod auth;
mod browse;
mod cleanup;
mod delete;
mod dto;
mod error;
mod libraries;
mod metadata;
mod playback;
mod setup;
mod stream;
mod users;

pub use error::ApiError;

use crate::app::AppState;
use crate::{lan, web};
use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;

/// Default page size for list endpoints.
pub const DEFAULT_LIMIT: i64 = 100;
/// Largest page size a client may request.
pub const MAX_LIMIT: i64 = 500;

/// Clamps client pagination parameters.
pub fn page(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (
        limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
        offset.unwrap_or(0).max(0),
    )
}

/// Builds the full application router.
pub fn router(state: AppState) -> Router {
    let lan_only = state.config.server.lan_only;
    let api = Router::new()
        .route("/health", get(setup::health))
        .route("/setup", get(setup::status).post(setup::create_admin))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/libraries", get(libraries::list).post(libraries::create))
        .route("/libraries/{id}", delete(libraries::remove))
        .route(
            "/scan",
            get(libraries::scan_status).post(libraries::start_scan),
        )
        .route("/metadata/refresh", post(libraries::refresh_metadata))
        .route(
            "/settings/library",
            get(libraries::get_settings).put(libraries::put_settings),
        )
        .route(
            "/settings/metadata",
            get(metadata::get_settings).put(metadata::put_settings),
        )
        .route(
            "/items/{id}/identify",
            get(metadata::identify_search).post(metadata::identify_apply),
        )
        .route(
            "/items/{id}/cleanup",
            get(cleanup::preview).post(cleanup::run),
        )
        .route("/items/{id}/delete", get(delete::preview).post(delete::run))
        .route("/items/{id}/title-scan", post(metadata::title_scan))
        .route(
            "/items/{id}/title-scan/frames/{index}",
            get(metadata::title_scan_frame),
        )
        .route("/tmdb/image", get(metadata::tmdb_image))
        .route("/activity", get(activity::get))
        .route("/home", get(browse::home))
        .route("/movies", get(browse::movies))
        .route("/shows", get(browse::shows))
        .route("/shows/{id}", get(browse::show))
        .route("/shows/{id}/image/{kind}", get(stream::show_image))
        .route("/search", get(browse::search))
        .route("/items/{id}", get(browse::item))
        .route("/items/{id}/stream", get(stream::stream_item))
        .route("/items/{id}/playback", post(playback::start))
        .route("/transcode/{session}/main.m3u8", get(playback::playlist))
        .route("/transcode/{session}/init.mp4", get(playback::init_segment))
        .route("/transcode/{session}/seg/{file}", get(playback::segment))
        .route("/transcode/{session}/stop", post(playback::stop))
        .route("/items/{id}/image/{kind}", get(stream::item_image))
        .route("/items/{id}/progress", put(browse::set_progress))
        .route("/subtitles/{id}", get(stream::subtitle))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", delete(users::remove))
        .route("/users/{id}/password", put(users::set_password))
        .fallback(|| async { ApiError::NotFound });

    let router = Router::new()
        .nest("/api/v1", api)
        .merge(web::router())
        .with_state(state);
    if lan_only {
        router.layer(middleware::from_fn(lan::lan_only))
    } else {
        router
    }
}

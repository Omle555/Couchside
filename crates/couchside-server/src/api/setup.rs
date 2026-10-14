//! Health check and first-run setup.

use super::auth::{hash_password, start_session, validate_credentials};
use super::dto::LoginRequest;
use super::ApiError;
use crate::app::AppState;
use axum::extract::State;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};

/// `GET /health`
pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

/// `GET /setup` — tells the web UI whether to show the first-run screen.
pub async fn status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let needs_setup = state.db.count_users().await? == 0;
    Ok(Json(json!({
        "needs_setup": needs_setup,
        "version": env!("CARGO_PKG_VERSION"),
        "ffprobe": state.manager.has_ffprobe(),
        "ffmpeg": state.title_scanner.available(),
        "tmdb": state.manager.has_tmdb(),
    })))
}

/// `POST /setup` — creates the first administrator. Only works while there are no users.
pub async fn create_admin(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    validate_credentials(&req.username, &req.password)?;
    let _guard = state.setup_lock.lock().await;
    if state.db.count_users().await? > 0 {
        return Err(ApiError::Forbidden);
    }
    let hash = hash_password(req.password).await?;
    let user = state
        .db
        .create_user(req.username.trim(), &hash, true)
        .await?;
    tracing::info!(user = %user.username, "created administrator account");
    start_session(&state, &user).await
}

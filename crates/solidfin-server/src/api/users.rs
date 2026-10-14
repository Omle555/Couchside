//! User management.

use super::auth::{hash_password, validate_credentials, AdminUser, AuthUser};
use super::dto::{CreateUserRequest, PasswordRequest, UserDto};
use super::ApiError;
use crate::app::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use solidfin_core::UserId;

/// `GET /users`
pub async fn list(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let users = state.db.list_users().await?;
    Ok(Json(users.iter().map(UserDto::from).collect()))
}

/// `POST /users`
pub async fn create(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    validate_credentials(&req.username, &req.password)?;
    let hash = hash_password(req.password).await?;
    let user = state
        .db
        .create_user(req.username.trim(), &hash, req.is_admin)
        .await?;
    tracing::info!(user = %user.username, admin = user.is_admin, "user created");
    Ok((StatusCode::CREATED, Json(UserDto::from(&user))))
}

/// `DELETE /users/{id}` — admins can't delete themselves or the last admin.
pub async fn remove(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let id = UserId(id);
    if id == admin.id {
        return Err(ApiError::BadRequest(
            "You can't delete your own account.".into(),
        ));
    }
    let target = state.db.get_user(id).await?.ok_or(ApiError::NotFound)?;
    if target.is_admin && state.db.count_admins().await? <= 1 {
        return Err(ApiError::BadRequest(
            "Solidfin needs at least one administrator.".into(),
        ));
    }
    state.db.delete_user(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /users/{id}/password` — users can change their own; admins can change anyone's.
pub async fn set_password(
    State(state): State<AppState>,
    AuthUser(caller): AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<PasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let id = UserId(id);
    if id != caller.id && !caller.is_admin {
        return Err(ApiError::Forbidden);
    }
    let target = state.db.get_user(id).await?.ok_or(ApiError::NotFound)?;
    validate_credentials(&target.username, &req.password)?;
    let hash = hash_password(req.password).await?;
    state.db.set_password(id, &hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

//! Password hashing, session tokens and the authentication extractors.

use super::dto::{LoginRequest, UserDto};
use super::ApiError;
use crate::app::AppState;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::extract::{FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use couchside_db::User;
use rand::RngCore;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Cookie holding the session token.
pub const SESSION_COOKIE: &str = "couchside_session";
/// Sessions unused for this long are discarded (30 days).
pub const SESSION_IDLE_SECS: i64 = 30 * 24 * 60 * 60;

/// Hashes a password with Argon2id on a blocking thread.
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        let mut salt_bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut salt_bytes);
        let salt = SaltString::encode_b64(&salt_bytes).map_err(ApiError::internal)?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(ApiError::internal)
    })
    .await?
}

/// Checks a password against a stored hash on a blocking thread.
pub async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    Ok(tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await?)
}

/// Validates new credentials.
pub fn validate_credentials(username: &str, password: &str) -> Result<(), ApiError> {
    let name = username.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::BadRequest(
            "Username must be 1 to 64 characters.".into(),
        ));
    }
    if password.chars().count() < 6 {
        return Err(ApiError::BadRequest(
            "Password must be at least 6 characters.".into(),
        ));
    }
    Ok(())
}

/// SHA-256 of a token, as stored in the database.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Creates a session for `user` and returns a response that sets the cookie.
pub async fn start_session(state: &AppState, user: &User) -> Result<Response, ApiError> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    state
        .db
        .create_session(&hash_token(&token), user.id)
        .await?;

    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_IDLE_SECS}"
    );
    let mut response = Json(json!({ "token": token, "user": UserDto::from(user) })).into_response();
    let value = HeaderValue::from_str(&cookie).map_err(ApiError::internal)?;
    response.headers_mut().insert(SET_COOKIE, value);
    Ok(response)
}

/// Reads the session token from the cookie or an `Authorization: Bearer` header.
fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(bearer) = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(bearer.trim().to_string());
    }
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_string())
}

/// Any signed-in user.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_headers(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let user = state
            .db
            .user_for_session(&hash_token(&token), SESSION_IDLE_SECS)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        Ok(Self(user))
    }
}

/// A signed-in administrator.
pub struct AdminUser(pub User);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if user.is_admin {
            Ok(Self(user))
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

/// `POST /auth/login`
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let user = state.db.user_by_username(req.username.trim()).await?;
    // Verify against a dummy hash when the user is missing so timing doesn't reveal names.
    let (hash, user) = match user {
        Some(u) => (u.password_hash.clone(), Some(u)),
        None => (
            "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$J1v2bMxbDdKqkmzJvhfqxwbW0r1ty/ff9g0wdOJXF0Q"
                .to_string(),
            None,
        ),
    };
    let ok = verify_password(req.password, hash).await?;
    match user {
        Some(user) if ok => {
            tracing::info!(user = %user.username, "signed in");
            start_session(&state, &user).await
        }
        _ => Err(ApiError::BadRequest("Wrong username or password.".into())),
    }
}

/// `POST /auth/logout`
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = token_from_headers(&headers) {
        state.db.delete_session(&hash_token(&token)).await?;
    }
    let mut response = Json(json!({ "ok": true })).into_response();
    let expired = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    let value = HeaderValue::from_str(&expired).map_err(ApiError::internal)?;
    response.headers_mut().insert(SET_COOKIE, value);
    Ok(response)
}

/// `GET /auth/me`
pub async fn me(AuthUser(user): AuthUser) -> Json<UserDto> {
    Json(UserDto::from(&user))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cookie_and_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("a=1; couchside_session=abc; b=2"),
        );
        assert_eq!(token_from_headers(&headers).as_deref(), Some("abc"));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer xyz"));
        assert_eq!(token_from_headers(&headers).as_deref(), Some("xyz"));
    }

    #[tokio::test]
    async fn password_round_trip() {
        let hash = hash_password("hunter22".into()).await.expect("hash");
        assert!(verify_password("hunter22".into(), hash.clone())
            .await
            .expect("verify"));
        assert!(!verify_password("wrong".into(), hash).await.expect("verify"));
    }
}

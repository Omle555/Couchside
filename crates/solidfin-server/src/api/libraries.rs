//! Library management and scan control.

use super::auth::{AdminUser, AuthUser};
use super::dto::{CreateLibraryRequest, LibraryDto, LibrarySettingsDto};
use super::ApiError;
use crate::app::{renames_allowed, AppState};
use crate::manager::ScanStatus;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use solidfin_core::LibraryId;
use solidfin_db::ALLOW_FILE_RENAMES;

/// `GET /libraries`
pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<LibraryDto>>, ApiError> {
    let libs = state.db.list_libraries().await?;
    Ok(Json(libs.into_iter().map(LibraryDto::from).collect()))
}

/// Normalises a user-entered folder path and checks it exists.
async fn validate_folder(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim().trim_matches('"');
    // Keep `D:\` as is, but drop trailing separators elsewhere.
    let normalized = if trimmed.len() > 3 {
        trimmed.trim_end_matches(['\\', '/'])
    } else {
        trimmed
    };
    if normalized.is_empty() {
        return Err(ApiError::BadRequest("Please enter a folder path.".into()));
    }
    let path = std::path::PathBuf::from(normalized);
    if !path.is_absolute() {
        return Err(ApiError::BadRequest(
            "Use a full path, like D:\\Movies or \\\\NAS\\Video\\Movies.".into(),
        ));
    }
    let is_dir = tokio::fs::metadata(&path)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false);
    if !is_dir {
        return Err(ApiError::BadRequest(format!(
            "Solidfin can't open the folder {normalized}. Check the path and that it is reachable from this PC."
        )));
    }
    Ok(normalized.to_string())
}

/// `POST /libraries` — adds a library and starts scanning it.
pub async fn create(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(req): Json<CreateLibraryRequest>,
) -> Result<(StatusCode, Json<LibraryDto>), ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "Please give the library a name.".into(),
        ));
    }
    let path = validate_folder(&req.path).await?;
    let library = state.db.create_library(name, req.kind, &path).await?;
    tracing::info!(name, path = %path, "library added");
    state.manager.trigger();
    Ok((StatusCode::CREATED, Json(LibraryDto::from(library))))
}

/// `DELETE /libraries/{id}` — forgets a library (files on disk are untouched).
pub async fn remove(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if state.db.delete_library(LibraryId(id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// `GET /scan`
pub async fn scan_status(State(state): State<AppState>, _user: AuthUser) -> Json<ScanStatus> {
    Json(state.manager.status())
}

/// `POST /scan` — rescans all libraries now.
pub async fn start_scan(State(state): State<AppState>, _admin: AdminUser) -> Json<Value> {
    let started = state.manager.trigger();
    Json(json!({ "started": started }))
}

/// `POST /metadata/refresh` — re-downloads all online metadata.
pub async fn refresh_metadata(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Value>, ApiError> {
    state.db.reset_metadata().await?;
    let started = state.manager.trigger();
    Ok(Json(json!({ "started": started })))
}

/// `GET /settings/library`
pub async fn get_settings(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<LibrarySettingsDto>, ApiError> {
    Ok(Json(LibrarySettingsDto {
        allow_file_renames: renames_allowed(&state.db, &state.config).await?,
    }))
}

/// `PUT /settings/library` — saved in the database, so it overrides config.toml.
pub async fn put_settings(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(req): Json<LibrarySettingsDto>,
) -> Result<Json<LibrarySettingsDto>, ApiError> {
    let value = if req.allow_file_renames {
        "true"
    } else {
        "false"
    };
    state
        .db
        .set_setting(ALLOW_FILE_RENAMES, Some(value))
        .await?;
    tracing::info!(
        allow_file_renames = req.allow_file_renames,
        "library settings saved"
    );
    get_settings(State(state), admin).await
}

#[cfg(test)]
mod tests {
    use crate::app::renames_allowed;
    use solidfin_core::Config;
    use solidfin_db::{Db, ALLOW_FILE_RENAMES};

    #[tokio::test]
    async fn settings_switch_overrides_config() {
        let db = Db::open_in_memory().await.expect("db");
        let mut config = Config::default();
        assert!(!renames_allowed(&db, &config).await.expect("q"));
        config.library.allow_file_renames = true;
        assert!(renames_allowed(&db, &config).await.expect("q"));
        db.set_setting(ALLOW_FILE_RENAMES, Some("false"))
            .await
            .expect("set");
        assert!(!renames_allowed(&db, &config).await.expect("q"));
        config.library.allow_file_renames = false;
        db.set_setting(ALLOW_FILE_RENAMES, Some("true"))
            .await
            .expect("set");
        assert!(renames_allowed(&db, &config).await.expect("q"));
    }
}

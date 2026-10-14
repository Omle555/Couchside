//! "Delete" on a movie's page: sends the movie's folder (or, in a folder shared with other
//! movies, just its own files) to the Recycle Bin and removes it from Couchside. Only when
//! file changes are allowed in Settings, and only the exact paths the user was shown.

use super::auth::AdminUser;
use super::cleanup::{log_moves, movie_target, relative};
use super::dto::{DeleteMovieRequest, DeletePreviewDto};
use super::error::capitalize;
use super::ApiError;
use crate::app::{renames_allowed, AppState};
use crate::recycle::recycle;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use couchside_core::MediaId;
use couchside_library::{plan_movie_delete, DeletePlan};
use std::path::{Path as FsPath, PathBuf};

const NOT_ALLOWED: &str =
    "Deleting movies is switched off. Turn on \"Allow file renames and deletes\" under Settings → Libraries.";
const BUSY: &str =
    "Couchside is scanning your libraries right now. Try again when the scan has finished.";

async fn plan(item_path: &str, root: &FsPath) -> Result<DeletePlan, ApiError> {
    let video = PathBuf::from(item_path);
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || plan_movie_delete(&video, &root))
        .await?
        .map_err(|err| ApiError::BadRequest(capitalize(&err.to_string())))
}

fn preview_dto(plan: &DeletePlan, root: &FsPath) -> DeletePreviewDto {
    match plan {
        DeletePlan::Folder(dir) => DeletePreviewDto {
            folder: Some(relative(dir, root)),
            files: Vec::new(),
        },
        DeletePlan::Files(files) => DeletePreviewDto {
            folder: None,
            files: files.iter().map(|f| relative(f, root)).collect(),
        },
    }
}

/// `GET /items/{id}/delete` — what deleting the movie would remove. Changes nothing.
pub async fn preview(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<DeletePreviewDto>, ApiError> {
    if !renames_allowed(&state.db, &state.config).await? {
        return Err(ApiError::BadRequest(NOT_ALLOWED.into()));
    }
    let target = movie_target(&state, MediaId(id)).await?;
    let plan = plan(&target.item.path, &target.root).await?;
    Ok(Json(preview_dto(&plan, &target.root)))
}

/// `POST /items/{id}/delete` — deletes exactly what the preview showed (the request repeats
/// it; if anything changed since, nothing is deleted) and removes the movie from Couchside.
pub async fn run(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
    Json(req): Json<DeleteMovieRequest>,
) -> Result<StatusCode, ApiError> {
    if !renames_allowed(&state.db, &state.config).await? {
        return Err(ApiError::BadRequest(NOT_ALLOWED.into()));
    }
    let _guard = state
        .manager
        .try_exclusive()
        .ok_or_else(|| ApiError::Conflict(BUSY.into()))?;
    let id = MediaId(id);
    let target = movie_target(&state, id).await?;
    let plan = plan(&target.item.path, &target.root).await?;
    let shown = preview_dto(&plan, &target.root);
    if shown.folder != req.folder || shown.files != req.files {
        return Err(ApiError::Conflict(
            "The files changed since you opened Delete. Nothing was deleted; please try again."
                .into(),
        ));
    }

    let paths = plan.paths();
    let log = state.config.data_dir().join("logs").join("renames.log");
    let (done, result) = tokio::task::spawn_blocking(move || {
        let mut done = Vec::new();
        let mut result = Ok(());
        for path in &paths {
            if let Err(err) = recycle(path) {
                result = Err(err);
                break;
            }
            done.push((path.clone(), PathBuf::from("Recycle Bin")));
        }
        log_moves(&log, id, "deleted", &done);
        (done, result)
    })
    .await?;
    if let Err(err) = result {
        tracing::warn!(item = %id, error = %err, deleted = done.len(), "could not delete a movie");
        return Err(ApiError::BadRequest(format!(
            "{} Close any player using this movie and try again.",
            capitalize(&err.to_string())
        )));
    }

    state.db.delete_media(&[id]).await?;
    for name in [&target.item.poster_file, &target.item.backdrop_file]
        .into_iter()
        .flatten()
    {
        if FsPath::new(name)
            .file_name()
            .is_some_and(|n| n == name.as_str())
        {
            let _ = tokio::fs::remove_file(state.image_dir.join(name)).await;
        }
    }
    tracing::info!(item = %id, path = %target.item.path, "movie deleted");
    Ok(StatusCode::NO_CONTENT)
}

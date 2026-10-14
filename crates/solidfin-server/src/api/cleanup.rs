//! "Clean up" on a movie's page: deletes the movie's stored details and artwork and builds
//! them again from its current TMDB match. When `allow_file_renames` is switched on it can
//! also rename the file to `Title (Year)/Title (Year) 1080p.ext`, but only to the exact name
//! the user was shown first.

use super::auth::AdminUser;
use super::dto::{CleanupPreviewDto, CleanupRequest, CleanupResultDto, RenameStepDto};
use super::error::capitalize;
use super::stream::safe_media_path;
use super::ApiError;
use crate::app::{renames_allowed, AppState};
use crate::manager::MetadataTaskError;
use axum::extract::{Path, State};
use axum::Json;
use solidfin_core::naming::parse_movie;
use solidfin_core::standard_names::{movie_file_stem, movie_folder_name, resolution_label};
use solidfin_core::{now_unix, MediaId, MediaKind};
use solidfin_db::{MediaItem, RenamedFile};
use solidfin_library::{
    execute_rename, move_extras, plan_movie_rename, removable_extras, undo_rename, RenameError,
    RenamePlan,
};
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};

const BUSY: &str =
    "Solidfin is scanning your libraries right now. Try Clean up again when the scan has finished.";

/// The movie being cleaned up.
pub(super) struct Target {
    pub(super) item: MediaItem,
    /// Resolved file on disk (for ffprobe).
    pub(super) file: PathBuf,
    /// Its library folder, as stored.
    pub(super) root: PathBuf,
}

pub(super) async fn movie_target(state: &AppState, id: MediaId) -> Result<Target, ApiError> {
    let item = state.db.get_item(id).await?.ok_or(ApiError::NotFound)?;
    if item.kind != MediaKind::Movie {
        return Err(ApiError::BadRequest(
            "Only movies can be cleaned up.".into(),
        ));
    }
    let file = safe_media_path(state, &item.path).await?;
    let library = state
        .db
        .get_library(item.library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Target {
        item,
        file,
        root: PathBuf::from(library.path),
    })
}

/// Checks Clean up can run (TMDB key, a match) before anything is changed.
async fn check_identified(state: &AppState, id: MediaId) -> Result<(), ApiError> {
    if !state.manager.has_tmdb() {
        return Err(MetadataTaskError::NoTmdb.into());
    }
    let lookup = state.db.movie_lookup(id).await?;
    if lookup.and_then(|l| l.tmdb_id).is_none() {
        return Err(MetadataTaskError::NotIdentified.into());
    }
    Ok(())
}

/// A path relative to the library folder, as shown to the user.
pub(super) fn relative(path: &FsPath, root: &FsPath) -> String {
    match path.strip_prefix(root) {
        Ok(rest) => rest.display().to_string(),
        Err(_) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

/// Works out the standard name for the movie from `title`, `year` and its resolution,
/// leaving out the files in `exclude` (about to be removed).
async fn rename_plan(
    target: &Target,
    title: &str,
    year: Option<i64>,
    exclude: Vec<PathBuf>,
) -> Result<Result<Option<RenamePlan>, RenameError>, ApiError> {
    let resolution = resolution_label(target.item.width, target.item.height);
    let (Some(folder), Some(stem)) = (
        movie_folder_name(title, year),
        movie_file_stem(title, year, resolution.as_deref()),
    ) else {
        return Ok(Ok(None));
    };
    let video = PathBuf::from(&target.item.path);
    let root = target.root.clone();
    Ok(tokio::task::spawn_blocking(move || {
        plan_movie_rename(&video, &root, &folder, &stem, &exclude)
    })
    .await?)
}

/// The images and subtitles Clean up would move out of the movie's folder.
async fn extras(target: &Target) -> Result<Result<Vec<PathBuf>, RenameError>, ApiError> {
    let video = PathBuf::from(&target.item.path);
    let root = target.root.clone();
    Ok(tokio::task::spawn_blocking(move || removable_extras(&video, &root)).await?)
}

fn rename_note(err: &RenameError) -> String {
    format!("The file can't be renamed: {err}.")
}

/// `GET /items/{id}/cleanup` — what Clean up would do, including the new file name and the
/// images and subtitles it would remove when file changes are allowed. Changes nothing
/// except refreshing the file's media info.
pub async fn preview(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
) -> Result<Json<CleanupPreviewDto>, ApiError> {
    let id = MediaId(id);
    let mut target = movie_target(&state, id).await?;
    check_identified(&state, id).await?;
    let mut dto = CleanupPreviewDto {
        renames_enabled: renames_allowed(&state.db, &state.config).await?,
        current_name: None,
        new_name: None,
        other_steps: Vec::new(),
        rename_note: None,
        remove_files: Vec::new(),
    };
    if !dto.renames_enabled {
        return Ok(Json(dto));
    }

    state.manager.refresh_media_info(id, &target.file).await?;
    target.item = state.db.get_item(id).await?.ok_or(ApiError::NotFound)?;
    let (title, year) = state.manager.matched_title(id).await?;
    dto.current_name = Some(relative(FsPath::new(&target.item.path), &target.root));
    let removals = match extras(&target).await? {
        Ok(files) => files,
        Err(err) => {
            dto.rename_note = Some(format!("The folder can't be read: {err}."));
            return Ok(Json(dto));
        }
    };
    dto.remove_files = removals.iter().map(|f| relative(f, &target.root)).collect();
    match rename_plan(&target, &title, year, removals).await? {
        Ok(Some(plan)) => {
            dto.new_name = Some(relative(&plan.new_video, &target.root));
            dto.other_steps = plan
                .moves
                .iter()
                .filter(|m| m.to != plan.new_video)
                .map(|m| RenameStepDto {
                    from: relative(&m.from, &target.root),
                    to: relative(&m.to, &target.root),
                })
                .collect();
        }
        Ok(None) => dto.rename_note = Some("The file already has its standard name.".into()),
        Err(err) => dto.rename_note = Some(rename_note(&err)),
    }
    Ok(Json(dto))
}

/// `POST /items/{id}/cleanup` — rebuilds the movie's details. When file changes are allowed
/// it then moves out the listed images and subtitles (only ones the preview would list too)
/// and renames the file if `rename_to` is still the new name.
pub async fn run(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<i64>,
    Json(req): Json<CleanupRequest>,
) -> Result<Json<CleanupResultDto>, ApiError> {
    let id = MediaId(id);
    let _guard = state
        .manager
        .try_exclusive()
        .ok_or_else(|| ApiError::Conflict(BUSY.into()))?;
    let target = movie_target(&state, id).await?;
    check_identified(&state, id).await?;

    state.manager.refresh_media_info(id, &target.file).await?;
    let old_images = state.manager.rebuild_movie(id).await?;
    let item = state.db.get_item(id).await?.ok_or(ApiError::NotFound)?;
    remove_unused_images(&state, &item, old_images).await;
    let target = Target { item, ..target };

    if !renames_allowed(&state.db, &state.config).await? {
        return Ok(Json(CleanupResultDto {
            renamed: false,
            message: "Details and artwork rebuilt from TMDB.".into(),
        }));
    }

    let mut notes = vec!["Details rebuilt.".to_string()];
    let wanted_removals = req.remove_files.unwrap_or_default();
    if !wanted_removals.is_empty() {
        let removable = extras(&target)
            .await?
            .map_err(|err| ApiError::BadRequest(capitalize(&err.to_string())))?;
        let chosen: Vec<PathBuf> = removable
            .into_iter()
            .filter(|f| wanted_removals.contains(&relative(f, &target.root)))
            .collect();
        let count = remove_extras(&state, &target, chosen).await?;
        notes.push(format!(
            "Removed {count} image and subtitle file{}.",
            if count == 1 { "" } else { "s" }
        ));
    }

    let mut renamed = false;
    if let Some(wanted) = req.rename_to {
        match rename_plan(&target, &target.item.title, target.item.year, Vec::new()).await? {
            Ok(Some(plan)) => {
                let new_name = relative(&plan.new_video, &target.root);
                if new_name == wanted {
                    rename(&state, &target, plan).await?;
                    renamed = true;
                    notes.push(format!("Renamed to {new_name}."));
                } else {
                    notes.push(format!(
                        "The new name would now be {new_name}, so the file was not renamed. \
                         Open Clean up again to check it."
                    ));
                }
            }
            Ok(None) => notes.push("The file already has its standard name.".into()),
            Err(err) => notes.push(rename_note(&err)),
        }
    }
    Ok(Json(CleanupResultDto {
        renamed,
        message: notes.join(" "),
    }))
}

/// Moves images and subtitles out of the library into `<data>\removed\<time>-item<id>`,
/// unlinks the subtitles and logs each move. Returns how many files were moved.
async fn remove_extras(
    state: &AppState,
    target: &Target,
    files: Vec<PathBuf>,
) -> Result<usize, ApiError> {
    if files.is_empty() {
        return Ok(0);
    }
    let id = target.item.id;
    let data = state.config.data_dir();
    let dest = data
        .join("removed")
        .join(format!("{}-item{}", now_unix(), id));
    let log = data.join("logs").join("renames.log");
    let root = target.root.clone();
    let (moved, result) = tokio::task::spawn_blocking(move || {
        let mut moved = Vec::new();
        let result = move_extras(&files, &root, &dest, &mut moved);
        log_moves(&log, id, "removed", &moved);
        (moved, result)
    })
    .await?;
    let gone: Vec<String> = moved
        .iter()
        .map(|(from, _)| from.to_string_lossy().into_owned())
        .collect();
    state.db.delete_subtitles(id, &gone).await?;
    tracing::info!(item = %id, files = moved.len(), "Clean up moved images and subtitles out of the library");
    result.map_err(|err| ApiError::BadRequest(capitalize(&err.to_string())))?;
    Ok(moved.len())
}

/// Renames the files, records the new paths, and writes the rename to `logs\renames.log`.
/// If the database can't be updated, the files are put back.
async fn rename(state: &AppState, target: &Target, plan: RenamePlan) -> Result<(), ApiError> {
    let id = target.item.id;
    let subtitles: Vec<(String, String)> = state
        .db
        .subtitle_paths(id)
        .await?
        .into_iter()
        .map(|old| {
            let new = plan
                .map_path(FsPath::new(&old))
                .to_string_lossy()
                .into_owned();
            (old, new)
        })
        .filter(|(old, new)| old != new)
        .collect();
    let renamed = renamed_file(&plan.new_video);

    let log = state.config.data_dir().join("logs").join("renames.log");
    let blocking_plan = plan.clone();
    let moves: Vec<(PathBuf, PathBuf)> = plan
        .moves
        .iter()
        .map(|m| (m.from.clone(), m.to.clone()))
        .collect();
    tokio::task::spawn_blocking(move || {
        execute_rename(&blocking_plan)?;
        log_moves(&log, id, "renamed", &moves);
        Ok::<(), RenameError>(())
    })
    .await?
    .map_err(|err| ApiError::BadRequest(capitalize(&err.to_string())))?;

    if let Err(err) = state.db.rename_media_file(id, &renamed, &subtitles).await {
        tracing::error!(item = %id, error = %err, "could not record a rename; putting the files back");
        tokio::task::spawn_blocking(move || undo_rename(&plan)).await?;
        return Err(err.into());
    }
    tracing::info!(item = %id, to = %renamed.path, "movie file renamed by Clean up");
    Ok(())
}

/// The new path with the name parsed from it, the same way the scanner would read it.
fn renamed_file(new_video: &FsPath) -> RenamedFile {
    let stem = new_video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = new_video
        .parent()
        .and_then(FsPath::file_name)
        .map(|n| n.to_string_lossy().into_owned());
    let parsed = parse_movie(&stem, folder.as_deref());
    RenamedFile {
        path: new_video.to_string_lossy().into_owned(),
        parsed_title: parsed.title,
        parsed_year: parsed.year.map(i64::from),
        hint_tmdb_id: parsed.ids.tmdb,
        hint_imdb_id: parsed.ids.imdb,
    }
}

/// Appends one line per move (time, action, item, old path, new path) so it can be undone
/// by hand.
pub(super) fn log_moves(log: &FsPath, id: MediaId, action: &str, moves: &[(PathBuf, PathBuf)]) {
    if moves.is_empty() {
        return;
    }
    let write = || -> std::io::Result<()> {
        if let Some(dir) = log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?;
        let time = now_unix();
        for (from, to) in moves {
            writeln!(
                file,
                "{time}\t{action}\titem {id}\t{}\t->\t{}",
                from.display(),
                to.display()
            )?;
        }
        Ok(())
    };
    if let Err(err) = write() {
        tracing::warn!(error = %err, "could not write renames.log");
    }
}

/// Deletes artwork files the movie no longer uses.
pub(super) async fn remove_unused_images(state: &AppState, item: &MediaItem, old: Vec<String>) {
    let in_use = [item.poster_file.as_deref(), item.backdrop_file.as_deref()];
    for name in old {
        let plain = FsPath::new(&name)
            .file_name()
            .is_some_and(|n| n == name.as_str());
        if !plain || in_use.contains(&Some(name.as_str())) {
            continue;
        }
        match tokio::fs::remove_file(state.image_dir.join(&name)).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(file = %name, error = %err, "could not delete old artwork"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_file_reads_like_the_scanner() {
        let path = PathBuf::from("/m/Runner (2026)/Runner (2026) 1080p.mkv");
        let renamed = renamed_file(&path);
        assert_eq!(renamed.parsed_title, "Runner");
        assert_eq!(renamed.parsed_year, Some(2026));
        assert_eq!(renamed.path, path.to_string_lossy());
    }

    #[test]
    fn relative_names() {
        let root = FsPath::new("/m");
        assert_eq!(
            relative(FsPath::new("/m/Runner (2026)/Runner.mkv"), root),
            PathBuf::from("Runner (2026)")
                .join("Runner.mkv")
                .display()
                .to_string()
        );
        assert_eq!(
            relative(FsPath::new("/elsewhere/Runner.mkv"), root),
            "Runner.mkv"
        );
    }
}

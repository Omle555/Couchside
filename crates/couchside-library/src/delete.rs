//! Working out what the "Delete" button removes for a movie. Changes nothing itself; the
//! server sends the listed paths to the Recycle Bin.

use crate::rename::{file_names, is_own_folder, sidecar_renames, RenameError};
use couchside_core::paths::starts_with_path;
use std::path::{Path, PathBuf};

/// What deleting a movie removes from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletePlan {
    /// The movie's own folder, with everything in it.
    Folder(PathBuf),
    /// The video and the files named after it, when its folder holds other movies too
    /// (or it sits in the library root).
    Files(Vec<PathBuf>),
}

impl DeletePlan {
    /// Every path that will be removed.
    pub fn paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Folder(dir) => vec![dir.clone()],
            Self::Files(files) => files.clone(),
        }
    }
}

/// Works out what deleting `video` removes. Never reaches outside `root`, never removes the
/// library root itself, and never removes another movie.
pub fn plan_movie_delete(video: &Path, root: &Path) -> Result<DeletePlan, RenameError> {
    if !starts_with_path(video, root) || video == root {
        return Err(RenameError::OutsideLibrary);
    }
    let dir = video.parent().ok_or(RenameError::OutsideLibrary)?;
    if is_own_folder(dir, root) {
        return Ok(DeletePlan::Folder(dir.to_path_buf()));
    }
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = video
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let others: Vec<String> = file_names(dir)?
        .into_iter()
        .filter(|n| *n != name)
        .collect();
    let mut files = vec![video.to_path_buf()];
    files.extend(
        sidecar_renames(&others, &stem, &stem)
            .into_iter()
            .map(|(n, _)| dir.join(n)),
    );
    Ok(DeletePlan::Files(files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("couchside-delete-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"x").expect("write");
    }

    #[test]
    fn own_folder_goes_whole() {
        let lib = temp("own");
        let video = lib.join("Runner (2026)").join("Runner (2026).mkv");
        touch(&video);
        touch(&lib.join("Runner (2026)").join("poster.jpg"));
        touch(&lib.join("Other (2001)").join("Other (2001).mkv"));
        assert_eq!(
            plan_movie_delete(&video, &lib).expect("plan"),
            DeletePlan::Folder(lib.join("Runner (2026)"))
        );
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn shared_folder_keeps_other_movies() {
        let lib = temp("shared");
        let video = lib.join("Runner.mkv");
        for f in [
            "Runner.mkv",
            "Runner.en.srt",
            "Runner.nfo",
            "Other.mkv",
            "Other.srt",
            "folder.jpg",
        ] {
            touch(&lib.join(f));
        }
        let DeletePlan::Files(mut files) = plan_movie_delete(&video, &lib).expect("plan") else {
            panic!("expected files");
        };
        files.sort();
        assert_eq!(
            files,
            vec![
                lib.join("Runner.en.srt"),
                lib.join("Runner.mkv"),
                lib.join("Runner.nfo")
            ]
        );
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn refuses_outside_paths() {
        let lib = temp("outside");
        assert!(plan_movie_delete(Path::new("/elsewhere/x.mkv"), &lib).is_err());
        let _ = std::fs::remove_dir_all(&lib);
    }
}

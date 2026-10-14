//! "Clean up" removals: the images and subtitle files that came with a movie. They are moved
//! out of the library into Couchside's own `removed` folder (not deleted), so they can be put
//! back by hand. Video files are never touched.

use crate::rename::{file_names, io_error, is_own_folder, sidecar_renames, RenameError};
use couchside_core::naming::{is_ignored_name, is_subtitle_file, is_video_file};
use couchside_core::paths::starts_with_path;
use std::path::{Path, PathBuf};

/// Image file extensions removed by Clean up.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tbn"];
/// Subtitle extensions removed by Clean up (more than the scanner reads).
const EXTRA_SUBTITLE_EXTENSIONS: &[&str] = &["sub", "idx", "sup"];
/// How deep Clean up looks inside a movie's own folder (`Subs\English\...`).
const MAX_DEPTH: usize = 3;

/// True for image and subtitle files; never for videos.
pub fn is_removable_extra(path: &Path) -> bool {
    if is_video_file(path) {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    is_subtitle_file(path)
        || IMAGE_EXTENSIONS.contains(&ext.as_str())
        || EXTRA_SUBTITLE_EXTENSIONS.contains(&ext.as_str())
}

fn collect(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) -> Result<(), RenameError> {
    for entry in std::fs::read_dir(dir).map_err(io_error(dir))? {
        let entry = entry.map_err(io_error(dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = dir.join(&name);
        if kind.is_dir() && depth < MAX_DEPTH && !is_ignored_name(&name) {
            collect(&path, depth + 1, found)?;
        } else if kind.is_file() && is_removable_extra(&path) {
            found.push(path);
        }
    }
    Ok(())
}

/// The image and subtitle files Clean up would remove for `video`. Reads folders, changes
/// nothing.
///
/// In a movie's own folder that is every image and subtitle in it (and its subfolders). In a
/// folder shared with other movies it is only the ones named after this movie, so the other
/// movies keep theirs.
pub fn removable_extras(video: &Path, root: &Path) -> Result<Vec<PathBuf>, RenameError> {
    let dir = video.parent().ok_or(RenameError::OutsideLibrary)?;
    if !starts_with_path(dir, root) {
        return Err(RenameError::OutsideLibrary);
    }
    let mut found = Vec::new();
    if is_own_folder(dir, root) {
        collect(dir, 0, &mut found)?;
    } else {
        let stem = video
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let names = file_names(dir)?;
        found = sidecar_renames(&names, &stem, &stem)
            .into_iter()
            .map(|(name, _)| dir.join(name))
            .filter(|p| is_removable_extra(p))
            .collect();
    }
    found.sort();
    Ok(found)
}

/// Moves `files` from the library into `dest`, keeping their path below `root`, and adds
/// each move to `done`. Stops at the first failure; files already moved stay in `dest`.
pub fn move_extras(
    files: &[PathBuf],
    root: &Path,
    dest: &Path,
    done: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), RenameError> {
    for file in files {
        let rel = file
            .strip_prefix(root)
            .map_err(|_| RenameError::OutsideLibrary)?;
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io_error(parent))?;
        }
        if std::fs::rename(file, &target).is_err() {
            // Different drive: copy, then remove the original.
            std::fs::copy(file, &target).map_err(io_error(file))?;
            std::fs::remove_file(file).map_err(io_error(file))?;
        }
        done.push((file.clone(), target));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("couchside-extras-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"x").expect("write");
    }

    #[test]
    fn kinds() {
        assert!(is_removable_extra(Path::new("a/poster.JPG")));
        assert!(is_removable_extra(Path::new("a/movie.en.srt")));
        assert!(is_removable_extra(Path::new("a/movie.idx")));
        assert!(!is_removable_extra(Path::new("a/movie.mkv")));
        assert!(!is_removable_extra(Path::new("a/movie.nfo")));
    }

    #[test]
    fn own_folder_loses_all_images_and_subtitles() {
        let lib = temp("own");
        let dir = lib.join("runner");
        let video = dir.join("runner.mkv");
        for f in [
            "runner.mkv",
            "poster.jpg",
            "fanart.png",
            "runner.en.srt",
            "Subs/2_English.srt",
            "movie.nfo",
        ] {
            touch(&dir.join(f));
        }
        let found = removable_extras(&video, &lib).expect("extras");
        assert_eq!(found.len(), 4);
        assert!(!found.iter().any(|p| p == &video));
        assert!(!found.iter().any(|p| p.ends_with("movie.nfo")));

        let other_root = temp("own-dest");
        let mut moved = Vec::new();
        move_extras(&found, &lib, &other_root, &mut moved).expect("move");
        assert_eq!(moved.len(), 4);
        assert!(video.exists());
        assert!(dir.join("movie.nfo").exists());
        assert!(!dir.join("poster.jpg").exists());
        assert!(other_root
            .join("runner")
            .join("Subs")
            .join("2_English.srt")
            .exists());
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&other_root);
    }

    #[test]
    fn shared_folder_keeps_other_movies_files() {
        let lib = temp("shared");
        let video = lib.join("Runner.mkv");
        for f in [
            "Runner.mkv",
            "Runner.en.srt",
            "Runner-poster.jpg",
            "Other.mkv",
            "Other.srt",
            "folder.jpg",
        ] {
            touch(&lib.join(f));
        }
        let found = removable_extras(&video, &lib).expect("extras");
        assert_eq!(
            found,
            vec![lib.join("Runner-poster.jpg"), lib.join("Runner.en.srt")]
        );
        let _ = std::fs::remove_dir_all(&lib);
    }
}

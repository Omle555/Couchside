//! "Clean up" renames: moving a movie file (and the subtitle, NFO and artwork files named
//! after it) to `Movie Title (Year)/Movie Title (Year) 1080p.ext`.
//!
//! Only used when `allow_file_renames` is switched on in config.toml. Never overwrites a
//! file, never leaves the library folder, and puts everything back if a step fails.

use crate::walk::walk;
use couchside_core::naming::is_video_file;
use couchside_core::paths::starts_with_path;
use std::path::{Path, PathBuf};

/// Endings of artwork files named after a video, e.g. `Movie-poster.jpg`.
const ARTWORK_SUFFIXES: &[&str] = &[
    "-poster",
    "-fanart",
    "-backdrop",
    "-thumb",
    "-banner",
    "-logo",
    "-landscape",
    "-clearart",
    "-disc",
];

/// One rename. When `from` is a folder, everything inside it moves too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMove {
    /// Current path.
    pub from: PathBuf,
    /// New path.
    pub to: PathBuf,
}

/// The renames needed to give a movie its standard name, in the order they run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    /// Folder to create before moving anything into it.
    pub create_dir: Option<PathBuf>,
    /// Renames in order; later steps use the paths left by earlier ones.
    pub moves: Vec<FileMove>,
    /// Where the video ends up.
    pub new_video: PathBuf,
}

impl RenamePlan {
    /// Where `path` ends up after this plan runs (unchanged if the plan doesn't touch it).
    pub fn map_path(&self, path: &Path) -> PathBuf {
        map_path(path, &self.moves)
    }
}

/// Why a rename can't happen.
#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    /// The movie or its new name would be outside the library folder.
    #[error("the movie is not inside its library folder")]
    OutsideLibrary,
    /// Something already has the new name; Couchside never overwrites files.
    #[error("\"{0}\" already exists, so nothing was renamed")]
    AlreadyExists(String),
    /// Reading the folder or renaming failed (file in use, no permission...).
    #[error("could not rename \"{name}\": {source}")]
    Io {
        /// File or folder name involved.
        name: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(crate) fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> RenameError + '_ {
    move |source| RenameError::Io {
        name: display_name(path),
        source,
    }
}

/// Same name, ignoring case on Windows (where `a.mkv` and `A.mkv` are one file).
fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// Applies `moves` in order to a path, following folder renames.
pub fn map_path(path: &Path, moves: &[FileMove]) -> PathBuf {
    let mut current = path.to_path_buf();
    for m in moves {
        if current == m.from {
            current = m.to.clone();
        } else if let Ok(rest) = current.strip_prefix(&m.from) {
            current = m.to.join(rest);
        }
    }
    current
}

/// Files next to a video that belong to it, with their new names:
/// `Old.en.srt` → `New.en.srt`, `Old.nfo` → `New.nfo`, `Old-poster.jpg` → `New-poster.jpg`.
///
/// `names` are the other file names in the folder. Files that belong to a different video
/// whose name starts the same way (`Old.Part2.mkv` and its `Old.Part2.srt`) are left alone.
pub fn sidecar_renames(names: &[String], old_stem: &str, new_stem: &str) -> Vec<(String, String)> {
    let other_video_stems: Vec<&str> = names
        .iter()
        .filter(|n| is_video_file(Path::new(n.as_str())))
        .filter_map(|n| n.rsplit_once('.').map(|(stem, _)| stem))
        .filter(|stem| !same_name(stem, old_stem))
        .collect();
    names
        .iter()
        .filter(|name| !is_video_file(Path::new(name.as_str())))
        .filter_map(|name| {
            let prefix = name.get(..old_stem.len())?;
            let rest = name.get(old_stem.len()..)?;
            if !prefix.eq_ignore_ascii_case(old_stem) {
                return None;
            }
            let belongs = rest.starts_with('.')
                || ARTWORK_SUFFIXES
                    .iter()
                    .any(|s| rest.to_ascii_lowercase().starts_with(&format!("{s}.")));
            let other_owner = other_video_stems.iter().any(|stem| {
                stem.len() > old_stem.len()
                    && name
                        .get(..stem.len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(stem))
            });
            (belongs && !other_owner).then(|| (name.clone(), format!("{new_stem}{rest}")))
        })
        .collect()
}

/// Lists the plain file names in a folder.
pub(crate) fn file_names(dir: &Path) -> Result<Vec<String>, RenameError> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io_error(dir))? {
        let entry = entry.map_err(io_error(dir))?;
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

/// True if `dir` is a movie's own folder: inside the library (not its root) and holding
/// exactly one video, counting subfolders.
pub(crate) fn is_own_folder(dir: &Path, root: &Path) -> bool {
    if dir.components().count() <= root.components().count() {
        return false;
    }
    match walk(dir) {
        Ok(found) => found.unreadable_dirs == 0 && found.videos.len() == 1,
        Err(_) => false,
    }
}

/// Fails if `to` exists, unless it is `from` itself spelled with different capitals.
fn check_free(from: &Path, to: &Path) -> Result<(), RenameError> {
    let exists = to.try_exists().map_err(io_error(to))?;
    let case_only = from.parent() == to.parent()
        && same_name(&display_name(from), &display_name(to))
        && from != to;
    if exists && !case_only {
        return Err(RenameError::AlreadyExists(display_name(to)));
    }
    Ok(())
}

/// Works out the renames that give `video` its standard name. Returns `None` when it
/// already has it. Reads the folder but changes nothing.
///
/// A movie in its own folder keeps that folder, renamed to `folder_name`. A movie sharing
/// a folder with other videos (or sitting in the library root) gets a new `folder_name`
/// folder next to it. Files in `exclude` (about to be removed) are left out of the plan.
pub fn plan_movie_rename(
    video: &Path,
    root: &Path,
    folder_name: &str,
    file_stem: &str,
    exclude: &[PathBuf],
) -> Result<Option<RenamePlan>, RenameError> {
    let canonical_video = video.canonicalize().map_err(io_error(video))?;
    let canonical_root = root.canonicalize().map_err(io_error(root))?;
    if !starts_with_path(&canonical_video, &canonical_root) || canonical_video == canonical_root {
        return Err(RenameError::OutsideLibrary);
    }
    let (Some(dir), Some(old_name), Some(old_stem)) = (
        video.parent(),
        video.file_name().map(|n| n.to_string_lossy().into_owned()),
        video.file_stem().map(|n| n.to_string_lossy().into_owned()),
    ) else {
        return Err(RenameError::OutsideLibrary);
    };
    let new_name = match video.extension() {
        Some(ext) => format!("{file_stem}.{}", ext.to_string_lossy().to_ascii_lowercase()),
        None => file_stem.to_string(),
    };

    let siblings: Vec<String> = file_names(dir)?
        .into_iter()
        .filter(|n| *n != old_name && !exclude.contains(&dir.join(n)))
        .collect();
    let mut renames = vec![(old_name, new_name.clone())];
    renames.extend(sidecar_renames(&siblings, &old_stem, file_stem));

    let mut create_dir = None;
    let mut moves = Vec::new();
    if is_own_folder(dir, root) {
        let mut folder = dir.to_path_buf();
        if display_name(dir) != folder_name {
            let parent = dir.parent().ok_or(RenameError::OutsideLibrary)?;
            folder = parent.join(folder_name);
            check_free(dir, &folder)?;
            moves.push(FileMove {
                from: dir.to_path_buf(),
                to: folder.clone(),
            });
        }
        for (old, new) in &renames {
            if old != new {
                check_free(&dir.join(old), &dir.join(new))?;
                moves.push(FileMove {
                    from: folder.join(old),
                    to: folder.join(new),
                });
            }
        }
    } else {
        let folder = dir.join(folder_name);
        if !folder.try_exists().map_err(io_error(&folder))? {
            create_dir = Some(folder.clone());
        }
        for (old, new) in &renames {
            let to = folder.join(new);
            check_free(&dir.join(old), &to)?;
            moves.push(FileMove {
                from: dir.join(old),
                to,
            });
        }
    }

    if moves.is_empty() {
        return Ok(None);
    }
    let new_video = map_path(video, &moves);
    if !starts_with_path(&new_video, root) {
        return Err(RenameError::OutsideLibrary);
    }
    Ok(Some(RenamePlan {
        create_dir,
        moves,
        new_video,
    }))
}

/// Carries out a plan. If any step fails, the steps already done are reversed and the
/// error is returned, so files end up either all renamed or all where they were.
pub fn execute_rename(plan: &RenamePlan) -> Result<(), RenameError> {
    if let Some(dir) = &plan.create_dir {
        std::fs::create_dir(dir).map_err(io_error(dir))?;
    }
    for (done, m) in plan.moves.iter().enumerate() {
        let result = check_free(&m.from, &m.to)
            .and_then(|()| std::fs::rename(&m.from, &m.to).map_err(io_error(&m.from)));
        if let Err(err) = result {
            undo_moves(&plan.moves[..done]);
            if let Some(dir) = &plan.create_dir {
                // Only removes the folder if it is still empty.
                let _ = std::fs::remove_dir(dir);
            }
            return Err(err);
        }
    }
    Ok(())
}

/// Puts files back after a plan ran (used if the database could not be updated).
pub fn undo_rename(plan: &RenamePlan) {
    undo_moves(&plan.moves);
    if let Some(dir) = &plan.create_dir {
        let _ = std::fs::remove_dir(dir);
    }
}

fn undo_moves(moves: &[FileMove]) {
    for m in moves.iter().rev() {
        if let Err(err) = std::fs::rename(&m.to, &m.from) {
            tracing::error!(
                from = %m.to.display(),
                to = %m.from.display(),
                error = %err,
                "could not undo a rename; move this file back by hand"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sidecars_follow_the_video() {
        let found = sidecar_renames(
            &names(&[
                "runner.en.srt",
                "Runner.nfo",
                "Runner-poster.jpg",
                "Runner-notes.txt",
                "Other.srt",
                "poster.jpg",
                "Runner.Part2.mkv",
                "Runner.Part2.en.srt",
            ]),
            "Runner",
            "Runner (2026) 1080p",
        );
        assert_eq!(
            found,
            vec![
                ("runner.en.srt".into(), "Runner (2026) 1080p.en.srt".into()),
                ("Runner.nfo".into(), "Runner (2026) 1080p.nfo".into()),
                (
                    "Runner-poster.jpg".into(),
                    "Runner (2026) 1080p-poster.jpg".into()
                ),
            ]
        );
    }

    #[test]
    fn paths_follow_folder_renames() {
        let moves = vec![
            FileMove {
                from: PathBuf::from("/lib/runner"),
                to: PathBuf::from("/lib/Runner (2026)"),
            },
            FileMove {
                from: PathBuf::from("/lib/Runner (2026)/runner.mkv"),
                to: PathBuf::from("/lib/Runner (2026)/Runner (2026) 1080p.mkv"),
            },
        ];
        assert_eq!(
            map_path(Path::new("/lib/runner/runner.mkv"), &moves),
            PathBuf::from("/lib/Runner (2026)/Runner (2026) 1080p.mkv")
        );
        assert_eq!(
            map_path(Path::new("/lib/runner/Subs/en.srt"), &moves),
            PathBuf::from("/lib/Runner (2026)/Subs/en.srt")
        );
        assert_eq!(
            map_path(Path::new("/lib/other.mkv"), &moves),
            PathBuf::from("/lib/other.mkv")
        );
    }

    /// A temporary library folder removed when the test ends.
    struct TempLib(PathBuf);

    impl TempLib {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("couchside-rename-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp library");
            Self(dir)
        }

        fn file(&self, rel: &str, body: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(&path, body).expect("write");
            path
        }
    }

    impl Drop for TempLib {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn own_folder_is_renamed_with_its_files() {
        let lib = TempLib::new("own");
        let video = lib.file("runner 2026 web/runner.2026.web.mkv", "video");
        lib.file("runner 2026 web/runner.2026.web.en.srt", "subs");
        lib.file("runner 2026 web/poster.jpg", "art");

        let plan = plan_movie_rename(&video, &lib.0, "Runner (2026)", "Runner (2026) 1080p", &[])
            .expect("plan")
            .expect("something to do");
        assert_eq!(plan.create_dir, None);
        assert_eq!(
            plan.new_video,
            lib.0.join("Runner (2026)").join("Runner (2026) 1080p.mkv")
        );
        execute_rename(&plan).expect("rename");

        let folder = lib.0.join("Runner (2026)");
        assert_eq!(
            std::fs::read_to_string(&plan.new_video).expect("video"),
            "video"
        );
        assert!(folder.join("Runner (2026) 1080p.en.srt").exists());
        assert!(folder.join("poster.jpg").exists());
        assert!(!lib.0.join("runner 2026 web").exists());

        // Already standard: nothing more to do.
        assert_eq!(
            plan_movie_rename(
                &plan.new_video,
                &lib.0,
                "Runner (2026)",
                "Runner (2026) 1080p",
                &[]
            )
            .expect("plan"),
            None
        );

        undo_rename(&plan);
        assert!(video.exists());
        assert!(!folder.exists());
    }

    #[test]
    fn shared_folder_gets_a_new_folder() {
        let lib = TempLib::new("shared");
        let video = lib.file("Runner.mkv", "video");
        lib.file("Runner.nfo", "nfo");
        lib.file("Other Movie (2001).mkv", "other");

        let plan = plan_movie_rename(&video, &lib.0, "Runner (2026)", "Runner (2026) 720p", &[])
            .expect("plan")
            .expect("something to do");
        assert_eq!(plan.create_dir, Some(lib.0.join("Runner (2026)")));
        execute_rename(&plan).expect("rename");
        assert!(lib.0.join("Runner (2026)/Runner (2026) 720p.mkv").exists());
        assert!(lib.0.join("Runner (2026)/Runner (2026) 720p.nfo").exists());
        assert!(lib.0.join("Other Movie (2001).mkv").exists());
    }

    #[test]
    fn never_overwrites() {
        let lib = TempLib::new("exists");
        let video = lib.file("Runner.mkv", "video");
        lib.file("Other.mkv", "other");
        lib.file("Runner (2026)/Runner (2026) 1080p.mkv", "someone else");

        let err = plan_movie_rename(&video, &lib.0, "Runner (2026)", "Runner (2026) 1080p", &[])
            .expect_err("target exists");
        assert!(matches!(err, RenameError::AlreadyExists(_)));
        assert_eq!(std::fs::read_to_string(&video).expect("video"), "video");
    }

    #[test]
    fn failed_step_puts_everything_back() {
        let lib = TempLib::new("rollback");
        let video = lib.file("Runner.mkv", "video");
        lib.file("Runner.nfo", "nfo");
        lib.file("Other.mkv", "other");
        let plan = plan_movie_rename(&video, &lib.0, "Runner (2026)", "Runner (2026)", &[])
            .expect("plan")
            .expect("something to do");
        // Something takes the subtitle's new name after planning.
        lib.file("Runner (2026)/Runner (2026).nfo", "late arrival");
        let plan = RenamePlan {
            create_dir: None,
            ..plan
        };
        assert!(execute_rename(&plan).is_err());
        assert_eq!(
            std::fs::read_to_string(&video).expect("video back"),
            "video"
        );
        assert!(lib.0.join("Runner.nfo").exists());
    }

    #[test]
    fn refuses_files_outside_the_library() {
        let lib = TempLib::new("outside");
        let other = TempLib::new("outside-other");
        let video = other.file("Runner.mkv", "video");
        assert!(matches!(
            plan_movie_rename(&video, &lib.0, "Runner (2026)", "Runner (2026)", &[]),
            Err(RenameError::OutsideLibrary)
        ));
    }
}

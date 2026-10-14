//! Incremental library scanning.

use crate::walk::{walk, walk_root_files, FoundVideo, WalkResult, SUBTITLE_DIRS};
use couchside_core::naming::{
    is_ignored_name, is_subtitle_file, is_video_file, parse_episode, parse_movie, parse_show_folder,
};
use couchside_core::{LibraryKind, MediaId, MediaKind, ShowId};
use couchside_db::{Db, DbError, Library, NewMedia, NewSubtitle};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Counts reported after a scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanStats {
    /// New or changed files written to the database.
    pub added_or_updated: usize,
    /// Files whose size and modified time were unchanged.
    pub unchanged: usize,
    /// Database entries removed because their file is gone.
    pub removed: usize,
    /// Video files that could not be understood (e.g. no SxxEyy in a show library).
    pub skipped: usize,
}

/// Errors that abort a library scan.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The library root could not be read (drive offline, share unavailable...).
    #[error("library folder {path} is not reachable: {source}")]
    RootUnavailable {
        /// The library root.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A database error.
    #[error(transparent)]
    Db(#[from] DbError),
    /// The blocking walk task panicked or was cancelled.
    #[error("scan task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// The part of a library touched by a change on disk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScanTarget {
    /// A folder directly inside the library root (a movie or show folder) and everything in it.
    Folder(PathBuf),
    /// The video and subtitle files lying directly in the library root.
    RootFiles,
}

impl ScanTarget {
    /// The target a changed path under `root` belongs to. `None` for the root itself,
    /// paths outside it and names the scanner ignores.
    pub fn for_path(root: &Path, changed: &Path) -> Option<Self> {
        let relative = changed.strip_prefix(root).ok()?;
        let mut components = relative.components();
        let first = components.next()?;
        let name = first.as_os_str().to_string_lossy();
        if is_ignored_name(&name) {
            return None;
        }
        let top = root.join(first.as_os_str());
        let loose_file =
            components.next().is_none() && (is_video_file(&top) || is_subtitle_file(&top));
        Some(if loose_file {
            Self::RootFiles
        } else {
            Self::Folder(top)
        })
    }

    /// Whether a stored file path lies inside this target of `root`.
    fn contains(&self, root: &Path, path: &str) -> bool {
        let path = Path::new(path);
        match self {
            Self::Folder(folder) => starts_with_nocase(path, folder),
            Self::RootFiles => {
                let Some(parent) = path.parent() else {
                    return false;
                };
                let in_subtitle_dir = parent
                    .file_name()
                    .is_some_and(|n| SUBTITLE_DIRS.iter().any(|d| n == *d));
                same_path(parent, root)
                    || in_subtitle_dir && parent.parent().is_some_and(|p| same_path(p, root))
            }
        }
    }
}

/// Case-insensitive path equality (Windows file names ignore case).
fn same_path(a: &Path, b: &Path) -> bool {
    starts_with_nocase(a, b) && a.components().count() == b.components().count()
}

/// Case-insensitive `Path::starts_with`.
fn starts_with_nocase(path: &Path, prefix: &Path) -> bool {
    let mut path = path.components();
    prefix.components().all(|p| {
        path.next().is_some_and(|c| {
            c.as_os_str().to_string_lossy().to_lowercase()
                == p.as_os_str().to_string_lossy().to_lowercase()
        })
    })
}

/// Scans one library: adds new files, updates changed ones, links subtitles and
/// removes entries for deleted files. Unchanged files cost one lookup each.
pub async fn scan_library(db: &Db, library: &Library) -> Result<ScanStats, ScanError> {
    scan(db, library, None).await
}

/// Scans only `target` of a library after files there were added, changed, renamed or
/// removed. Entries elsewhere in the library are left alone.
pub async fn scan_target(
    db: &Db,
    library: &Library,
    target: &ScanTarget,
) -> Result<ScanStats, ScanError> {
    scan(db, library, Some(target)).await
}

/// Reads the files of the whole library (`None`) or one target of it.
pub(crate) fn walk_scope(root: &Path, target: Option<&ScanTarget>) -> std::io::Result<WalkResult> {
    match target {
        None => walk(root),
        Some(ScanTarget::RootFiles) => walk_root_files(root),
        Some(ScanTarget::Folder(folder)) => {
            // An offline drive must fail, not look like a removed folder.
            std::fs::metadata(root)?;
            match std::fs::metadata(folder) {
                Ok(meta) if meta.is_dir() => walk(folder),
                Ok(_) => Ok(WalkResult::default()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(WalkResult::default()),
                Err(err) => Err(err),
            }
        }
    }
}

async fn scan(
    db: &Db,
    library: &Library,
    target: Option<&ScanTarget>,
) -> Result<ScanStats, ScanError> {
    let root = PathBuf::from(&library.path);
    let walk_root = root.clone();
    let walk_target = target.cloned();
    let walked = tokio::task::spawn_blocking(move || walk_scope(&walk_root, walk_target.as_ref()))
        .await?
        .map_err(|source| ScanError::RootUnavailable {
            path: library.path.clone(),
            source,
        })?;

    let mut videos_per_dir: HashMap<PathBuf, usize> = HashMap::new();
    for video in &walked.videos {
        if let Some(parent) = video.path.parent() {
            *videos_per_dir.entry(parent.to_path_buf()).or_default() += 1;
        }
    }

    let mut stats = ScanStats::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut show_ids: HashMap<PathBuf, ShowId> = HashMap::new();

    for video in &walked.videos {
        let Some(path_str) = video.path.to_str() else {
            tracing::warn!(path = %video.path.display(), "skipping file with an invalid Unicode name");
            stats.skipped += 1;
            continue;
        };
        let entry = match library.kind {
            LibraryKind::Movies => Some(movie_entry(library, &root, video, path_str)),
            LibraryKind::Shows => {
                episode_entry(db, library, &root, video, path_str, &mut show_ids).await?
            }
        };
        let Some(entry) = entry else {
            tracing::debug!(
                path = path_str,
                "could not find season/episode numbers; skipping"
            );
            stats.skipped += 1;
            continue;
        };
        seen.insert(path_str.to_lowercase());

        let id = match db.file_state(path_str).await? {
            Some(state)
                if state.size == entry.file_size
                    && state.mtime == entry.file_mtime
                    && !state.needs_reparse =>
            {
                stats.unchanged += 1;
                state.id
            }
            _ => {
                stats.added_or_updated += 1;
                db.upsert_media(&entry).await?
            }
        };

        let parent = video.path.parent().unwrap_or(&root);
        let only_video = videos_per_dir.get(parent).copied().unwrap_or(0) == 1;
        sync_subtitles(db, id, &video.path, only_video, &walked.subtitles).await?;
    }

    if walked.unreadable_dirs > 0 {
        tracing::warn!(
            library = %library.name,
            folders = walked.unreadable_dirs,
            "some folders could not be read; not removing missing items this time"
        );
    } else {
        let gone: Vec<MediaId> = db
            .library_media_paths(library.id)
            .await?
            .into_iter()
            .filter(|(_, path)| {
                target.is_none_or(|t| t.contains(&root, path))
                    && !seen.contains(&path.to_lowercase())
            })
            .map(|(id, _)| id)
            .collect();
        if !gone.is_empty() {
            db.delete_media(&gone).await?;
        }
        stats.removed = gone.len();
        db.delete_empty_shows(library.id).await?;
    }

    tracing::info!(library = %library.name, ?stats, "scan finished");
    Ok(stats)
}

fn movie_entry(library: &Library, root: &Path, video: &FoundVideo, path_str: &str) -> NewMedia {
    let stem = video
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent_name = video
        .path
        .parent()
        .filter(|p| *p != root)
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let parsed = parse_movie(&stem, parent_name.as_deref());
    NewMedia {
        library_id: library.id,
        kind: MediaKind::Movie,
        path: path_str.to_string(),
        file_size: i64::try_from(video.size).unwrap_or(i64::MAX),
        file_mtime: video.mtime,
        parsed_title: parsed.title,
        parsed_year: parsed.year.map(i64::from),
        show_id: None,
        season: None,
        episode: None,
        episode_end: None,
        hint_tmdb_id: parsed.ids.tmdb,
        hint_imdb_id: parsed.ids.imdb,
    }
}

async fn episode_entry(
    db: &Db,
    library: &Library,
    root: &Path,
    video: &FoundVideo,
    path_str: &str,
    show_ids: &mut HashMap<PathBuf, ShowId>,
) -> Result<Option<NewMedia>, DbError> {
    let Ok(relative) = video.path.strip_prefix(root) else {
        return Ok(None);
    };
    let mut components = relative.components();
    let (Some(show_component), Some(_)) = (components.next(), components.next()) else {
        // Episode files must live inside a show folder.
        return Ok(None);
    };
    let Some(stem) = video
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
    else {
        return Ok(None);
    };
    let Some(parsed) = parse_episode(&stem) else {
        return Ok(None);
    };

    let show_folder = root.join(show_component.as_os_str());
    let show_id = match show_ids.get(&show_folder) {
        Some(id) => *id,
        None => {
            let folder_name = show_component.as_os_str().to_string_lossy();
            let show = parse_show_folder(&folder_name);
            let id = db
                .upsert_show(
                    library.id,
                    &show_folder.to_string_lossy(),
                    &show.title,
                    show.year.map(i64::from),
                )
                .await?;
            show_ids.insert(show_folder, id);
            id
        }
    };

    let title = parsed
        .title
        .unwrap_or_else(|| format!("Episode {}", parsed.episode));
    Ok(Some(NewMedia {
        library_id: library.id,
        kind: MediaKind::Episode,
        path: path_str.to_string(),
        file_size: i64::try_from(video.size).unwrap_or(i64::MAX),
        file_mtime: video.mtime,
        parsed_title: title,
        parsed_year: None,
        show_id: Some(show_id),
        season: Some(i64::from(parsed.season)),
        episode: Some(i64::from(parsed.episode)),
        episode_end: parsed.episode_end.map(i64::from),
        hint_tmdb_id: None,
        hint_imdb_id: None,
    }))
}

/// Updates the subtitle links for one video if they changed on disk.
async fn sync_subtitles(
    db: &Db,
    media_id: MediaId,
    video: &Path,
    only_video_in_dir: bool,
    subtitles: &HashMap<PathBuf, Vec<PathBuf>>,
) -> Result<(), DbError> {
    let dir = video.parent().unwrap_or(video);
    let mut candidates: Vec<&PathBuf> = subtitles
        .get(dir)
        .map(|v| v.iter().collect())
        .unwrap_or_default();
    if only_video_in_dir {
        for sub_dir in SUBTITLE_DIRS {
            if let Some(found) = subtitles.get(&dir.join(sub_dir)) {
                candidates.extend(found.iter());
            }
        }
    }
    let mut found = match_subtitles(video, &candidates, only_video_in_dir);
    found.sort();
    found.dedup();
    let mut existing = db.subtitle_paths(media_id).await?;
    existing.sort_unstable();
    let mut new_paths: Vec<&str> = found.iter().map(|s| s.path.as_str()).collect();
    new_paths.sort_unstable();
    if existing
        .iter()
        .map(String::as_str)
        .ne(new_paths.iter().copied())
    {
        db.replace_subtitles(media_id, &found).await?;
    }
    Ok(())
}

/// Picks subtitle files belonging to `video`:
/// `Movie.srt`, `Movie.en.srt`, `Movie.en.forced.srt`, or any subtitle when the
/// video is alone in its folder (`English.srt`, `Subs/2_English.srt`).
pub(crate) fn match_subtitles(
    video: &Path,
    candidates: &[&PathBuf],
    only_video_in_dir: bool,
) -> Vec<NewSubtitle> {
    let Some(video_stem) = video.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let video_lower = video_stem.to_lowercase();
    let mut out = Vec::new();
    for sub in candidates {
        let (Some(sub_stem), Some(ext), Some(path)) = (
            sub.file_stem().and_then(|s| s.to_str()),
            sub.extension().and_then(|e| e.to_str()),
            sub.to_str(),
        ) else {
            continue;
        };
        let sub_lower = sub_stem.to_lowercase();
        let language = if sub_lower == video_lower {
            None
        } else if sub_lower.starts_with(&video_lower)
            && sub_lower[video_lower.len()..].starts_with('.')
        {
            let skip = video_stem.chars().count() + 1;
            Some(sub_stem.chars().skip(skip).collect::<String>())
        } else if only_video_in_dir {
            Some(sub_stem.to_string())
        } else {
            continue;
        };
        out.push(NewSubtitle {
            path: path.to_string(),
            language: language.filter(|l| !l.is_empty()),
            format: ext.to_ascii_lowercase(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use couchside_core::LibraryKind;

    fn subs(video: &str, files: &[&str], only: bool) -> Vec<(String, Option<String>)> {
        let paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        let refs: Vec<&PathBuf> = paths.iter().collect();
        match_subtitles(Path::new(video), &refs, only)
            .into_iter()
            .map(|s| (s.path, s.language))
            .collect()
    }

    #[test]
    fn subtitles_match_by_stem() {
        let found = subs(
            "/m/Heat (1995).mkv",
            &[
                "/m/Heat (1995).srt",
                "/m/Heat (1995).en.srt",
                "/m/Other.srt",
            ],
            false,
        );
        assert_eq!(
            found,
            vec![
                ("/m/Heat (1995).srt".to_string(), None),
                ("/m/Heat (1995).en.srt".to_string(), Some("en".to_string())),
            ]
        );
    }

    #[test]
    fn subtitles_lone_video_takes_all() {
        let found = subs("/m/Heat/heat.mkv", &["/m/Heat/English.srt"], true);
        assert_eq!(
            found,
            vec![(
                "/m/Heat/English.srt".to_string(),
                Some("English".to_string())
            )]
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("couchside-scan-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"not really a video").expect("write");
    }

    #[tokio::test]
    async fn scans_movies_incrementally() {
        let root = temp_dir("movies");
        touch(&root.join("The Matrix (1999)").join("The Matrix (1999).mkv"));
        touch(
            &root
                .join("The Matrix (1999)")
                .join("The Matrix (1999).en.srt"),
        );
        touch(&root.join("Heat.1995.1080p.BluRay.mkv"));
        touch(&root.join("Heat.1995.1080p.BluRay-sample.mkv"));
        touch(&root.join("notes.txt"));

        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, &root.to_string_lossy())
            .await
            .expect("library");

        let stats = scan_library(&db, &lib).await.expect("scan");
        assert_eq!(stats.added_or_updated, 2);
        let (items, _) = db
            .list_movies(None, couchside_db::MovieSort::Title, 10, 0)
            .await
            .expect("list");
        let titles: Vec<_> = items.iter().map(|i| (i.title.as_str(), i.year)).collect();
        assert_eq!(titles, [("Heat", Some(1995)), ("The Matrix", Some(1999))]);
        let matrix = items
            .iter()
            .find(|i| i.title == "The Matrix")
            .expect("matrix");
        assert_eq!(db.subtitles_for(matrix.id).await.expect("subs").len(), 1);

        let stats = scan_library(&db, &lib).await.expect("rescan");
        assert_eq!(stats.unchanged, 2);
        assert_eq!(stats.added_or_updated, 0);

        std::fs::remove_file(root.join("Heat.1995.1080p.BluRay.mkv")).expect("remove");
        let stats = scan_library(&db, &lib).await.expect("rescan");
        assert_eq!(stats.removed, 1);

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn scans_shows() {
        let root = temp_dir("shows");
        let show = root.join("Firefly (2002)");
        touch(
            &show
                .join("Season 01")
                .join("Firefly - S01E01 - Serenity.mkv"),
        );
        touch(
            &show
                .join("Season 01")
                .join("Firefly - S01E02 - The Train Job.mkv"),
        );
        touch(&show.join("Season 01").join("Behind the scenes.mkv"));
        touch(&root.join("loose S01E01.mkv"));

        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("TV", LibraryKind::Shows, &root.to_string_lossy())
            .await
            .expect("library");
        let stats = scan_library(&db, &lib).await.expect("scan");
        assert_eq!(stats.added_or_updated, 2);
        assert_eq!(stats.skipped, 2);

        let (shows, total) = db.list_shows(None, 10, 0).await.expect("shows");
        assert_eq!(total, 1);
        assert_eq!(shows[0].title, "Firefly");
        assert_eq!(shows[0].year, Some(2002));
        let eps = db.show_episodes(shows[0].id).await.expect("episodes");
        let names: Vec<_> = eps
            .iter()
            .map(|e| (e.season, e.episode, e.title.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                (Some(1), Some(1), "Serenity"),
                (Some(1), Some(2), "The Train Job")
            ]
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn offline_root_is_an_error_not_an_empty_library() {
        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Gone", LibraryKind::Movies, "/definitely/not/here")
            .await
            .expect("library");
        let err = scan_library(&db, &lib).await.expect_err("should fail");
        assert!(matches!(err, ScanError::RootUnavailable { .. }));
    }

    #[test]
    fn changed_paths_map_to_their_top_folder() {
        let root = Path::new("lib");
        let movie = root.join("Heat (1995)");
        assert_eq!(
            ScanTarget::for_path(root, &movie.join("Subs").join("en.srt")),
            Some(ScanTarget::Folder(movie.clone()))
        );
        assert_eq!(
            ScanTarget::for_path(root, &movie),
            Some(ScanTarget::Folder(movie))
        );
        assert_eq!(
            ScanTarget::for_path(root, &root.join("Heat.1995.mkv")),
            Some(ScanTarget::RootFiles)
        );
        assert_eq!(
            ScanTarget::for_path(root, &root.join("Movie.en.srt")),
            Some(ScanTarget::RootFiles)
        );
        assert_eq!(ScanTarget::for_path(root, root), None);
        assert_eq!(ScanTarget::for_path(root, Path::new("elsewhere")), None);
        assert_eq!(
            ScanTarget::for_path(root, &root.join("$RECYCLE.BIN").join("x.mkv")),
            None
        );
    }

    #[test]
    fn targets_contain_paths_ignoring_case() {
        let root = Path::new("lib");
        let folder = ScanTarget::Folder(root.join("Heat (1995)"));
        let inside = root.join("HEAT (1995)").join("Heat.mkv");
        let outside = root.join("Heat (1995) 2").join("Heat.mkv");
        assert!(folder.contains(root, &inside.to_string_lossy()));
        assert!(!folder.contains(root, &outside.to_string_lossy()));

        let loose = ScanTarget::RootFiles;
        assert!(loose.contains(root, &root.join("Heat.mkv").to_string_lossy()));
        assert!(loose.contains(root, &root.join("Subs").join("Heat.mkv").to_string_lossy()));
        assert!(!loose.contains(root, &inside.to_string_lossy()));
    }

    #[tokio::test]
    async fn target_scan_only_touches_its_folder() {
        let root = temp_dir("target");
        let matrix = root.join("The Matrix (1999)");
        let heat = root.join("Heat (1995)");
        touch(&matrix.join("The Matrix (1999).mkv"));
        touch(&heat.join("Heat (1995).mkv"));
        touch(&root.join("Alien.1979.mkv"));

        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, &root.to_string_lossy())
            .await
            .expect("library");
        scan_library(&db, &lib).await.expect("scan");

        // A new movie folder appears and another movie is deleted, but only the
        // new folder is scanned: the deleted movie stays until its own folder is scanned.
        let ronin = root.join("Ronin (1998)");
        touch(&ronin.join("Ronin (1998).mkv"));
        std::fs::remove_dir_all(&heat).expect("remove heat");
        let stats = scan_target(&db, &lib, &ScanTarget::Folder(ronin))
            .await
            .expect("scan ronin");
        assert_eq!((stats.added_or_updated, stats.removed), (1, 0));

        let stats = scan_target(&db, &lib, &ScanTarget::Folder(heat))
            .await
            .expect("scan removed folder");
        assert_eq!((stats.added_or_updated, stats.removed), (0, 1));

        std::fs::remove_file(root.join("Alien.1979.mkv")).expect("remove alien");
        let stats = scan_target(&db, &lib, &ScanTarget::RootFiles)
            .await
            .expect("scan root files");
        assert_eq!((stats.unchanged, stats.removed), (0, 1));

        let (items, total) = db
            .list_movies(None, couchside_db::MovieSort::Title, 10, 0)
            .await
            .expect("list");
        assert_eq!(total, 2);
        let titles: Vec<_> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["The Matrix", "Ronin"]);

        std::fs::remove_dir_all(&root).ok();
    }
}

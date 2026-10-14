//! Blocking directory walk that collects video and subtitle files.

use couchside_core::naming::{is_ignored_name, is_subtitle_file, is_video_file};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A video file found on disk.
#[derive(Debug, Clone)]
pub(crate) struct FoundVideo {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
}

/// Everything found under a library root.
#[derive(Debug, Default)]
pub(crate) struct WalkResult {
    pub videos: Vec<FoundVideo>,
    /// Subtitle files grouped by their parent directory.
    pub subtitles: HashMap<PathBuf, Vec<PathBuf>>,
    /// Folders that could not be read (logged, not fatal).
    pub unreadable_dirs: usize,
}

/// Folders next to a lone video whose subtitles belong to it.
pub(crate) const SUBTITLE_DIRS: [&str; 4] = ["Subs", "subs", "Subtitles", "subtitles"];

/// Walks `root` recursively. Fails only if `root` itself cannot be read, so an
/// offline drive never looks like an empty library.
pub(crate) fn walk(root: &Path) -> std::io::Result<WalkResult> {
    walk_where(root, |_, _| true)
}

/// Walks only the files directly in `root` and its subtitle folders: the loose files of
/// a library root.
pub(crate) fn walk_root_files(root: &Path) -> std::io::Result<WalkResult> {
    walk_where(root, |parent, name| {
        parent == root && SUBTITLE_DIRS.contains(&name)
    })
}

/// Walks `root`, entering a sub-folder only when `descend(parent, name)` allows it.
fn walk_where(root: &Path, descend: impl Fn(&Path, &str) -> bool) -> std::io::Result<WalkResult> {
    let mut result = WalkResult::default();
    // Read the root eagerly so its failure is reported to the caller.
    let root_entries = std::fs::read_dir(root)?;
    let mut pending: Vec<(PathBuf, Option<std::fs::ReadDir>)> =
        vec![(root.to_path_buf(), Some(root_entries))];

    while let Some((dir, entries)) = pending.pop() {
        let entries = match entries {
            Some(e) => e,
            None => match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!(dir = %dir.display(), error = %err, "skipping unreadable folder");
                    result.unreadable_dirs += 1;
                    continue;
                }
            },
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if is_ignored_name(&name) {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                if descend(&dir, &name) {
                    pending.push((path, None));
                }
            } else if file_type.is_file() {
                if is_video_file(&path) {
                    let Ok(meta) = entry.metadata() else {
                        continue;
                    };
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
                    result.videos.push(FoundVideo {
                        path,
                        size: meta.len(),
                        mtime,
                    });
                } else if is_subtitle_file(&path) {
                    result.subtitles.entry(dir.clone()).or_default().push(path);
                }
            }
            // Symlinks/junctions are skipped to avoid loops.
        }
    }
    result.videos.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

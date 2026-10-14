//! Turning bursts of file-system events into folder rescans.

use crate::scanner::{walk_scope, ScanTarget};
use solidfin_core::LibraryId;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// Something to rescan after files changed on disk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Change {
    /// The library the change happened in.
    pub library: LibraryId,
    /// The part of the library to rescan; `None` rescans all of it (used when the
    /// operating system dropped events and the exact changes are unknown).
    pub target: Option<ScanTarget>,
}

/// Collects changes and releases each one only after its folder has been quiet for a
/// while, so a burst of events becomes one rescan.
#[derive(Debug)]
pub struct ChangeQueue {
    quiet: Duration,
    pending: HashMap<Change, Instant>,
}

impl ChangeQueue {
    /// A queue that waits `quiet` after the last event before releasing a change.
    pub fn new(quiet: Duration) -> Self {
        Self {
            quiet,
            pending: HashMap::new(),
        }
    }

    /// Notes that `change` happened at `now`, restarting its quiet period.
    pub fn record(&mut self, change: Change, now: Instant) {
        self.pending.insert(change, now);
    }

    /// Removes and returns the changes that have been quiet long enough.
    pub fn take_ready(&mut self, now: Instant) -> Vec<Change> {
        let quiet = self.quiet;
        let mut ready = Vec::new();
        self.pending.retain(|change, last| {
            let due = now.saturating_duration_since(*last) >= quiet;
            if due {
                ready.push(change.clone());
            }
            !due
        });
        ready.sort_by_key(|c| (c.library, c.target.is_some()));
        ready
    }

    /// When the next pending change becomes ready, if any are waiting.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending.values().min().map(|last| *last + self.quiet)
    }
}

/// Whether a video file in `target` of the library at `root` is still open for writing by
/// another program, as during a copy. Always false outside Windows. Blocking.
pub fn files_in_use(root: &Path, target: &ScanTarget) -> bool {
    let Ok(walked) = walk_scope(root, Some(target)) else {
        return false;
    };
    walked
        .videos
        .iter()
        .any(|video| is_being_written(&video.path))
}

#[cfg(windows)]
fn is_being_written(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    // Refusing to share write access fails while any other program has the file open
    // for writing; readers (players, Solidfin's own streams) don't get in the way.
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
    {
        Ok(_) => false,
        Err(err) => err.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

#[cfg(not(windows))]
fn is_being_written(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn change(library: i64, folder: Option<&str>) -> Change {
        Change {
            library: LibraryId(library),
            target: folder.map(|f| ScanTarget::Folder(PathBuf::from(f))),
        }
    }

    #[test]
    fn changes_wait_until_quiet() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let mut queue = ChangeQueue::new(10 * second);
        assert_eq!(queue.next_due(), None);

        queue.record(change(1, Some("a")), start);
        queue.record(change(1, Some("b")), start + 2 * second);
        // More events in "a" restart its wait.
        queue.record(change(1, Some("a")), start + 5 * second);
        assert_eq!(queue.next_due(), Some(start + 12 * second));

        assert!(queue.take_ready(start + 11 * second).is_empty());
        assert_eq!(
            queue.take_ready(start + 12 * second),
            vec![change(1, Some("b"))]
        );
        assert_eq!(
            queue.take_ready(start + 15 * second),
            vec![change(1, Some("a"))]
        );
        assert_eq!(queue.next_due(), None);
    }

    #[test]
    fn whole_library_rescans_come_first() {
        let start = Instant::now();
        let mut queue = ChangeQueue::new(Duration::ZERO);
        queue.record(change(1, Some("a")), start);
        queue.record(change(1, None), start);
        assert_eq!(
            queue.take_ready(start),
            vec![change(1, None), change(1, Some("a"))]
        );
    }

    #[cfg(windows)]
    #[test]
    fn file_open_for_writing_is_in_use() {
        let root = std::env::temp_dir().join(format!("solidfin-in-use-{}", std::process::id()));
        let folder = root.join("Heat (1995)");
        std::fs::create_dir_all(&folder).expect("mkdir");
        let video = folder.join("Heat (1995).mkv");
        let target = ScanTarget::Folder(folder);

        let writer = std::fs::File::create(&video).expect("create");
        assert!(files_in_use(&root, &target));
        drop(writer);
        assert!(!files_in_use(&root, &target));

        let reader = std::fs::File::open(&video).expect("open");
        assert!(!files_in_use(&root, &target));
        drop(reader);

        std::fs::remove_dir_all(&root).ok();
    }
}

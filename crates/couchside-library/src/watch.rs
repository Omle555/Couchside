//! Watching library folders for added, changed, renamed and removed files.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;

/// File-system changes reported by a [`FolderWatcher`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchEvent {
    /// Files or folders that were added, changed, renamed or removed.
    pub paths: Vec<PathBuf>,
    /// The operating system lost track of some changes (too many at once); the libraries
    /// holding `paths`, or every library if `paths` is empty, need a full rescan.
    pub rescan: bool,
}

/// Errors starting the folder watcher.
#[derive(Debug, thiserror::Error)]
#[error("could not start watching folders: {0}")]
pub struct WatchError(#[from] notify::Error);

/// Watches library roots (and everything below them) and sends a [`WatchEvent`] for every
/// change.
pub struct FolderWatcher {
    watcher: RecommendedWatcher,
    watched: HashSet<PathBuf>,
}

impl FolderWatcher {
    /// Starts a watcher that sends its events to `events`. Nothing is watched until
    /// [`FolderWatcher::watch_only`] is called.
    pub fn new(events: UnboundedSender<WatchEvent>) -> Result<Self, WatchError> {
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            match result {
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
                Ok(event) => {
                    let rescan = event.need_rescan();
                    // The receiver is gone only while the server shuts down.
                    let _ = events.send(WatchEvent {
                        paths: event.paths,
                        rescan,
                    });
                }
                Err(err) => tracing::warn!(error = %err, "folder watching error"),
            }
        })?;
        Ok(Self {
            watcher,
            watched: HashSet::new(),
        })
    }

    /// Watches exactly `roots`: starts on new ones and stops on ones no longer listed.
    /// A root that can't be watched now (offline drive or share) is logged and tried again
    /// on the next call. Blocking: opening a network folder can take a while.
    pub fn watch_only(&mut self, roots: &[PathBuf]) {
        let wanted: HashSet<&PathBuf> = roots.iter().collect();
        let stale: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|root| !wanted.contains(root))
            .cloned()
            .collect();
        for root in stale {
            if let Err(err) = self.watcher.unwatch(&root) {
                tracing::debug!(folder = %root.display(), error = %err, "could not stop watching folder");
            }
            self.watched.remove(&root);
        }
        for root in wanted {
            if self.watched.contains(root) {
                continue;
            }
            match self.watcher.watch(root, RecursiveMode::Recursive) {
                Ok(()) => {
                    tracing::info!(folder = %root.display(), "watching library folder for changes");
                    self.watched.insert(root.clone());
                }
                Err(err) => {
                    tracing::warn!(folder = %root.display(), error = %err, "could not watch library folder; will retry");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn reports_new_files() {
        let root = std::env::temp_dir().join(format!("couchside-watch-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).expect("mkdir");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = FolderWatcher::new(tx).expect("watcher");
        watcher.watch_only(std::slice::from_ref(&root));

        let movie = root.join("Heat (1995)");
        std::fs::create_dir_all(&movie).expect("mkdir movie");
        std::fs::write(movie.join("Heat (1995).mkv"), b"video").expect("write");

        let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("an event within 10 s")
            .expect("watcher still running");
        assert!(event.paths.iter().all(|p| p.starts_with(&root)));

        watcher.watch_only(&[]);
        drop(watcher);
        std::fs::remove_dir_all(&root).ok();
    }
}

//! The transcode temp folder (rule 7.6).

use std::io;
use std::path::Path;

/// Every transcode session's folder name starts with this, so cleaning up only ever removes
/// Couchside's own folders, even if the temp folder is set to somewhere shared.
pub const SESSION_DIR_PREFIX: &str = "cs-";

/// Creates the temp folder if needed and removes session folders left over from a previous
/// run (e.g. after a crash or power cut). Returns how many were removed. Blocking: call it
/// from `spawn_blocking`.
pub fn clear_temp_dir(dir: &Path) -> io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut removed = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(SESSION_DIR_PREFIX));
        if !ours || !entry.file_type()?.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => removed += 1,
            // Keep going: one locked folder must not stop the server from starting.
            Err(err) => tracing::warn!(
                path = %entry.path().display(),
                error = %err,
                "could not remove old transcode folder"
            ),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_session_folders() {
        let dir = std::env::temp_dir().join(format!("couchside-temp-test-{}", std::process::id()));
        let session = dir.join(format!("{SESSION_DIR_PREFIX}abc"));
        std::fs::create_dir_all(&session).expect("session dir");
        std::fs::write(session.join("seg-1.m4s"), b"x").expect("segment");
        std::fs::create_dir_all(dir.join("Movies")).expect("other dir");
        std::fs::write(dir.join(format!("{SESSION_DIR_PREFIX}note.txt")), b"x").expect("file");

        assert_eq!(clear_temp_dir(&dir).expect("clear"), 1);
        assert!(!session.exists());
        assert!(dir.join("Movies").is_dir());
        assert!(dir.join(format!("{SESSION_DIR_PREFIX}note.txt")).is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn creates_missing_folder() {
        let dir = std::env::temp_dir()
            .join(format!("couchside-temp-new-{}", std::process::id()))
            .join("transcodes");
        assert_eq!(clear_temp_dir(&dir).expect("clear"), 0);
        assert!(dir.is_dir());
        if let Some(parent) = dir.parent() {
            std::fs::remove_dir_all(parent).ok();
        }
    }
}

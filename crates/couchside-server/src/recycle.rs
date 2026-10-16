//! Sends files and folders to the Windows Recycle Bin, or on Linux to a hidden
//! `.couchside-trash` folder next to them (used by a movie's "Delete" button).

use std::path::{Path, PathBuf};

/// Moves `path` (a file or a whole folder) to the Recycle Bin and returns where it went. On
/// drives without one, such as network shares, Windows deletes it permanently.
#[cfg(windows)]
pub fn recycle(path: &Path) -> std::io::Result<PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };

    // The shell wants a list of paths ending in two NULs.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT;
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: u16::try_from(flags.0).unwrap_or(u16::MAX),
        ..Default::default()
    };
    // SAFETY: `op` is fully initialised, `pFrom` points to a double-NUL-terminated UTF-16
    // buffer that lives until the call returns, and no other pointers are set.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(std::io::Error::other(format!(
            "Windows could not delete it (error {code}); the file may be in use"
        )));
    }
    Ok(PathBuf::from("Recycle Bin"))
}

/// Folder, next to what is deleted, that holds deleted files on Linux. The scanner skips names
/// starting with a dot, so nothing in it shows up in the library.
#[cfg(any(not(windows), test))]
const TRASH_DIR: &str = ".couchside-trash";

/// Linux has no system Recycle Bin a service can use: moves `path` into `.couchside-trash` in
/// the same folder (same drive, so it is instant and can be moved back) and returns the new path.
#[cfg(not(windows))]
pub fn recycle(path: &Path) -> std::io::Result<PathBuf> {
    move_to_trash(path)
}

/// Moves `path` into the trash folder beside it, never overwriting anything already there.
#[cfg(any(not(windows), test))]
fn move_to_trash(path: &Path) -> std::io::Result<PathBuf> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(std::io::Error::other(
            "can't delete a drive or library root",
        ));
    };
    let trash = parent.join(TRASH_DIR);
    std::fs::create_dir_all(&trash)?;
    let name = name.to_string_lossy();
    for n in 1..1000 {
        let target = if n == 1 {
            trash.join(&*name)
        } else {
            trash.join(format!("{name} ({n})"))
        };
        if target.symlink_metadata().is_err() {
            std::fs::rename(path, &target)?;
            return Ok(target);
        }
    }
    Err(std::io::Error::other(
        "the trash folder is full of copies of this name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_keeps_earlier_copies() {
        let dir = std::env::temp_dir().join(format!("couchside-trash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Alien (1979)")).expect("folder");
        std::fs::write(dir.join("Alien (1979)").join("Alien.mkv"), b"x").expect("file");

        let first = move_to_trash(&dir.join("Alien (1979)")).expect("first");
        assert_eq!(first, dir.join(TRASH_DIR).join("Alien (1979)"));
        assert!(first.join("Alien.mkv").exists());

        std::fs::create_dir_all(dir.join("Alien (1979)")).expect("again");
        let second = move_to_trash(&dir.join("Alien (1979)")).expect("second");
        assert_eq!(second, dir.join(TRASH_DIR).join("Alien (1979) (2)"));
        assert!(first.join("Alien.mkv").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

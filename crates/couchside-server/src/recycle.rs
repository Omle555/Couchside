//! Sends files and folders to the Windows Recycle Bin (used by a movie's "Delete" button).

use std::path::Path;

/// Moves `path` (a file or a whole folder) to the Recycle Bin. On drives without one, such
/// as network shares, Windows deletes it permanently.
#[cfg(windows)]
pub fn recycle(path: &Path) -> std::io::Result<()> {
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
    Ok(())
}

/// Outside Windows there is no Recycle Bin: deletes `path` permanently.
#[cfg(not(windows))]
pub fn recycle(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

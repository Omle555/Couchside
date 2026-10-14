//! Starting FFmpeg tools safely: arguments as a list, no console window, killed when dropped,
//! and on Windows also placed in a job that Windows closes (killing them) if Couchside dies.

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// A command for an FFmpeg tool with stdin closed, output captured and `kill_on_drop`.
pub(crate) fn new_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: don't flash a console window when running as a tray/service app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(windows)]
pub(crate) use windows_job::KillOnCloseJob;

#[cfg(windows)]
mod windows_job {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A Windows job object whose processes are killed when its last handle closes. Couchside
    /// holds the only handle, so if Couchside exits or crashes, every ffmpeg in it is killed.
    pub(crate) struct KillOnCloseJob(HANDLE);

    // SAFETY: a job handle is a kernel handle that may be used from any thread; Couchside
    // never closes it except in `Drop`, which runs once.
    unsafe impl Send for KillOnCloseJob {}
    // SAFETY: as above; the job functions used are thread-safe.
    unsafe impl Sync for KillOnCloseJob {}

    impl KillOnCloseJob {
        /// Creates an unnamed job with the kill-on-close limit set.
        pub(crate) fn new() -> windows::core::Result<Self> {
            // SAFETY: no security attributes and no name are passed; the returned handle is
            // owned by the new `KillOnCloseJob` and closed in its `Drop`.
            let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }?;
            let job = Self(handle);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .unwrap_or(u32::MAX);
            // SAFETY: `info` is a valid JOBOBJECT_EXTENDED_LIMIT_INFORMATION that outlives the
            // call, `size` is its exact size, and `job.0` is a live job handle.
            unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&info).cast(),
                    size,
                )
            }?;
            Ok(job)
        }

        /// Puts a running child process into the job.
        pub(crate) fn assign(&self, child: &tokio::process::Child) -> windows::core::Result<()> {
            let Some(raw) = child.raw_handle() else {
                // The process has already exited: nothing to guard.
                return Ok(());
            };
            // SAFETY: `raw` is the child's process handle, kept open by `child` for the
            // duration of this call; `self.0` is a live job handle.
            unsafe { AssignProcessToJobObject(self.0, HANDLE(raw)) }
        }
    }

    impl Drop for KillOnCloseJob {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed only here.
            if let Err(err) = unsafe { CloseHandle(self.0) } {
                tracing::warn!(error = %err, "could not close the ffmpeg job handle");
            }
        }
    }
}

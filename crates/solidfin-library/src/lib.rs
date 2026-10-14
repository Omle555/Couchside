//! Library scanning (finding movies, episodes and subtitles on disk), media probing with
//! ffprobe and still frames grabbed with ffmpeg. Files in a library are only ever changed by
//! "Clean up" (renames, moving images and subtitles out) and "Delete", and only when file
//! changes are allowed.

mod delete;
mod extras;
mod frames;
mod probe;
mod rename;
mod scanner;
mod walk;

pub use delete::{plan_movie_delete, DeletePlan};
pub use extras::{is_removable_extra, move_extras, removable_extras};
pub use frames::{Ffmpeg, Frame, FrameError, FrameRequest};
pub use probe::{Ffprobe, ProbeError};
pub use rename::{
    execute_rename, map_path, plan_movie_rename, sidecar_renames, undo_rename, FileMove,
    RenameError, RenamePlan,
};
pub use scanner::{scan_library, ScanError, ScanStats};

//! Transcoding for Couchside: deciding how a file can reach a client (direct play, remux,
//! audio convert or full transcode), cutting streams into HLS segments with ffmpeg, and
//! managing the temporary files that conversions write.
//!
//! See `docs/transcoding-plan.md` and `docs/adr/0002-transcoding.md`.

mod decision;
mod process;
mod segments;
mod sessions;
mod temp;

pub use decision::{
    decide, AudioCodec, ClientProfile, Container, PlaybackDecision, PlaybackMode, Reason,
    SourceInfo, VideoCodec,
};
pub use segments::{read_keyframes, segments, Segment};
pub use sessions::{ManagerSettings, StreamRequest, TranscodeManager};
pub use temp::{clear_temp_dir, SESSION_DIR_PREFIX};

/// Errors from streaming and keyframe scans.
#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    /// ffmpeg was not found, so nothing can be converted.
    #[error("ffmpeg was not found")]
    NoFfmpeg,
    /// Too many streams are already running.
    #[error("{0} videos are already being converted")]
    Busy(u32),
    /// No such session (or it belongs to someone else).
    #[error("stream not found")]
    NotFound,
    /// ffmpeg or ffprobe took too long.
    #[error("timed out waiting for ffmpeg")]
    Timeout,
    /// ffmpeg or ffprobe could not be started, or a file could not be read or written.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// ffmpeg or ffprobe failed.
    #[error("ffmpeg failed: {0}")]
    Failed(String),
}

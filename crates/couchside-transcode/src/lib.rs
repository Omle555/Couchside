//! Transcoding for Couchside: deciding how a file can reach a client (direct play, remux,
//! audio convert or full transcode) and managing the temporary files that conversions write.
//!
//! See `docs/transcoding-plan.md` and `docs/adr/0002-transcoding.md`.

mod decision;
mod temp;

pub use decision::{
    decide, AudioCodec, ClientProfile, Container, PlaybackDecision, PlaybackMode, Reason,
    SourceInfo, VideoCodec,
};
pub use temp::{clear_temp_dir, SESSION_DIR_PREFIX};

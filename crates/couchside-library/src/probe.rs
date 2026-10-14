//! Media probing with an external `ffprobe` binary.
//!
//! ffprobe is run as a child process with arguments passed as a list (never
//! through a shell), a timeout, and `kill_on_drop` so no process is orphaned.

use couchside_db::ProbeData;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors from a single probe.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// ffprobe could not be started.
    #[error("could not run ffprobe: {0}")]
    Spawn(#[from] std::io::Error),
    /// ffprobe took too long and was killed.
    #[error("ffprobe timed out")]
    Timeout,
    /// ffprobe exited with an error (usually an unreadable or corrupt file).
    #[error("ffprobe failed: {0}")]
    Failed(String),
    /// ffprobe printed something that was not the expected JSON.
    #[error("unexpected ffprobe output: {0}")]
    Parse(#[from] serde_json::Error),
}

/// A located, working ffprobe executable.
#[derive(Debug, Clone)]
pub struct Ffprobe {
    path: PathBuf,
}

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

/// Finds a working copy of an FFmpeg tool (`ffprobe`, `ffmpeg`): `preferred` paths first, then
/// next to the Couchside executable (also `ffmpeg\bin\`), then `PATH`.
pub(crate) async fn locate_tool(name: &str, preferred: Vec<PathBuf>) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut candidates = preferred;
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        candidates.push(exe_dir.join(&exe_name));
        candidates.push(exe_dir.join("ffmpeg").join("bin").join(&exe_name));
    }
    candidates.push(PathBuf::from(&exe_name));

    for candidate in candidates {
        let mut cmd = new_command(&candidate);
        cmd.arg("-version");
        let ok = matches!(
            tokio::time::timeout(VERSION_TIMEOUT, cmd.output()).await,
            Ok(Ok(output)) if output.status.success()
        );
        if ok {
            tracing::info!(path = %candidate.display(), "using {name}");
            return Some(candidate);
        }
    }
    None
}

impl Ffprobe {
    /// Finds ffprobe: the configured path, then next to the Couchside executable
    /// (also `ffmpeg\bin\`), then `PATH`. Returns `None` if none of them work.
    pub async fn locate(configured: Option<&Path>) -> Option<Self> {
        let preferred = configured.map(Path::to_path_buf).into_iter().collect();
        match locate_tool("ffprobe", preferred).await {
            Some(path) => Some(Self { path }),
            None => {
                tracing::warn!("ffprobe not found; durations and codec info will be unavailable");
                None
            }
        }
    }

    /// Where the working ffprobe executable is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads duration, container and codec details from a media file.
    pub async fn probe(&self, file: &Path) -> Result<ProbeData, ProbeError> {
        let mut cmd = new_command(&self.path);
        cmd.args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file);
        let output = tokio::time::timeout(PROBE_TIMEOUT, cmd.output())
            .await
            .map_err(|_| ProbeError::Timeout)??;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ProbeError::Failed(
                stderr.trim().chars().take(300).collect(),
            ));
        }
        parse_probe_output(&output.stdout)
    }
}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    #[serde(default)]
    disposition: Option<ProbeDisposition>,
}

#[derive(Debug, Deserialize)]
struct ProbeDisposition {
    #[serde(default)]
    attached_pic: i64,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
}

/// Converts ffprobe's JSON into [`ProbeData`].
fn parse_probe_output(json: &[u8]) -> Result<ProbeData, ProbeError> {
    let out: ProbeOutput = serde_json::from_slice(json)?;
    // Skip embedded cover art, which ffprobe reports as a video stream.
    let video = out.streams.iter().find(|s| {
        s.codec_type.as_deref() == Some("video")
            && s.disposition.as_ref().is_none_or(|d| d.attached_pic == 0)
    });
    let audio = out
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));
    let format = out.format.as_ref();
    Ok(ProbeData {
        runtime_seconds: format
            .and_then(|f| f.duration.as_deref())
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0),
        container: format.and_then(|f| f.format_name.clone()),
        video_codec: video.and_then(|s| s.codec_name.clone()),
        audio_codec: audio.and_then(|s| s.codec_name.clone()),
        width: video.and_then(|s| s.width),
        height: video.and_then(|s| s.height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{
            "streams": [
                {"codec_type": "video", "codec_name": "mjpeg", "width": 600, "height": 900, "disposition": {"attached_pic": 1}},
                {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080, "disposition": {"attached_pic": 0}},
                {"codec_type": "audio", "codec_name": "aac"}
            ],
            "format": {"format_name": "matroska,webm", "duration": "5400.250000"}
        }"#;
        let data = parse_probe_output(json).expect("parses");
        assert_eq!(data.video_codec.as_deref(), Some("h264"));
        assert_eq!(data.audio_codec.as_deref(), Some("aac"));
        assert_eq!(data.width, Some(1920));
        assert_eq!(data.container.as_deref(), Some("matroska,webm"));
        assert!((data.runtime_seconds.unwrap_or_default() - 5400.25).abs() < 0.001);
    }

    #[test]
    fn tolerates_missing_fields() {
        let data = parse_probe_output(b"{}").expect("parses");
        assert!(data.runtime_seconds.is_none());
        assert!(data.video_codec.is_none());
    }
}

//! Still frames from part of a video, grabbed with an external `ffmpeg` binary.
//!
//! Only keyframes are decoded, so even a 4K file is read in seconds. ffmpeg runs with
//! arguments passed as a list, a timeout and `kill_on_drop`, like ffprobe.

use crate::probe::{locate_tool, new_command};
use std::path::{Path, PathBuf};
use std::time::Duration;

const GRAB_TIMEOUT: Duration = Duration::from_secs(180);

/// Errors from grabbing frames.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// ffmpeg could not be started.
    #[error("could not run ffmpeg: {0}")]
    Spawn(#[from] std::io::Error),
    /// ffmpeg took too long and was killed.
    #[error("ffmpeg timed out")]
    Timeout,
    /// ffmpeg exited with an error (usually an unreadable or corrupt file).
    #[error("ffmpeg failed: {0}")]
    Failed(String),
}

/// Which part of a video to grab frames from.
#[derive(Debug, Clone, Copy)]
pub struct FrameRequest {
    /// Where to start looking (zero for the opening).
    pub start: Duration,
    /// How long a stretch of video to look at from `start`.
    pub window: Duration,
    /// Smallest gap between two grabbed frames.
    pub min_gap: Duration,
    /// Most frames to return.
    pub max_frames: u32,
    /// Width of the returned images in pixels.
    pub width: u32,
}

/// One grabbed frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Position in the video, in seconds.
    pub time_seconds: f64,
    /// The frame as a JPEG image.
    pub jpeg: Vec<u8>,
}

/// A located, working ffmpeg executable.
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    path: PathBuf,
}

impl Ffmpeg {
    /// Finds ffmpeg: `preferred` paths first, then next to the Couchside executable
    /// (also `ffmpeg\bin\`), then `PATH`. Returns `None` if none of them work.
    pub async fn locate(preferred: Vec<PathBuf>) -> Option<Self> {
        match locate_tool("ffmpeg", preferred).await {
            Some(path) => Some(Self { path }),
            None => {
                tracing::warn!("ffmpeg not found; Identify can't scan videos for their title");
                None
            }
        }
    }

    /// Grabs keyframes from `req.start` to `req.start + req.window` of `file`, at least
    /// `min_gap` apart. Frame times are positions in the whole video.
    pub async fn grab_frames(
        &self,
        file: &Path,
        req: FrameRequest,
    ) -> Result<Vec<Frame>, FrameError> {
        let filter = format!(
            "select='isnan(prev_selected_t)+gte(t-prev_selected_t,{gap})',showinfo,scale={width}:-2",
            gap = req.min_gap.as_secs_f64(),
            width = req.width,
        );
        let mut cmd = new_command(&self.path);
        cmd.args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"])
            .args(["-skip_frame", "nokey"]);
        if !req.start.is_zero() {
            // Input seeking: fast, and output timestamps restart near zero.
            cmd.args(["-ss", &format!("{:.3}", req.start.as_secs_f64())]);
        }
        cmd.args(["-t", &req.window.as_secs().to_string()])
            .arg("-i")
            .arg(file)
            .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf", &filter])
            .args(["-fps_mode", "passthrough"])
            .args(["-frames:v", &req.max_frames.to_string()])
            .args(["-f", "image2pipe", "-c:v", "mjpeg", "-q:v", "4", "pipe:1"]);
        let output = tokio::time::timeout(GRAB_TIMEOUT, cmd.output())
            .await
            .map_err(|_| FrameError::Timeout)??;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            let last = stderr.lines().last().unwrap_or_default();
            return Err(FrameError::Failed(last.trim().chars().take(300).collect()));
        }
        let images = split_jpegs(&output.stdout);
        let offset = req.start.as_secs_f64();
        let times: Vec<f64> = frame_times(&stderr)
            .into_iter()
            .map(|t| t + offset)
            .collect();
        if images.len() != times.len() {
            tracing::debug!(
                images = images.len(),
                times = times.len(),
                "ffmpeg frame count and timestamps differ"
            );
        }
        Ok(images
            .into_iter()
            .zip(times)
            .map(|(jpeg, time_seconds)| Frame { time_seconds, jpeg })
            .collect())
    }
}

/// Frame times from the `showinfo` filter's log lines, in output order.
fn frame_times(stderr: &str) -> Vec<f64> {
    stderr
        .lines()
        .filter(|line| line.contains("Parsed_showinfo"))
        .filter_map(|line| {
            let rest = &line[line.find("pts_time:")? + "pts_time:".len()..];
            rest.split_whitespace().next()?.parse::<f64>().ok()
        })
        .filter(|t| t.is_finite())
        .map(|t| t.max(0.0))
        .collect()
}

/// Splits ffmpeg's MJPEG pipe output into separate JPEG images by walking the JPEG markers.
fn split_jpegs(data: &[u8]) -> Vec<Vec<u8>> {
    let mut images = Vec::new();
    let mut pos = 0;
    while let Some(start) = find_soi(data, pos) {
        match jpeg_end(data, start) {
            Some(end) => {
                images.push(data[start..end].to_vec());
                pos = end;
            }
            None => break,
        }
    }
    images
}

fn find_soi(data: &[u8], from: usize) -> Option<usize> {
    data.get(from..)?
        .windows(2)
        .position(|w| w == [0xFF, 0xD8])
        .map(|i| from + i)
}

/// Index just past the end-of-image marker of the JPEG starting at `start`.
fn jpeg_end(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 2;
    loop {
        if *data.get(i)? != 0xFF {
            return None;
        }
        let marker = *data.get(i + 1)?;
        match marker {
            // Fill bytes before a marker.
            0xFF => i += 1,
            0xD9 => return Some(i + 2),
            // Markers without a length.
            0x01 | 0xD0..=0xD7 => i += 2,
            _ => {
                let len = usize::from(u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]));
                i += 2 + len;
                if marker == 0xDA {
                    i = skip_entropy_data(data, i)?;
                }
            }
        }
    }
}

/// Skips compressed image data: it ends at the first 0xFF that isn't a stuffed byte or a
/// restart marker.
fn skip_entropy_data(data: &[u8], mut i: usize) -> Option<usize> {
    loop {
        if *data.get(i)? == 0xFF {
            match *data.get(i + 1)? {
                0x00 | 0xD0..=0xD7 => i += 2,
                _ => return Some(i),
            }
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal JPEG-shaped byte sequence: SOI, a segment containing 0xFF 0xD9,
    /// SOS, entropy data with a stuffed byte and a restart marker, EOI.
    fn fake_jpeg(fill: u8) -> Vec<u8> {
        vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xDB, 0x00, 0x05, 0xFF, 0xD9, fill, // DQT-like segment with FFD9 inside
            0xFF, 0xDA, 0x00, 0x03, 0x01, // SOS header
            fill, 0xFF, 0x00, 0x12, 0xFF, 0xD3, 0x34, // data, stuffed FF, RST3
            0xFF, 0xD9, // EOI
        ]
    }

    #[test]
    fn splits_concatenated_jpegs() {
        let mut stream = fake_jpeg(1);
        stream.extend(fake_jpeg(2));
        stream.extend(fake_jpeg(3));
        let images = split_jpegs(&stream);
        assert_eq!(images.len(), 3);
        assert_eq!(images[1], fake_jpeg(2));
    }

    #[test]
    fn ignores_truncated_jpeg() {
        let mut stream = fake_jpeg(1);
        let partial = fake_jpeg(2);
        stream.extend(&partial[..partial.len() - 2]);
        assert_eq!(split_jpegs(&stream).len(), 1);
        assert!(split_jpegs(&[]).is_empty());
    }

    #[test]
    fn reads_showinfo_times() {
        let log = "\
Input #0, matroska,webm, from 'x.mkv':
[Parsed_showinfo_1 @ 000001] config in time_base: 1/1000, frame_rate: 24000/1001
[Parsed_showinfo_1 @ 000001] n:   0 pts:      0 pts_time:0       duration:42 fmt:yuv420p
[Parsed_showinfo_1 @ 000001] color_range:tv color_space:bt709
[Parsed_showinfo_1 @ 000001] n:   1 pts:   4171 pts_time:4.171   duration:42
[Parsed_showinfo_1 @ 000001] n:   2 pts:  10010 pts_time:10.01   duration:42
frame=    3 fps=0.0 q=4.0 size=N/A time=00:00:10.01";
        assert_eq!(frame_times(log), vec![0.0, 4.171, 10.01]);
    }
}

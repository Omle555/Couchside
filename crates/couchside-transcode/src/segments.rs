//! Keyframe times and the HLS segments cut from them.
//!
//! When video is copied, a segment can only start on a keyframe. ffmpeg's HLS muxer is told
//! to cut at every keyframe (`-hls_time` near zero), so segment `n` always runs from keyframe
//! `n` to keyframe `n + 1`, whichever keyframe ffmpeg was started from. That lets the whole
//! playlist be written before ffmpeg has produced anything, and lets ffmpeg be restarted at
//! any segment when the viewer seeks ahead.

use crate::process::new_command;
use crate::TranscodeError;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

/// Longest a keyframe scan may take (it reads the whole file once).
const SCAN_TIMEOUT: Duration = Duration::from_secs(600);

/// How far past a keyframe ffmpeg is asked to seek. ffmpeg seeks to the keyframe at or before
/// the requested time, but first moves the request back by about 0.13 s for files with
/// B-frames; aiming a little after the keyframe makes it land on that keyframe every time.
const SEEK_PAST_KEYFRAME: f64 = 0.2;

/// One HLS segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Start, in seconds from the start of the video.
    pub start: f64,
    /// Length in seconds.
    pub duration: f64,
}

/// Reads the keyframe times of a file's first video stream, in milliseconds from the start
/// of the file. Reads (but doesn't decode) the whole file, so a big file takes a while.
pub async fn read_keyframes(ffprobe: &Path, file: &Path) -> Result<Vec<i64>, TranscodeError> {
    let start = run_ffprobe(
        ffprobe,
        file,
        &["-show_entries", "format=start_time", "-of", "csv=p=0"],
    )
    .await?;
    let start: f64 = start.trim().parse().unwrap_or(0.0);
    let packets = run_ffprobe(
        ffprobe,
        file,
        &[
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,flags",
            "-of",
            "csv=p=0",
        ],
    )
    .await?;
    let times = parse_keyframes(&packets, start);
    if times.is_empty() {
        return Err(TranscodeError::Failed("no keyframes found".into()));
    }
    Ok(times)
}

async fn run_ffprobe(ffprobe: &Path, file: &Path, args: &[&str]) -> Result<String, TranscodeError> {
    let mut cmd = new_command(ffprobe);
    cmd.args(["-v", "error"]).args(args).arg(file);
    let output = tokio::time::timeout(SCAN_TIMEOUT, cmd.output())
        .await
        .map_err(|_| TranscodeError::Timeout)??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TranscodeError::Failed(
            stderr.trim().chars().take(300).collect(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Keyframe times from ffprobe's `pts_time,flags` lines, relative to `start` (the file's
/// start time), sorted and without duplicates.
fn parse_keyframes(csv: &str, start: f64) -> Vec<i64> {
    let mut times: Vec<i64> = csv
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.trim().split_once(',')?;
            if !flags.starts_with('K') {
                return None;
            }
            let t = pts.parse::<f64>().ok()? - start;
            t.is_finite().then(|| (t.max(0.0) * 1000.0).round() as i64)
        })
        .collect();
    times.sort_unstable();
    times.dedup();
    times
}

/// The segments of a video `duration` seconds long with keyframes at `keyframes_ms`.
/// The first segment always starts at zero.
pub fn segments(keyframes_ms: &[i64], duration: f64) -> Vec<Segment> {
    let limit = (duration * 1000.0) as i64;
    let mut starts: Vec<f64> = keyframes_ms
        .iter()
        .filter(|&&ms| ms < limit)
        .map(|&ms| ms as f64 / 1000.0)
        .collect();
    if let Some(first) = starts.first_mut() {
        *first = 0.0;
    }
    let ends = starts.iter().skip(1).copied().chain([duration]);
    starts
        .iter()
        .zip(ends)
        .map(|(&start, end)| Segment {
            start,
            duration: (end - start).max(0.001),
        })
        .collect()
}

/// Where to point ffmpeg's `-ss` so it starts exactly on segment `index`.
pub fn seek_time(segments: &[Segment], index: usize) -> Option<f64> {
    let seg = segments.get(index)?;
    Some(seg.start + SEEK_PAST_KEYFRAME.min(seg.duration / 2.0))
}

/// A complete (VOD) HLS playlist listing every segment as `seg/{n}.m4s`.
pub fn playlist(segments: &[Segment]) -> String {
    let target = segments
        .iter()
        .map(|s| s.duration.ceil() as u64)
        .max()
        .unwrap_or(1)
        .max(1);
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"init.mp4\"\n"
    );
    for (n, seg) in segments.iter().enumerate() {
        // Writing to a String can't fail.
        let _ = write!(out, "#EXTINF:{:.3},\nseg/{n}.m4s\n", seg.duration);
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_packets() {
        let csv = "10.000000,K__\n10.041708,___\nN/A,K_\n13.000000,K_\n13.000000,K__\n9.9,K\n";
        assert_eq!(parse_keyframes(csv, 10.0), vec![0, 3000]);
    }

    #[test]
    fn segments_run_between_keyframes() {
        let segs = segments(&[42, 3000, 6000, 9500], 10.0);
        assert_eq!(segs.len(), 4);
        assert_eq!(
            segs[0],
            Segment {
                start: 0.0,
                duration: 3.0
            }
        );
        assert_eq!(segs[2].start, 6.0);
        assert!((segs[3].duration - 0.5).abs() < 1e-9);
        // Keyframes past the end are ignored.
        assert_eq!(segments(&[0, 3000, 12000], 10.0).len(), 2);
        assert!(segments(&[], 10.0).is_empty());
    }

    #[test]
    fn seeks_just_past_the_keyframe() {
        let segs = segments(&[0, 3000, 3100], 10.0);
        assert_eq!(seek_time(&segs, 0), Some(0.2));
        assert!((seek_time(&segs, 1).unwrap_or_default() - 3.05).abs() < 1e-9);
        assert_eq!(seek_time(&segs, 9), None);
    }

    #[test]
    fn playlist_lists_every_segment() {
        let text = playlist(&segments(&[0, 2500, 9000], 12.0));
        assert!(text.contains("#EXT-X-TARGETDURATION:7\n"));
        assert!(text.contains("#EXTINF:2.500,\nseg/0.m4s\n"));
        assert!(text.contains("#EXTINF:3.000,\nseg/2.m4s\n"));
        assert!(text.ends_with("#EXT-X-ENDLIST\n"));
    }
}

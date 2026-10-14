//! End-to-end check of a streaming session against real ffmpeg/ffprobe on `PATH`, with a
//! tiny generated clip (rule 13.3). Skipped when the tools aren't installed.

use couchside_transcode::{
    read_keyframes, segments, ManagerSettings, PlaybackMode, StreamRequest, TranscodeError,
    TranscodeManager,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> Option<PathBuf> {
    let ok = Command::new(name)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    ok.then(|| PathBuf::from(name))
}

/// 30 s of H.264 with a keyframe every 2 s and AC3 audio, in MKV.
fn make_clip(dir: &Path) -> PathBuf {
    let clip = dir.join("clip.mkv");
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "30",
        ])
        .args([
            "-c:v",
            "libx264",
            "-g",
            "48",
            "-keyint_min",
            "48",
            "-sc_threshold",
            "0",
        ])
        .args(["-c:a", "ac3", "-ac", "2"])
        .arg(&clip)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "could not make the test clip");
    clip
}

#[tokio::test]
async fn audio_convert_session_serves_and_cleans_up() {
    let (Some(ffmpeg), Some(ffprobe)) = (tool("ffmpeg"), tool("ffprobe")) else {
        eprintln!("ffmpeg/ffprobe not on PATH; skipping");
        return;
    };
    let base = std::env::temp_dir().join(format!("couchside-stream-test-{}", std::process::id()));
    std::fs::create_dir_all(&base).expect("temp dir");
    let clip = make_clip(&base);

    let keyframes = read_keyframes(&ffprobe, &clip).await.expect("keyframes");
    assert_eq!(keyframes.len(), 15, "a keyframe every 2 s: {keyframes:?}");
    let segs = segments(&keyframes, 30.0);

    let temp_dir = base.join("transcodes");
    let manager = TranscodeManager::new(ManagerSettings {
        ffmpeg: Some(ffmpeg),
        temp_dir: temp_dir.clone(),
        max_remuxes: 1,
        temp_max_bytes: u64::MAX,
    });
    let request = |id: &str, device: &str| StreamRequest {
        session_id: id.into(),
        user_id: 7,
        device_id: device.into(),
        input: clip.clone(),
        mode: PlaybackMode::AudioConvert,
        video_stream: 0,
        hevc: false,
        audio_stream: Some(1),
        segments: segs.clone(),
        start_seconds: 0.0,
    };
    manager.start(request("abc", "phone")).await.expect("start");

    let playlist = manager.playlist("abc", 7).expect("playlist");
    assert!(playlist.contains("seg/14.m4s"));
    assert!(matches!(
        manager.playlist("abc", 8),
        Err(TranscodeError::NotFound)
    ));

    let init = manager.init_segment("abc", 7).await.expect("init");
    assert!(init.windows(4).any(|w| w == b"moov"));
    let first = manager.segment("abc", 7, 0).await.expect("segment 0");
    assert!(std::fs::metadata(&first).expect("seg 0").len() > 0);

    // A segment far ahead restarts ffmpeg there.
    let late = manager.segment("abc", 7, 13).await.expect("segment 13");
    let init_path = late.with_file_name("init.mp4");
    let probe = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "packet=pts_time",
        ])
        .args(["-of", "csv=p=0", "-i"])
        .arg(format!("concat:{}|{}", init_path.display(), late.display()))
        .output()
        .expect("ffprobe segment");
    let first_pts = String::from_utf8_lossy(&probe.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<f64>().ok())
        .fold(f64::MAX, f64::min);
    assert!(
        (first_pts - segs[13].start).abs() < 0.05,
        "segment 13 starts at {first_pts}, expected {}",
        segs[13].start
    );

    // Another device over the limit is turned away; the same device replaces its stream.
    assert!(matches!(
        manager.start(request("def", "tv")).await,
        Err(TranscodeError::Busy(1))
    ));
    manager
        .start(request("ghi", "phone"))
        .await
        .expect("replace");
    assert!(matches!(
        manager.playlist("abc", 7),
        Err(TranscodeError::NotFound)
    ));
    assert!(!temp_dir.join("cs-abc").exists());

    manager.shutdown().await;
    assert!(!temp_dir.join("cs-ghi").exists());
    std::fs::remove_dir_all(&base).ok();
}

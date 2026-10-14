//! End-to-end check of a streaming session against real ffmpeg/ffprobe on `PATH`, with a
//! tiny generated clip (rule 13.3). Skipped when the tools aren't installed.

use couchside_core::config::HardwareAccel;
use couchside_transcode::{
    fixed_segments, read_keyframes, segments, ManagerSettings, PlaybackMode, StreamRequest,
    TranscodeError, TranscodeManager, VideoEncode, VideoEncoder,
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

/// `seconds` of H.264 with a keyframe every 2 s and AC3 audio, in MKV.
fn make_clip(dir: &Path, seconds: u32) -> PathBuf {
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
            &seconds.to_string(),
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

/// The first packet time of `stream` ("v" or "a") and the video height of a segment, read
/// through `init` the way a player does: with the init segment it was served first, even
/// when ffmpeg has been restarted since.
fn probe_segment(ffprobe: &Path, init: &Path, segment: &Path, stream: &str) -> (f64, i64) {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", stream])
        .args(["-show_entries", "packet=pts_time:stream=height"])
        .args(["-of", "csv=p=0", "-i"])
        .arg(format!("concat:{}|{}", init.display(), segment.display()))
        .output()
        .expect("ffprobe segment");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // One line per packet, then the stream's height last (empty for audio).
    let mut lines: Vec<&str> = text.lines().map(str::trim).collect();
    let height = lines
        .pop()
        .and_then(|l| l.trim_end_matches(',').parse::<i64>().ok())
        .unwrap_or_default();
    let first_pts = lines
        .iter()
        .filter_map(|l| l.trim_end_matches(',').parse::<f64>().ok())
        .fold(f64::MAX, f64::min);
    (first_pts, height)
}

/// Saves the session's init segment as the player gets it.
async fn served_init(manager: &TranscodeManager, id: &str, dir: &Path) -> PathBuf {
    let bytes = manager.init_segment(id, 7).await.expect("init");
    assert!(bytes.windows(4).any(|w| w == b"moov"));
    let path = dir.join(format!("served-init-{id}.mp4"));
    std::fs::write(&path, bytes).expect("write init");
    path
}

/// A full transcode, scaled down, on the processor and (when this PC has one) the NVIDIA GPU.
#[tokio::test]
async fn transcode_session_scales_and_cuts_fixed_segments() {
    let (Some(ffmpeg), Some(ffprobe)) = (tool("ffmpeg"), tool("ffprobe")) else {
        eprintln!("ffmpeg/ffprobe not on PATH; skipping");
        return;
    };
    let base =
        std::env::temp_dir().join(format!("couchside-transcode-test-{}", std::process::id()));
    std::fs::create_dir_all(&base).expect("temp dir");
    let clip = make_clip(&base, 60);
    let segs = fixed_segments(60.0);
    assert_eq!(segs.len(), 10);

    for hardware in [HardwareAccel::None, HardwareAccel::Nvenc] {
        let temp_dir = base.join(format!("transcodes-{hardware:?}"));
        let manager = TranscodeManager::new(ManagerSettings {
            ffmpeg: Some(ffmpeg.clone()),
            temp_dir: temp_dir.clone(),
            max_remuxes: 4,
            max_transcodes: 1,
            hardware,
            temp_max_bytes: u64::MAX,
        });
        manager.check_gpu().await;
        if hardware == HardwareAccel::Nvenc && manager.video_encoder() != VideoEncoder::Gpu {
            eprintln!("no NVIDIA encoder here; skipping the GPU run");
            continue;
        }
        let request = |id: &str, device: &str| StreamRequest {
            session_id: id.into(),
            user_id: 7,
            device_id: device.into(),
            input: clip.clone(),
            mode: PlaybackMode::Transcode,
            video: Some(VideoEncode {
                height: Some(144),
                source_height: Some(180),
                hdr: None,
            }),
            video_stream: 0,
            hevc: false,
            audio_stream: Some(1),
            segments: segs.clone(),
            start_seconds: 0.0,
        };
        manager.start(request("abc", "phone")).await.expect("start");
        let init = served_init(&manager, "abc", &base).await;
        let first = manager.segment("abc", 7, 0).await.expect("segment 0");
        let (pts, height) = probe_segment(&ffprobe, &init, &first, "v");
        assert_eq!(height, 144, "{hardware:?}");
        // At the very start the muxer shifts everything by the encoder's B-frame delay (a
        // few frames) to keep timestamps positive; audio moves with it.
        assert!(pts < 0.15, "{hardware:?}: segment 0 starts at {pts}");

        // Seeking far ahead (and back) restarts ffmpeg exactly on the segment start, and its
        // segments still fit the first init segment, for picture and sound.
        for (n, at) in [(9, 54.0), (7, 42.0)] {
            let seg = manager.segment("abc", 7, n).await.expect("segment");
            for stream in ["v", "a"] {
                let (pts, _) = probe_segment(&ffprobe, &init, &seg, stream);
                assert!(
                    (pts - at).abs() < 0.05,
                    "{hardware:?}: segment {n} ({stream}) starts at {pts}"
                );
            }
        }

        // The conversion limit is separate from the repackaging limit.
        assert!(matches!(
            manager.start(request("def", "tv")).await,
            Err(TranscodeError::Busy(1))
        ));
        manager.shutdown().await;
        assert!(!temp_dir.join("cs-abc").exists());
    }
    std::fs::remove_dir_all(&base).ok();
}

#[tokio::test]
async fn audio_convert_session_serves_and_cleans_up() {
    let (Some(ffmpeg), Some(ffprobe)) = (tool("ffmpeg"), tool("ffprobe")) else {
        eprintln!("ffmpeg/ffprobe not on PATH; skipping");
        return;
    };
    let base = std::env::temp_dir().join(format!("couchside-stream-test-{}", std::process::id()));
    std::fs::create_dir_all(&base).expect("temp dir");
    let clip = make_clip(&base, 30);

    let keyframes = read_keyframes(&ffprobe, &clip).await.expect("keyframes");
    assert_eq!(keyframes.len(), 15, "a keyframe every 2 s: {keyframes:?}");
    let segs = segments(&keyframes, 30.0);

    let temp_dir = base.join("transcodes");
    let manager = TranscodeManager::new(ManagerSettings {
        ffmpeg: Some(ffmpeg),
        temp_dir: temp_dir.clone(),
        max_remuxes: 1,
        max_transcodes: 1,
        hardware: HardwareAccel::None,
        temp_max_bytes: u64::MAX,
    });
    let request = |id: &str, device: &str| StreamRequest {
        session_id: id.into(),
        user_id: 7,
        device_id: device.into(),
        input: clip.clone(),
        mode: PlaybackMode::AudioConvert,
        video: None,
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

    let init = served_init(&manager, "abc", &base).await;
    let first = manager.segment("abc", 7, 0).await.expect("segment 0");
    assert!(std::fs::metadata(&first).expect("seg 0").len() > 0);

    // A segment far ahead restarts ffmpeg there; it still fits the first init segment.
    let late = manager.segment("abc", 7, 13).await.expect("segment 13");
    let (first_pts, _) = probe_segment(&ffprobe, &init, &late, "v");
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

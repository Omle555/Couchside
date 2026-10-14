//! Choosing the cheapest way to get a file playing on a client: direct play, remux,
//! audio convert or full transcode (rule 7.4).

use crate::encode::Hdr;
use couchside_core::StreamKind;
use couchside_db::MediaStream;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Highest output TV mode allows.
const TV_MAX_HEIGHT: i64 = 1080;

/// A file container, as far as playback is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    /// MP4 / M4V.
    Mp4,
    /// WebM.
    Webm,
    /// Matroska (MKV).
    Mkv,
    /// Anything else (AVI, WMV, MPEG-TS, MOV...). Never played directly.
    #[serde(other)]
    Other,
}

impl Container {
    /// The container of a file from ffprobe's `format_name` and the file extension
    /// (ffprobe reports MKV and WebM alike as `matroska,webm`).
    pub fn from_probe(format_name: Option<&str>, path: &Path) -> Self {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let format = format_name.unwrap_or_default();
        if format.contains("matroska") {
            if ext == "webm" {
                Self::Webm
            } else {
                Self::Mkv
            }
        } else if format.contains("mp4") && matches!(ext.as_str(), "mp4" | "m4v") {
            Self::Mp4
        } else {
            Self::Other
        }
    }
}

impl fmt::Display for Container {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mp4 => "MP4",
            Self::Webm => "WebM",
            Self::Mkv => "MKV",
            Self::Other => "this kind of",
        })
    }
}

/// A video format, split by bit depth where browsers care about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// H.264, 8-bit.
    H264,
    /// H.264, 10-bit ("Hi10P"). No browser plays it.
    H264High10,
    /// HEVC / H.265, 8-bit.
    Hevc,
    /// HEVC / H.265, 10-bit (most HDR and 4K files).
    HevcMain10,
    /// VP9.
    Vp9,
    /// AV1.
    Av1,
    /// Anything else (MPEG-2, MPEG-4 Part 2/XviD, VC-1, WMV...).
    #[serde(other)]
    Other,
}

impl VideoCodec {
    /// The format of a probed video stream.
    pub fn from_stream(stream: &MediaStream) -> Self {
        let deep = stream.bit_depth.is_some_and(|d| d > 8);
        match (stream.codec.as_deref(), deep) {
            (Some("h264"), false) => Self::H264,
            (Some("h264"), true) => Self::H264High10,
            (Some("hevc"), false) => Self::Hevc,
            (Some("hevc"), true) => Self::HevcMain10,
            (Some("vp9"), _) => Self::Vp9,
            (Some("av1"), _) => Self::Av1,
            _ => Self::Other,
        }
    }
}

/// An audio format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    /// AAC.
    Aac,
    /// MP3.
    Mp3,
    /// Opus.
    Opus,
    /// Vorbis.
    Vorbis,
    /// FLAC.
    Flac,
    /// Dolby Digital.
    Ac3,
    /// Dolby Digital Plus.
    Eac3,
    /// DTS (including DTS-HD).
    Dts,
    /// Dolby TrueHD.
    Truehd,
    /// Anything else (PCM, WMA...).
    #[serde(other)]
    Other,
}

impl AudioCodec {
    /// The format of a probed audio stream.
    pub fn from_stream(stream: &MediaStream) -> Self {
        match stream.codec.as_deref() {
            Some("aac") => Self::Aac,
            Some("mp3") => Self::Mp3,
            Some("opus") => Self::Opus,
            Some("vorbis") => Self::Vorbis,
            Some("flac") => Self::Flac,
            Some("ac3") => Self::Ac3,
            Some("eac3") => Self::Eac3,
            Some("dts") => Self::Dts,
            Some("truehd") => Self::Truehd,
            _ => Self::Other,
        }
    }

    /// Whether the audio can be copied as-is into the fragmented MP4 that remuxing produces.
    fn fits_mp4(self) -> bool {
        matches!(
            self,
            Self::Aac | Self::Mp3 | Self::Opus | Self::Flac | Self::Ac3 | Self::Eac3
        )
    }
}

/// What a client says it can play, sent by the web player before playback.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ClientProfile {
    /// Containers it opens directly.
    pub containers: Vec<Container>,
    /// Video formats it decodes.
    pub video: Vec<VideoCodec>,
    /// Audio formats it decodes.
    pub audio: Vec<AudioCodec>,
    /// Tallest video worth sending (usually the screen height).
    pub max_height: Option<i64>,
    /// "TV mode": only H.264 and AAC/MP3 in MP4, at most 1080p, whatever else is claimed.
    pub tv_mode: bool,
}

impl ClientProfile {
    /// The profile actually used: unknown formats dropped (a client naming one it can play
    /// must not unlock every unknown format), and TV mode narrowing the claims to formats
    /// every TV plays.
    fn effective(&self) -> Self {
        if !self.tv_mode {
            let mut known = self.clone();
            known.containers.retain(|c| *c != Container::Other);
            known.video.retain(|c| *c != VideoCodec::Other);
            known.audio.retain(|c| *c != AudioCodec::Other);
            return known;
        }
        Self {
            containers: vec![Container::Mp4],
            video: vec![VideoCodec::H264],
            audio: vec![AudioCodec::Aac, AudioCodec::Mp3],
            max_height: Some(
                self.max_height
                    .map_or(TV_MAX_HEIGHT, |h| h.min(TV_MAX_HEIGHT)),
            ),
            tv_mode: true,
        }
    }
}

/// The viewer's quality choice in the player.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Quality {
    /// The device's own limit (if any).
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// No height limit: the original size whenever the device plays it.
    #[serde(rename = "original")]
    Original,
    /// At most 1080p.
    #[serde(rename = "1080p")]
    P1080,
    /// At most 720p.
    #[serde(rename = "720p")]
    P720,
    /// At most 480p.
    #[serde(rename = "480p")]
    P480,
}

impl Quality {
    /// `profile` with its height limit set by this choice. A limit below the video's height
    /// makes [`decide`] convert the video down to it.
    pub fn apply(self, profile: &ClientProfile) -> ClientProfile {
        let max_height = match self {
            Self::Auto => profile.max_height,
            Self::Original => None,
            Self::P1080 => Some(1080),
            Self::P720 => Some(720),
            Self::P480 => Some(480),
        };
        ClientProfile {
            max_height,
            ..profile.clone()
        }
    }
}

/// Height a converted video is scaled down to: the client's limit, when the source is taller.
pub fn output_height(source_height: Option<i64>, client: &ClientProfile) -> Option<i64> {
    let max = client.effective().max_height?;
    source_height.filter(|&h| h > max).map(|_| max)
}

/// The HDR kind of a video stream (PQ or HLG), which needs tone mapping to look right as SDR.
pub fn hdr_kind(stream: &MediaStream) -> Option<Hdr> {
    match stream.color_transfer.as_deref() {
        Some("smpte2084") => Some(Hdr::Pq),
        Some("arib-std-b67") => Some(Hdr::Hlg),
        _ => None,
    }
}

/// What is known about the file being played.
#[derive(Debug, Clone, Copy)]
pub struct SourceInfo<'a> {
    /// Container (see [`Container::from_probe`]).
    pub container: Container,
    /// Probed streams, in file order.
    pub streams: &'a [MediaStream],
    /// Video height in pixels.
    pub height: Option<i64>,
}

/// How a file will reach the client, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    /// The original file, as-is.
    DirectPlay,
    /// Video and audio copied into a new container.
    Remux,
    /// Video copied, audio converted to AAC.
    AudioConvert,
    /// Video re-encoded (and audio converted).
    Transcode,
}

/// Why a file can't be played directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The file has not been probed (or ffprobe found nothing), so direct play is attempted.
    NoStreamInfo,
    /// The client can't open the container.
    Container(Container),
    /// The client can't decode the video format (ffprobe codec name).
    VideoCodec(String),
    /// The video is taller than the client wants.
    TooTall {
        /// Source height.
        height: i64,
        /// Client limit.
        max: i64,
    },
    /// The client can't decode the audio format (ffprobe codec name).
    AudioCodec(String),
    /// The audio would play, but can't be copied into MP4 when repackaging.
    AudioNotInMp4(String),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStreamInfo => f.write_str("the file's formats aren't known yet"),
            Self::Container(c) => write!(f, "this device can't open {c} files"),
            Self::VideoCodec(name) => {
                write!(f, "this device can't play {} video", codec_label(name))
            }
            Self::TooTall { height, max } => {
                write!(f, "{height}p video is larger than this device's {max}p")
            }
            Self::AudioCodec(name) => {
                write!(f, "this device can't play {} audio", codec_label(name))
            }
            Self::AudioNotInMp4(name) => {
                write!(f, "{} audio can't be repackaged", codec_label(name))
            }
        }
    }
}

/// A readable name for an ffprobe codec name.
fn codec_label(name: &str) -> String {
    match name {
        "h264" => "H.264".into(),
        "hevc" => "HEVC".into(),
        "mpeg4" => "MPEG-4".into(),
        "mpeg2video" => "MPEG-2".into(),
        "eac3" => "E-AC3".into(),
        "truehd" => "TrueHD".into(),
        other => other.to_ascii_uppercase(),
    }
}

/// The outcome of [`decide`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackDecision {
    /// How the file will be delivered.
    pub mode: PlaybackMode,
    /// Why it isn't direct play (empty for a clean direct play).
    pub reasons: Vec<Reason>,
    /// ffprobe index of the video stream to use.
    pub video_stream: Option<i64>,
    /// ffprobe index of the audio stream to use.
    pub audio_stream: Option<i64>,
}

impl PlaybackDecision {
    /// Records a problem, moving to `mode` if it is more work than the current one.
    fn raise(&mut self, mode: PlaybackMode, reason: Reason) {
        self.mode = self.mode.max(mode);
        self.reasons.push(reason);
    }
}

/// The default stream of a kind, else the first one.
fn pick(streams: &[MediaStream], kind: StreamKind) -> Option<&MediaStream> {
    let mut of_kind = streams.iter().filter(|s| s.kind == kind);
    let first = of_kind.clone().next();
    of_kind.find(|s| s.is_default).or(first)
}

/// Chooses the cheapest playback mode that `client` can play.
pub fn decide(source: &SourceInfo<'_>, client: &ClientProfile) -> PlaybackDecision {
    let client = client.effective();
    let video = pick(source.streams, StreamKind::Video);
    let audio = pick(source.streams, StreamKind::Audio);
    let mut decision = PlaybackDecision {
        mode: PlaybackMode::DirectPlay,
        reasons: Vec::new(),
        video_stream: video.map(|s| s.stream_index),
        audio_stream: audio.map(|s| s.stream_index),
    };
    if video.is_none() && audio.is_none() {
        decision.reasons.push(Reason::NoStreamInfo);
        return decision;
    }

    if let Some(v) = video {
        if !client.video.contains(&VideoCodec::from_stream(v)) {
            decision.raise(
                PlaybackMode::Transcode,
                Reason::VideoCodec(v.codec.clone().unwrap_or_default()),
            );
        }
        if let (Some(height), Some(max)) = (source.height, client.max_height) {
            if height > max {
                decision.raise(PlaybackMode::Transcode, Reason::TooTall { height, max });
            }
        }
    }
    if let Some(a) = audio {
        if !client.audio.contains(&AudioCodec::from_stream(a)) {
            decision.raise(
                PlaybackMode::AudioConvert,
                Reason::AudioCodec(a.codec.clone().unwrap_or_default()),
            );
        }
    }
    if !client.containers.contains(&source.container) {
        decision.raise(PlaybackMode::Remux, Reason::Container(source.container));
        convert_audio_if_not_mp4(&mut decision, audio);
    }
    decision
}

/// Like [`decide`], for a client that tried the original file and couldn't play it even
/// though it claimed it could: the file is repackaged at least.
pub fn decide_after_direct_failure(
    source: &SourceInfo<'_>,
    client: &ClientProfile,
) -> PlaybackDecision {
    let mut decision = decide(source, client);
    if decision.mode == PlaybackMode::DirectPlay {
        decision.mode = PlaybackMode::Remux;
        convert_audio_if_not_mp4(&mut decision, pick(source.streams, StreamKind::Audio));
    }
    decision
}

/// Moves a remux to audio convert when the audio can't be copied into MP4 (e.g. Vorbis).
fn convert_audio_if_not_mp4(decision: &mut PlaybackDecision, audio: Option<&MediaStream>) {
    let Some(audio) = audio else { return };
    if decision.mode == PlaybackMode::Remux && !AudioCodec::from_stream(audio).fits_mp4() {
        decision.raise(
            PlaybackMode::AudioConvert,
            Reason::AudioNotInMp4(audio.codec.clone().unwrap_or_default()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(codec: &str, bit_depth: i64) -> MediaStream {
        MediaStream {
            stream_index: 0,
            kind: StreamKind::Video,
            codec: Some(codec.into()),
            profile: None,
            pix_fmt: None,
            bit_depth: Some(bit_depth),
            color_transfer: None,
            channels: None,
            language: None,
            title: None,
            is_default: false,
            is_forced: false,
        }
    }

    fn audio(index: i64, codec: &str, is_default: bool) -> MediaStream {
        MediaStream {
            stream_index: index,
            kind: StreamKind::Audio,
            bit_depth: None,
            channels: Some(6),
            is_default,
            ..video(codec, 8)
        }
    }

    /// Desktop Chrome on a PC with HEVC hardware decoding.
    fn chrome() -> ClientProfile {
        ClientProfile {
            containers: vec![Container::Mp4, Container::Webm, Container::Mkv],
            video: vec![
                VideoCodec::H264,
                VideoCodec::Hevc,
                VideoCodec::HevcMain10,
                VideoCodec::Vp9,
                VideoCodec::Av1,
            ],
            audio: vec![
                AudioCodec::Aac,
                AudioCodec::Mp3,
                AudioCodec::Opus,
                AudioCodec::Vorbis,
                AudioCodec::Flac,
            ],
            max_height: Some(2160),
            tv_mode: false,
        }
    }

    fn tv() -> ClientProfile {
        ClientProfile {
            tv_mode: true,
            ..chrome()
        }
    }

    fn mode(
        container: Container,
        streams: &[MediaStream],
        height: i64,
        client: &ClientProfile,
    ) -> PlaybackMode {
        let source = SourceInfo {
            container,
            streams,
            height: Some(height),
        };
        decide(&source, client).mode
    }

    /// Name, container, streams, height, client, expected mode.
    type Case = (
        &'static str,
        Container,
        Vec<MediaStream>,
        i64,
        ClientProfile,
        PlaybackMode,
    );

    #[test]
    fn decision_table() {
        use Container::{Mkv, Mp4, Other};
        use PlaybackMode::{AudioConvert, DirectPlay, Remux, Transcode};
        let cases: &[Case] = &[
            (
                "h264/aac mp4",
                Mp4,
                vec![video("h264", 8), audio(1, "aac", false)],
                1080,
                chrome(),
                DirectPlay,
            ),
            (
                "h264/aac mkv",
                Mkv,
                vec![video("h264", 8), audio(1, "aac", false)],
                1080,
                chrome(),
                DirectPlay,
            ),
            (
                "h264/ac3 mkv",
                Mkv,
                vec![video("h264", 8), audio(1, "ac3", false)],
                1080,
                chrome(),
                AudioConvert,
            ),
            (
                "h264/dts mkv",
                Mkv,
                vec![video("h264", 8), audio(1, "dts", false)],
                1080,
                chrome(),
                AudioConvert,
            ),
            (
                "hi10p",
                Mkv,
                vec![video("h264", 10), audio(1, "aac", false)],
                1080,
                chrome(),
                Transcode,
            ),
            (
                "xvid avi",
                Other,
                vec![video("mpeg4", 8), audio(1, "mp3", false)],
                480,
                chrome(),
                Transcode,
            ),
            (
                "4k hevc on 1080 screen",
                Mkv,
                vec![video("hevc", 10), audio(1, "aac", false)],
                2160,
                ClientProfile {
                    max_height: Some(1080),
                    ..chrome()
                },
                Transcode,
            ),
            (
                "h264/aac mkv on tv",
                Mkv,
                vec![video("h264", 8), audio(1, "aac", false)],
                1080,
                tv(),
                Remux,
            ),
            (
                "h264/ac3 mkv on tv",
                Mkv,
                vec![video("h264", 8), audio(1, "ac3", false)],
                1080,
                tv(),
                AudioConvert,
            ),
            (
                "hevc on tv",
                Mkv,
                vec![video("hevc", 8), audio(1, "aac", false)],
                1080,
                tv(),
                Transcode,
            ),
            (
                "h264/aac mp4 on tv",
                Mp4,
                vec![video("h264", 8), audio(1, "aac", false)],
                1080,
                tv(),
                DirectPlay,
            ),
            (
                "h264/vorbis needs mp4",
                Other,
                vec![video("h264", 8), audio(1, "vorbis", false)],
                720,
                chrome(),
                AudioConvert,
            ),
            (
                "h264/ac3 avi, ac3 ok",
                Other,
                vec![video("h264", 8), audio(1, "ac3", false)],
                720,
                ClientProfile {
                    audio: vec![AudioCodec::Aac, AudioCodec::Ac3],
                    ..chrome()
                },
                Remux,
            ),
            (
                "silent mp4",
                Mp4,
                vec![video("h264", 8)],
                720,
                chrome(),
                DirectPlay,
            ),
        ];
        for (name, container, streams, height, client, expected) in cases {
            assert_eq!(
                mode(*container, streams, *height, client),
                *expected,
                "{name}"
            );
        }
    }

    #[test]
    fn default_audio_stream_wins() {
        let streams = [
            video("h264", 8),
            audio(1, "aac", false),
            audio(2, "dts", true),
        ];
        let source = SourceInfo {
            container: Container::Mkv,
            streams: &streams,
            height: Some(1080),
        };
        let d = decide(&source, &chrome());
        assert_eq!(d.audio_stream, Some(2));
        assert_eq!(d.mode, PlaybackMode::AudioConvert);
        assert_eq!(d.reasons, vec![Reason::AudioCodec("dts".into())]);
        assert_eq!(d.reasons[0].to_string(), "this device can't play DTS audio");
    }

    #[test]
    fn failed_direct_play_repackages_and_converts_unfit_audio() {
        let source = |streams| SourceInfo {
            container: Container::Mkv,
            streams,
            height: Some(1080),
        };
        let vorbis = [video("h264", 8), audio(1, "vorbis", false)];
        let d = decide_after_direct_failure(&source(&vorbis), &chrome());
        assert_eq!(d.mode, PlaybackMode::AudioConvert);
        assert_eq!(d.reasons, vec![Reason::AudioNotInMp4("vorbis".into())]);

        let aac = [video("h264", 8), audio(1, "aac", false)];
        let d = decide_after_direct_failure(&source(&aac), &chrome());
        assert_eq!(d.mode, PlaybackMode::Remux);
        assert!(d.reasons.is_empty());
    }

    #[test]
    fn quality_choice_sets_the_height_limit() {
        let streams = [video("h264", 8), audio(1, "aac", false)];
        let source = SourceInfo {
            container: Container::Mp4,
            streams: &streams,
            height: Some(1080),
        };
        let phone = ClientProfile {
            max_height: Some(1080),
            ..chrome()
        };
        let mode_at = |q: Quality| decide(&source, &q.apply(&phone)).mode;
        assert_eq!(mode_at(Quality::Auto), PlaybackMode::DirectPlay);
        assert_eq!(mode_at(Quality::P1080), PlaybackMode::DirectPlay);
        assert_eq!(mode_at(Quality::P720), PlaybackMode::Transcode);
        assert_eq!(Quality::Original.apply(&phone).max_height, None);
        assert_eq!(
            output_height(Some(1080), &Quality::P720.apply(&phone)),
            Some(720)
        );
        assert_eq!(output_height(Some(480), &Quality::P720.apply(&phone)), None);
        assert_eq!(output_height(Some(2160), &tv()), Some(1080));
        assert_eq!(
            output_height(Some(2160), &Quality::Original.apply(&phone)),
            None
        );
        let q: Quality = serde_json::from_str("\"720p\"").expect("parses");
        assert_eq!(q, Quality::P720);
    }

    #[test]
    fn hdr_is_pq_or_hlg() {
        let mut v = video("hevc", 10);
        assert_eq!(hdr_kind(&v), None);
        v.color_transfer = Some("smpte2084".into());
        assert_eq!(hdr_kind(&v), Some(Hdr::Pq));
        v.color_transfer = Some("arib-std-b67".into());
        assert_eq!(hdr_kind(&v), Some(Hdr::Hlg));
    }

    #[test]
    fn unprobed_file_tries_direct_play() {
        let source = SourceInfo {
            container: Container::Other,
            streams: &[],
            height: None,
        };
        let d = decide(&source, &chrome());
        assert_eq!(d.mode, PlaybackMode::DirectPlay);
        assert_eq!(d.reasons, vec![Reason::NoStreamInfo]);
    }

    #[test]
    fn tv_mode_caps_height_at_1080() {
        let streams = [video("h264", 8), audio(1, "aac", false)];
        let client = ClientProfile {
            max_height: Some(2160),
            ..tv()
        };
        assert_eq!(
            mode(Container::Mp4, &streams, 2160, &client),
            PlaybackMode::Transcode
        );
        let small = ClientProfile {
            max_height: Some(720),
            ..tv()
        };
        assert_eq!(
            mode(Container::Mp4, &streams, 1080, &small),
            PlaybackMode::Transcode
        );
    }

    #[test]
    fn container_from_probe() {
        let c = |format: &str, file: &str| Container::from_probe(Some(format), Path::new(file));
        assert_eq!(c("matroska,webm", "a.mkv"), Container::Mkv);
        assert_eq!(c("matroska,webm", "a.WEBM"), Container::Webm);
        assert_eq!(c("mov,mp4,m4a,3gp,3g2,mj2", "a.mp4"), Container::Mp4);
        assert_eq!(c("mov,mp4,m4a,3gp,3g2,mj2", "a.mov"), Container::Other);
        assert_eq!(c("avi", "a.avi"), Container::Other);
        assert_eq!(
            Container::from_probe(None, Path::new("a.mp4")),
            Container::Other
        );
    }

    #[test]
    fn profile_from_json_ignores_unknown_formats() {
        let profile: ClientProfile = serde_json::from_str(
            r#"{"containers":["mp4","mkv","ogg"],"video":["h264","hevc_main10","vvc"],
                "audio":["aac","alac"],"max_height":1080}"#,
        )
        .expect("parses");
        assert_eq!(
            profile.containers,
            vec![Container::Mp4, Container::Mkv, Container::Other]
        );
        assert_eq!(
            profile.video,
            vec![VideoCodec::H264, VideoCodec::HevcMain10, VideoCodec::Other]
        );
        assert!(!profile.tv_mode);

        // "ogg" and "vvc" became Other, which must not let an AVI or XviD play directly.
        let streams = [video("h264", 8), audio(1, "aac", false)];
        assert_eq!(
            mode(Container::Other, &streams, 480, &profile),
            PlaybackMode::Remux
        );
        let xvid = [video("mpeg4", 8)];
        let claims_unknown = ClientProfile {
            containers: vec![Container::Other],
            video: vec![VideoCodec::Other],
            ..profile
        };
        assert_eq!(
            mode(Container::Other, &xvid, 480, &claims_unknown),
            PlaybackMode::Transcode
        );
    }
}

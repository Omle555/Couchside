//! Choosing how to play an item, and serving repackaged and converted streams as HLS.

use super::auth::AuthUser;
use super::error::capitalize;
use super::stream::{safe_media_path, serve_file};
use super::ApiError;
use crate::app::AppState;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use couchside_core::MediaId;
use couchside_db::MediaItem;
use couchside_transcode::{
    decide, decide_after_direct_failure, fixed_segments, hdr_kind, output_height, read_keyframes,
    segments as keyframe_segments, ClientProfile, Container, PlaybackDecision, PlaybackMode,
    Quality, SourceInfo, StreamRequest, VideoEncode, VideoEncoder,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `POST /items/{id}/playback` body.
#[derive(Debug, Deserialize)]
pub struct PlaybackRequest {
    /// What the device can play.
    #[serde(default)]
    pub profile: ClientProfile,
    /// Random id the device keeps; a new stream from it ends its previous one.
    pub device_id: String,
    /// Where playback will start.
    #[serde(default)]
    pub start_seconds: f64,
    /// The original file was tried and didn't play.
    #[serde(default)]
    pub direct_failed: bool,
    /// The viewer's quality choice.
    #[serde(default)]
    pub quality: Quality,
}

/// How to play an item.
#[derive(Debug, Serialize)]
pub struct PlaybackDto {
    /// What Couchside does to the file.
    pub mode: PlaybackMode,
    /// What to load: the original file or an HLS playlist.
    pub url: String,
    /// Stream id, to stop it when the viewer leaves.
    pub session_id: Option<String>,
    /// Why the file isn't played as it is.
    pub reason: Option<String>,
    /// The video must be converted but can't be (FFmpeg missing or the file not probed).
    pub needs_video_conversion: bool,
    /// The video is converted on the NVIDIA GPU.
    pub gpu: bool,
}

fn direct(item: &MediaItem, reason: Option<String>, needs_video_conversion: bool) -> PlaybackDto {
    PlaybackDto {
        mode: PlaybackMode::DirectPlay,
        url: format!("/api/v1/items/{}/stream", item.id.0),
        session_id: None,
        reason,
        needs_video_conversion,
        gpu: false,
    }
}

fn reason_text(decision: &PlaybackDecision, direct_failed: bool) -> Option<String> {
    let mut parts: Vec<String> = decision.reasons.iter().map(ToString::to_string).collect();
    if direct_failed {
        parts.push("the original file didn't play in this browser".into());
    }
    (!parts.is_empty()).then(|| capitalize(&parts.join("; ")))
}

/// `POST /items/{id}/playback` — picks direct play, remux, audio convert or transcode and
/// starts the stream if one is needed.
pub async fn start(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(id): Path<i64>,
    Json(req): Json<PlaybackRequest>,
) -> Result<Json<PlaybackDto>, ApiError> {
    let item = state
        .db
        .get_item(MediaId(id))
        .await?
        .ok_or(ApiError::NotFound)?;
    let path = safe_media_path(&state, &item.path).await?;
    let streams = state.db.streams_for(item.id).await?;
    let source = SourceInfo {
        container: Container::from_probe(item.container.as_deref(), &path),
        streams: &streams,
        height: item.height,
    };
    let profile = req.quality.apply(&req.profile);
    let first = decide(&source, &profile);
    let forced = req.direct_failed && first.mode == PlaybackMode::DirectPlay;
    let decision = if forced {
        decide_after_direct_failure(&source, &profile)
    } else {
        first
    };
    let mode = decision.mode;
    let reason = reason_text(&decision, forced);
    if mode == PlaybackMode::DirectPlay {
        return Ok(Json(direct(&item, None, false)));
    }
    let transcode = mode == PlaybackMode::Transcode;
    let (Some(video_stream), Some(duration)) = (decision.video_stream, item.runtime_seconds) else {
        tracing::info!(item = id, "can't convert without probed media info");
        return Ok(Json(direct(&item, reason, transcode)));
    };
    if !state.transcoder.available() {
        return Ok(Json(direct(&item, reason, transcode)));
    }

    let (segments, video) = if transcode {
        let hdr = streams
            .iter()
            .find(|s| s.stream_index == video_stream)
            .and_then(hdr_kind);
        let video = VideoEncode {
            height: output_height(item.height, &profile),
            source_height: item.height,
            hdr,
        };
        (fixed_segments(duration), Some(video))
    } else {
        let Some(ffprobe) = state.ffprobe.as_ref() else {
            tracing::info!(item = id, "can't repackage without ffprobe");
            return Ok(Json(direct(&item, reason, false)));
        };
        let keyframes = match state.db.keyframes(item.id).await? {
            Some(times) => times,
            None => {
                let times = read_keyframes(ffprobe.path(), &path).await?;
                state.db.set_keyframes(item.id, &times).await?;
                times
            }
        };
        (keyframe_segments(&keyframes, duration), None)
    };
    let gpu = transcode && state.transcoder.video_encoder() == VideoEncoder::Gpu;
    let mut raw = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let session_id = hex::encode(raw);
    state
        .transcoder
        .start(StreamRequest {
            session_id: session_id.clone(),
            user_id: user.id.0,
            device_id: req.device_id.chars().take(64).collect(),
            input: PathBuf::from(&path),
            mode,
            video,
            video_stream,
            hevc: streams
                .iter()
                .any(|s| s.stream_index == video_stream && s.codec.as_deref() == Some("hevc")),
            audio_stream: decision.audio_stream,
            segments,
            start_seconds: req.start_seconds.max(0.0),
        })
        .await?;
    Ok(Json(PlaybackDto {
        mode,
        url: format!("/api/v1/transcode/{session_id}/main.m3u8"),
        session_id: Some(session_id),
        reason,
        needs_video_conversion: false,
        gpu,
    }))
}

/// `GET /transcode/{session}/main.m3u8`
pub async fn playlist(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(session): Path<String>,
) -> Result<Response, ApiError> {
    let text = state.transcoder.playlist(&session, user.id.0)?;
    Ok((
        [
            (CONTENT_TYPE, "application/vnd.apple.mpegurl"),
            (CACHE_CONTROL, "no-cache"),
        ],
        text,
    )
        .into_response())
}

/// `GET /transcode/{session}/init.mp4`
pub async fn init_segment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(session): Path<String>,
) -> Result<Response, ApiError> {
    let bytes = state.transcoder.init_segment(&session, user.id.0).await?;
    Ok(([(CONTENT_TYPE, "video/mp4")], bytes).into_response())
}

/// `GET /transcode/{session}/seg/{n}.m4s`
pub async fn segment(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((session, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let n: usize = file
        .strip_suffix(".m4s")
        .and_then(|n| n.parse().ok())
        .ok_or(ApiError::NotFound)?;
    let path = state.transcoder.segment(&session, user.id.0, n).await?;
    serve_file(&path, &headers, "video/mp4").await
}

/// `POST /transcode/{session}/stop` — the viewer left (also sent with `sendBeacon`).
pub async fn stop(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(session): Path<String>,
) -> StatusCode {
    state.transcoder.stop_for_user(&session, user.id.0).await;
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use couchside_transcode::Reason;

    #[test]
    fn reasons_read_as_a_sentence() {
        let decision = PlaybackDecision {
            mode: PlaybackMode::AudioConvert,
            reasons: vec![Reason::AudioCodec("dts".into())],
            video_stream: Some(0),
            audio_stream: Some(1),
        };
        assert_eq!(
            reason_text(&decision, false).as_deref(),
            Some("This device can't play DTS audio.")
        );
        let clean = PlaybackDecision {
            reasons: Vec::new(),
            ..decision
        };
        assert_eq!(reason_text(&clean, false), None);
        assert_eq!(
            reason_text(&clean, true).as_deref(),
            Some("The original file didn't play in this browser.")
        );
    }
}

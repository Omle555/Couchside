//! Streaming sessions: one ffmpeg writing HLS segments for one viewer (rule 7.3).
//!
//! ffmpeg starts at the segment the viewer starts from and runs a little faster than
//! real time. A request for a segment far ahead of it (a seek) restarts it there. ffmpeg is
//! stopped when the viewer leaves, after [`ENCODER_IDLE`] without requests, and when the
//! server stops; a session's folder is deleted when the session ends (rule 7.6).

use crate::decision::PlaybackMode;
use crate::encode::{nvenc_works, VideoEncode};
use crate::process::new_command;
use crate::segments::{playlist, seek_time, Segment};
use crate::temp::SESSION_DIR_PREFIX;
use crate::TranscodeError;
use couchside_core::config::HardwareAccel;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Child;

/// ffmpeg is stopped after this long without a request (the viewer paused or left).
const ENCODER_IDLE: Duration = Duration::from_secs(120);
/// A session is ended after this long without a request.
const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
/// A request for a segment more than this far ahead of ffmpeg restarts ffmpeg there.
const RESTART_AHEAD_SECONDS: f64 = 20.0;
/// Longest a request waits for its segment.
const SEGMENT_WAIT: Duration = Duration::from_secs(60);
/// How often a waiting request checks for its segment.
const POLL: Duration = Duration::from_millis(100);
/// How often idle encoders and sessions are looked for.
const REAP_EVERY: Duration = Duration::from_secs(10);
/// Sessions used in the last minute are never ended to free disk space.
const RECENT_USE: Duration = Duration::from_secs(60);

/// Settings for the [`TranscodeManager`].
#[derive(Debug, Clone)]
pub struct ManagerSettings {
    /// ffmpeg executable, if one was found.
    pub ffmpeg: Option<PathBuf>,
    /// Folder holding the session folders.
    pub temp_dir: PathBuf,
    /// Most remux / audio-convert sessions at once.
    pub max_remuxes: u32,
    /// Most full transcodes at once.
    pub max_transcodes: u32,
    /// Hardware that encodes video.
    pub hardware: HardwareAccel,
    /// Bytes the session folders may use before idle sessions are ended.
    pub temp_max_bytes: u64,
}

/// What to stream and how.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    /// Random id for the session (letters and digits only).
    pub session_id: String,
    /// Who is watching.
    pub user_id: i64,
    /// The viewer's device; starting a new stream ends that device's previous one.
    pub device_id: String,
    /// The video file.
    pub input: PathBuf,
    /// Remux, audio convert or transcode.
    pub mode: PlaybackMode,
    /// How to re-encode the video (transcode only; video is copied otherwise).
    pub video: Option<VideoEncode>,
    /// ffprobe index of the video stream.
    pub video_stream: i64,
    /// The video is HEVC: tag it `hvc1`, the form browsers and TVs expect in MP4.
    pub hevc: bool,
    /// ffprobe index of the audio stream, if any.
    pub audio_stream: Option<i64>,
    /// Segments of the whole video.
    pub segments: Vec<Segment>,
    /// Where playback will start, in seconds.
    pub start_seconds: f64,
}

/// The running ffmpeg of a session.
struct Encoder {
    child: Child,
    /// First segment this ffmpeg writes.
    start: usize,
    /// Highest segment known to be complete (`None` until the first one is).
    done: Option<usize>,
    /// Last lines ffmpeg printed (errors only, as it runs with `-loglevel error`).
    errors: Arc<Mutex<String>>,
    /// Encoding video on the GPU.
    gpu: bool,
}

struct Session {
    req: StreamRequest,
    dir: PathBuf,
    playlist: String,
    encoder: tokio::sync::Mutex<Option<Encoder>>,
    last_access: Mutex<Instant>,
    init: tokio::sync::OnceCell<Vec<u8>>,
    /// The GPU failed for this stream: encode on the processor from now on.
    cpu_only: AtomicBool,
}

/// What encodes video when a file must be converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    /// The NVIDIA GPU (NVENC).
    Gpu,
    /// The processor (x264), as configured.
    Cpu,
    /// The processor, because the NVIDIA GPU was switched on but failed its test.
    GpuFailed,
    /// Nothing: ffmpeg wasn't found.
    Unavailable,
}

/// Starts, tracks and stops streaming sessions.
pub struct TranscodeManager {
    settings: ManagerSettings,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// The NVENC self-test failed.
    gpu_failed: AtomicBool,
    #[cfg(windows)]
    job: Option<crate::process::KillOnCloseJob>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A panic while holding one of these locks leaves plain data behind; keep going.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Session {
    fn touch(&self) {
        *lock(&self.last_access) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        lock(&self.last_access).elapsed()
    }

    fn segment_path(&self, n: usize) -> PathBuf {
        self.dir.join(format!("seg-{n}.m4s"))
    }

    /// The segment the viewer starts on.
    fn first_segment(&self) -> usize {
        let start = self.req.start_seconds;
        self.req
            .segments
            .iter()
            .rposition(|s| s.start <= start)
            .unwrap_or(0)
    }

    /// ffmpeg arguments to write segments from `start` on, encoding video on the GPU if `gpu`.
    fn ffmpeg_args(&self, start: usize, gpu: bool) -> Vec<String> {
        let video = self.req.video.as_ref();
        let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error"]
            .map(String::from)
            .to_vec();
        if let Some(v) = video {
            args.extend(v.input_args(gpu));
        }
        if start > 0 {
            if video.is_some() {
                // Re-encoding can start on any frame, so seek exactly to the segment start.
                let at = self.req.segments.get(start).map_or(0.0, |s| s.start);
                args.extend(["-ss".into(), format!("{at:.3}")]);
            } else {
                let at = seek_time(&self.req.segments, start).unwrap_or_default();
                args.extend(["-ss".into(), format!("{at:.3}"), "-noaccurate_seek".into()]);
            }
        }
        // Read ahead quickly at first, then a little faster than playback (converting video
        // is costly) or at four times real time (copying is cheap).
        let rate = if video.is_some() { "1.5" } else { "4" };
        args.extend(["-readrate", rate, "-readrate_initial_burst", "30"].map(String::from));
        args.push("-i".into());
        args.push(self.req.input.to_string_lossy().into_owned());
        // Keep the file's own timestamps so segments from different runs line up.
        args.extend(["-copyts", "-start_at_zero"].map(String::from));
        args.extend(["-map".into(), format!("0:{}", self.req.video_stream)]);
        if let Some(audio) = self.req.audio_stream {
            args.extend(["-map".into(), format!("0:{audio}")]);
        }
        match video {
            Some(v) => args.extend(v.args(gpu)),
            None => {
                args.extend(["-c:v", "copy"].map(String::from));
                if self.req.hevc {
                    args.extend(["-tag:v", "hvc1"].map(String::from));
                }
            }
        }
        match self.req.mode {
            PlaybackMode::Remux => args.extend(["-c:a", "copy"].map(String::from)),
            _ => args.extend(["-c:a", "aac", "-ac", "2", "-b:a", "192k"].map(String::from)),
        }
        args.extend(
            [
                "-sn",
                "-dn",
                "-max_muxing_queue_size",
                "4096",
                "-f",
                "hls",
                // Cut at every keyframe: see `segments.rs` and `encode.rs`.
                "-hls_time",
                "0.001",
                "-hls_segment_type",
                "fmp4",
                "-hls_fmp4_init_filename",
                "init.mp4",
                // Fragment times from the file's own timestamps, not from where this ffmpeg
                // started: a restarted ffmpeg's segments then fit the init segment the player
                // already has (served once per session).
                "-hls_segment_options",
                "movflags=+frag_discont",
                "-hls_flags",
                "temp_file",
                "-hls_list_size",
                "0",
                "-hls_playlist_type",
                "event",
            ]
            .map(String::from),
        );
        args.extend(["-start_number".into(), start.to_string()]);
        args.push("-hls_segment_filename".into());
        // Output names are relative: ffmpeg runs inside the session folder (it would put
        // init.mp4 in its working folder otherwise).
        args.push("seg-%d.m4s".into());
        args.push("ffmpeg.m3u8".into());
        args
    }
}

impl Encoder {
    /// Moves `done` forward over segments ffmpeg has finished (it renames each one into place
    /// only when complete).
    fn advance(&mut self, session: &Session) {
        let mut next = self.done.map_or(self.start, |d| d + 1);
        while session.segment_path(next).is_file() {
            self.done = Some(next);
            next += 1;
        }
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// ffmpeg reached the end of the file without an error.
    fn finished(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(status)) if status.success())
    }

    fn errors(&self) -> String {
        lock(&self.errors).trim().to_string()
    }
}

impl TranscodeManager {
    /// Creates the manager and starts its background clean-up. Call from a Tokio runtime.
    pub fn new(settings: ManagerSettings) -> Arc<Self> {
        let manager = Arc::new(Self {
            settings,
            sessions: Mutex::new(HashMap::new()),
            gpu_failed: AtomicBool::new(false),
            #[cfg(windows)]
            job: match crate::process::KillOnCloseJob::new() {
                Ok(job) => Some(job),
                Err(err) => {
                    tracing::warn!(error = %err, "could not create the ffmpeg job object");
                    None
                }
            },
        });
        tokio::spawn(reap_loop(Arc::downgrade(&manager)));
        manager
    }

    /// How many repackaged or converted streams are open.
    pub fn open_sessions(&self) -> usize {
        lock(&self.sessions).len()
    }

    /// Whether streams can be repackaged at all (ffmpeg was found).
    pub fn available(&self) -> bool {
        self.settings.ffmpeg.is_some()
    }

    /// What encodes video for new streams.
    pub fn video_encoder(&self) -> VideoEncoder {
        match (self.available(), self.settings.hardware) {
            (false, _) => VideoEncoder::Unavailable,
            (true, HardwareAccel::None) => VideoEncoder::Cpu,
            (true, HardwareAccel::Nvenc) if self.gpu_failed.load(Ordering::Relaxed) => {
                VideoEncoder::GpuFailed
            }
            (true, HardwareAccel::Nvenc) => VideoEncoder::Gpu,
        }
    }

    /// Tests the NVIDIA encoder once when it is switched on; if it fails, video is converted
    /// on the processor instead.
    pub async fn check_gpu(&self) {
        let Some(ffmpeg) = &self.settings.ffmpeg else {
            return;
        };
        if self.settings.hardware != HardwareAccel::Nvenc {
            return;
        }
        if nvenc_works(ffmpeg).await {
            tracing::info!("NVIDIA video encoder ready");
        } else {
            tracing::warn!("NVIDIA video encoder unavailable; converting video on the processor");
            self.gpu_failed.store(true, Ordering::Relaxed);
        }
    }

    /// Starts a session and its ffmpeg. Ends the device's previous session first.
    pub async fn start(&self, req: StreamRequest) -> Result<(), TranscodeError> {
        let ffmpeg = self
            .settings
            .ffmpeg
            .clone()
            .ok_or(TranscodeError::NoFfmpeg)?;
        if req.session_id.is_empty() || !req.session_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(TranscodeError::Failed("bad session id".into()));
        }
        if req.segments.is_empty() {
            return Err(TranscodeError::Failed("no segments".into()));
        }
        let replaced: Vec<String> = lock(&self.sessions)
            .values()
            .filter(|s| s.req.user_id == req.user_id && s.req.device_id == req.device_id)
            .map(|s| s.req.session_id.clone())
            .collect();
        for id in replaced {
            self.stop(&id).await;
        }
        let transcode = req.video.is_some();
        let limit = if transcode {
            self.settings.max_transcodes
        } else {
            self.settings.max_remuxes
        };
        let running = lock(&self.sessions)
            .values()
            .filter(|s| s.req.video.is_some() == transcode)
            .count();
        if running >= usize::try_from(limit).unwrap_or(usize::MAX) {
            return Err(TranscodeError::Busy(limit));
        }

        let dir = self
            .settings
            .temp_dir
            .join(format!("{SESSION_DIR_PREFIX}{}", req.session_id));
        tokio::fs::create_dir_all(&dir).await?;
        let session = Arc::new(Session {
            playlist: playlist(&req.segments),
            dir,
            req,
            encoder: tokio::sync::Mutex::new(None),
            last_access: Mutex::new(Instant::now()),
            init: tokio::sync::OnceCell::new(),
            cpu_only: AtomicBool::new(false),
        });
        let first = session.first_segment();
        {
            let mut encoder = session.encoder.lock().await;
            *encoder = Some(self.spawn(&ffmpeg, &session, first)?);
        }
        tracing::info!(
            session = %session.req.session_id,
            mode = ?session.req.mode,
            file = %session.req.input.display(),
            "stream started"
        );
        lock(&self.sessions).insert(session.req.session_id.clone(), session);
        Ok(())
    }

    fn spawn(
        &self,
        ffmpeg: &Path,
        session: &Session,
        start: usize,
    ) -> Result<Encoder, TranscodeError> {
        let gpu = session.req.video.is_some()
            && self.video_encoder() == VideoEncoder::Gpu
            && !session.cpu_only.load(Ordering::Relaxed);
        let mut cmd = new_command(ffmpeg);
        cmd.current_dir(&session.dir)
            .stdout(std::process::Stdio::null())
            .args(session.ffmpeg_args(start, gpu));
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        if let Some(job) = &self.job {
            if let Err(err) = job.assign(&child) {
                tracing::warn!(error = %err, "could not add ffmpeg to the job object");
            }
        }
        let errors = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = child.stderr.take() {
            // Drain stderr so ffmpeg never blocks on a full pipe; keep the tail for the log.
            let sink = Arc::clone(&errors);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut text = lock(&sink);
                    text.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if text.len() > 2000 {
                        let cut = text.len() - 1000;
                        let cut = (cut..text.len())
                            .find(|&i| text.is_char_boundary(i))
                            .unwrap_or(0);
                        text.drain(..cut);
                    }
                }
            });
        }
        tracing::debug!(session = %session.req.session_id, start, gpu, "ffmpeg started");
        Ok(Encoder {
            child,
            start,
            done: None,
            errors,
            gpu,
        })
    }

    fn get(&self, id: &str, user_id: i64) -> Result<Arc<Session>, TranscodeError> {
        let session = lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or(TranscodeError::NotFound)?;
        if session.req.user_id != user_id {
            return Err(TranscodeError::NotFound);
        }
        session.touch();
        Ok(session)
    }

    /// The session's playlist.
    pub fn playlist(&self, id: &str, user_id: i64) -> Result<String, TranscodeError> {
        Ok(self.get(id, user_id)?.playlist.clone())
    }

    /// The session's init segment (codec set-up), once ffmpeg has written it.
    pub async fn init_segment(&self, id: &str, user_id: i64) -> Result<Vec<u8>, TranscodeError> {
        let session = self.get(id, user_id)?;
        let bytes = session
            .init
            .get_or_try_init(|| async {
                // The init segment is complete once the first media segment exists.
                let first = session.encoder.lock().await.as_ref().map_or(0, |e| e.start);
                self.wait_for(&session, first).await?;
                Ok::<_, TranscodeError>(tokio::fs::read(session.dir.join("init.mp4")).await?)
            })
            .await?;
        Ok(bytes.clone())
    }

    /// Path of media segment `n`, waiting for ffmpeg (and restarting it there if it is far
    /// away) as needed.
    pub async fn segment(
        &self,
        id: &str,
        user_id: i64,
        n: usize,
    ) -> Result<PathBuf, TranscodeError> {
        let session = self.get(id, user_id)?;
        if n >= session.req.segments.len() {
            return Err(TranscodeError::NotFound);
        }
        self.wait_for(&session, n).await?;
        Ok(session.segment_path(n))
    }

    async fn wait_for(&self, session: &Session, n: usize) -> Result<(), TranscodeError> {
        let deadline = Instant::now() + SEGMENT_WAIT;
        loop {
            if session.segment_path(n).is_file() {
                return Ok(());
            }
            self.ensure_encoder_near(session, n).await?;
            if Instant::now() > deadline {
                return Err(TranscodeError::Timeout);
            }
            tokio::time::sleep(POLL).await;
            session.touch();
        }
    }

    /// Makes sure an ffmpeg is running that will reach segment `n` soon.
    async fn ensure_encoder_near(&self, session: &Session, n: usize) -> Result<(), TranscodeError> {
        let ffmpeg = self
            .settings
            .ffmpeg
            .clone()
            .ok_or(TranscodeError::NoFfmpeg)?;
        let mut slot = session.encoder.lock().await;
        if let Some(enc) = slot.as_mut() {
            enc.advance(session);
            if session.segment_path(n).is_file() {
                return Ok(());
            }
            let next = enc.done.map_or(enc.start, |d| d + 1);
            let near = n >= enc.start
                && session.req.segments[n].start
                    <= session.req.segments[next.min(n)].start + RESTART_AHEAD_SECONDS;
            if near && enc.running() {
                return Ok(());
            }
            if near && n == next {
                if session.segment_path(n).is_file() {
                    return Ok(());
                }
                if enc.finished() {
                    // The file ended before this segment (its last moments held no frame).
                    return Err(TranscodeError::NotFound);
                }
                // ffmpeg stopped right before this segment: it failed, and would again.
                let errors = enc.errors();
                let enc_was_gpu = enc.gpu;
                *slot = None;
                if enc_was_gpu {
                    // Rule 7.5: the same job again on the processor.
                    tracing::warn!(
                        session = %session.req.session_id,
                        error = %errors,
                        "NVIDIA encoder failed; converting on the processor instead"
                    );
                    session.cpu_only.store(true, Ordering::Relaxed);
                    *slot = Some(self.spawn(&ffmpeg, session, n)?);
                    return Ok(());
                }
                return Err(TranscodeError::Failed(if errors.is_empty() {
                    "ffmpeg stopped early".into()
                } else {
                    errors
                }));
            }
        }
        // Dropping the old encoder kills its ffmpeg.
        *slot = None;
        *slot = Some(self.spawn(&ffmpeg, session, n)?);
        Ok(())
    }

    /// Ends a session: stops its ffmpeg and deletes its folder.
    pub async fn stop(&self, id: &str) {
        let Some(session) = lock(&self.sessions).remove(id) else {
            return;
        };
        if let Some(mut enc) = session.encoder.lock().await.take() {
            if let Err(err) = enc.child.kill().await {
                tracing::debug!(error = %err, "ffmpeg had already stopped");
            }
        }
        if let Err(err) = tokio::fs::remove_dir_all(&session.dir).await {
            tracing::warn!(dir = %session.dir.display(), error = %err, "could not remove stream folder");
        }
        tracing::info!(session = %id, "stream ended");
    }

    /// Ends a session if it belongs to `user_id`.
    pub async fn stop_for_user(&self, id: &str, user_id: i64) {
        if self.get(id, user_id).is_ok() {
            self.stop(id).await;
        }
    }

    /// Ends every session (server shutdown).
    pub async fn shutdown(&self) {
        let ids: Vec<String> = lock(&self.sessions).keys().cloned().collect();
        for id in ids {
            self.stop(&id).await;
        }
    }

    /// Stops idle encoders, ends idle sessions and keeps the folders under the size limit.
    async fn reap(&self) {
        let sessions: Vec<Arc<Session>> = lock(&self.sessions).values().cloned().collect();
        let mut ended = Vec::new();
        for s in &sessions {
            let idle = s.idle_for();
            if idle > SESSION_IDLE {
                ended.push(s.req.session_id.clone());
            } else if idle > ENCODER_IDLE {
                let mut slot = s.encoder.lock().await;
                if slot.as_mut().is_some_and(Encoder::running) {
                    tracing::debug!(session = %s.req.session_id, "stopping idle ffmpeg");
                }
                *slot = None;
            }
        }
        let mut by_age: Vec<(Duration, String, PathBuf)> = sessions
            .iter()
            .filter(|s| !ended.contains(&s.req.session_id))
            .map(|s| (s.idle_for(), s.req.session_id.clone(), s.dir.clone()))
            .collect();
        by_age.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        let dirs: Vec<PathBuf> = by_age.iter().map(|(_, _, d)| d.clone()).collect();
        let mut total: u64 =
            tokio::task::spawn_blocking(move || dirs.iter().map(|d| dir_size(d)).sum())
                .await
                .unwrap_or(0);
        for (idle, id, dir) in by_age {
            if total <= self.settings.temp_max_bytes || idle < RECENT_USE {
                break;
            }
            let size = tokio::task::spawn_blocking(move || dir_size(&dir))
                .await
                .unwrap_or(0);
            tracing::info!(session = %id, "ending idle stream to free disk space");
            total = total.saturating_sub(size);
            ended.push(id);
        }
        for id in ended {
            self.stop(&id).await;
        }
    }
}

/// Total size of the files directly in `dir`.
fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

async fn reap_loop(manager: Weak<TranscodeManager>) {
    let mut tick = tokio::time::interval(REAP_EVERY);
    loop {
        tick.tick().await;
        let Some(manager) = manager.upgrade() else {
            return;
        };
        manager.reap().await;
    }
}

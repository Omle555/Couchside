# Transcoding plan

Status: **agreed** (2026-10-14). Phases 0 and 1 are built. LJ's answers to the open questions
are recorded at the end.

## 1. Where we are today

- Playback is direct play only. `GET /api/v1/items/{id}/stream`
  (`crates/couchside-server/src/api/stream.rs`) serves the original file with Range support.
  MKV is labelled `video/webm` so Chrome tries it.
- The web player (`playerPage` in `crates/couchside-server/src/web/app.js`) is a plain
  `<video src=…/stream>` with external subtitles as `<track>` elements. When the browser can't
  play the file it shows "Couchside doesn't convert video yet… use VLC".
- ffprobe runs per file during scans (`crates/couchside-library/src/probe.rs`) and stores only
  container, first video codec, first audio codec, width and height (`media_items`).
- ffmpeg is already located at startup (`Ffmpeg::locate`, configured path first) and used for
  title-scan frame grabs, with list arguments, timeouts and `kill_on_drop`.
- Auth is a session cookie, so `<video>` and segment requests from the same origin are
  authenticated without extra work.
- Rules that shape this plan: direct play > remux > transcode (7.4); every ffmpeg process tracked
  and killed (7.3); hardware acceleration opt-in with software fallback (7.5); temp files cleaned
  on session end and at startup (7.6); transcoding lives in a new `couchside-transcode` crate
  (2.4, ADR 0001); no new dependencies without asking (18.2).

## 2. The hardware

Checked on the server PC on 2026-10-14:

| Part | Detail | What it means |
|---|---|---|
| GPU | NVIDIA GeForce RTX 3070 (driver 32.0.16.1714) | NVENC/NVDEC: H.264, HEVC (incl. 10-bit) decode and encode. A test `h264_nvenc` encode ran at 6x realtime on a 720p test pattern. Recent drivers allow 8 concurrent NVENC sessions. |
| CPU | Intel Core i7-7700K, 4 cores / 8 threads | Software x264 manages roughly one 1080p transcode in real time at `veryfast`; 4K software transcodes won't keep up. The iGPU (Quick Sync) is not active, so QSV is out. |
| RAM | 16 GB | Fine. |
| FFmpeg | 9.0.2 full build (Gyan) | Has `h264_nvenc`, `hevc_nvenc`, `libx264`, `aac`, `libopus`, hwaccels `cuda`/`d3d11va`, and the filters needed for HDR tone mapping. |

So: **NVENC for video, CPU only as fallback.**

## 3. Clients and what they can play

| Client | Plays natively | Needs help with |
|---|---|---|
| Chrome on Windows PC | H.264, HEVC (with the RTX present), VP9, AV1; AAC, MP3, Opus, FLAC; MP4/WebM (MKV mostly works) | AC3/E-AC3, DTS, TrueHD audio; 10-bit H.264; AVI/WMV; image subtitles |
| Chrome on Android phones | H.264, VP9, usually HEVC (phone dependent); AAC, Opus | Same audio gaps; 4K/high bitrate on older phones; HEVC on some phones |
| Samsung TV browser (Tizen) | H.264/AAC in MP4 reliably; HLS natively; HEVC varies by model year | Almost anything else; its `canPlayType` answers are not always trustworthy |

The most common real-world failure is a fine H.264/HEVC MKV with AC3 or DTS audio. That only
needs the audio converted, which is cheap. Full video transcoding is for AVI/WMV/MPEG-2, 10-bit
H.264, HEVC on devices without HEVC, 4K on devices that can't take it, and burned-in subtitles.

The TV's earlier trouble *reaching* the server is a network issue that transcoding won't fix; it
should be sorted separately.

## 4. Playback modes

Decided per play, cheapest first:

1. **Direct play**: today's `/stream`. Container, video and audio all supported.
2. **Remux**: video and audio copied into fragmented MP4. For an unsupported container only
   (e.g. MKV on the TV). Near-zero CPU.
3. **Audio convert**: video copied, audio converted to AAC. For AC3/DTS/TrueHD. Low CPU.
4. **Full transcode**: video re-encoded to H.264 8-bit (NVENC, CPU fallback) plus AAC, scaled to
   the chosen quality. HDR sources are tone-mapped to SDR. Image subtitles can be burned in here.

## 5. Delivery: HLS with fMP4 segments

Modes 2–4 are delivered as **HLS** (`.m3u8` playlist + fragmented MP4 segments).

- Seeking works properly: the playlist lists every segment for the whole runtime up front, so
  the seek bar shows the full length and jumping ahead just requests a later segment.
- The Samsung TV and Android Chrome play HLS natively. Desktop Chrome needs **hls.js** (a ~300 KB
  Apache-2.0 script, bundled into the binary like `app.js`, never loaded from a CDN). This is the
  one new dependency in the plan; see question 1.
- Rejected alternative: a single progressive fMP4 stream piped from ffmpeg. No new dependency,
  but the browser can't seek in it, so every seek would restart ffmpeg with `?start=` and the
  player would need hand-built controls and time offsets. It is more code and a worse result.

**Segmenting.**
- Full transcode: fixed 6 s segments, keyframes forced at each boundary
  (`-force_key_frames expr:gte(t,n_forced*6)`), so the playlist can be computed from the runtime
  alone.
- Remux / audio convert: segments must start on the file's own keyframes. On first play
  Couchside reads the keyframe times with ffprobe (seconds, not minutes, since packets aren't
  decoded), caches them in the database keyed like probe results (path + size + mtime), and
  builds the playlist from them.

**Seeking past the encoder.** When a requested segment is more than a few segments ahead of what
ffmpeg has written, the session kills ffmpeg and restarts it at that segment with `-ss`,
`-start_number` and a matching timestamp offset. The request waits until the segment exists.

**Pacing.** ffmpeg runs with `-readrate 1.5 -readrate_initial_burst 30` so it builds a
cushion quickly, then stays a bit ahead of playback instead of encoding a whole film nobody may
watch. Remux and audio convert can run faster (they're cheap) but are still capped so a stopped
session doesn't churn the disk.

## 6. Deciding the mode (capability detection)

**Client side.** Before playing, `app.js` builds a capability profile:

- `MediaSource.isTypeSupported` / `video.canPlayType` for codec strings: H.264 high (8-bit),
  H.264 10-bit, HEVC main/main10, VP9, AV1; AAC, MP3, Opus, FLAC, AC3, E-AC3, DTS; containers
  MP4/WebM/MKV; and whether HLS is native or via MSE.
- Screen height (caps the output at e.g. 1080p on a 1080p TV or phone).
- A per-device override stored in `localStorage`: **TV mode** (force H.264/AAC MP4, max 1080p,
  stereo) and a quality choice (Auto / Original / 1080p / 720p / 480p).
- The Samsung TV is recognised by its `Tizen` user agent and gets TV mode by default, because its
  `canPlayType` answers can't be fully trusted.

**Server side.** `POST /api/v1/items/{id}/playback` takes the profile and returns the mode, the
URL to play and a human reason ("Converting DTS audio"). The decision is a pure function in
`couchside-transcode` with a unit-test table covering every combination we care about.

**Runtime fallback.** If direct play still fires the `<video>` `error` event, the player asks
again with direct play ruled out instead of showing the "use VLC" message. Same for remux →
full transcode.

**Richer probe data.** The decision needs more than we store today. New migration
`0003_media_streams.sql`:

- per item: overall bitrate, and a `probe_version` so existing items are re-probed once on the
  next scan;
- `media_streams` table: index, type, codec, profile, pixel format / bit depth, colour transfer
  (HDR10/HLG detection), channels, language, title, default/forced flags, and whether a subtitle
  is text or image.

That also unlocks choosing the audio language and showing embedded subtitles later.

## 7. Encoding settings

| | Video | Audio |
|---|---|---|
| Remux | copy | copy |
| Audio convert | copy | AAC stereo 192 kb/s (`-ac 2`, proper downmix) |
| Full transcode (NVENC) | `-hwaccel cuda` decode, `h264_nvenc -preset p5 -tune hq`, VBR at the quality cap | AAC stereo 192 kb/s |
| Full transcode (CPU fallback) | `libx264 -preset veryfast -crf 21` with a max rate | AAC stereo 192 kb/s |

Quality caps (LAN only, so generous): Original-res ≈ 20 Mb/s, 1080p 10 Mb/s, 720p 5 Mb/s,
480p 2 Mb/s.

HDR → SDR: `zscale` + `tonemap=hable` on the CPU path, NVENC path uses CUDA decode and the same
filters (slower than pure GPU but correct). Only matters if the library has HDR files
(question 3).

**Fallback (rule 7.5).** If NVENC fails to start (driver update, session limit) the session logs
it once and restarts the same job with libx264. A short NVENC self-test runs at startup and its
result shows in Settings.

## 8. Sessions, processes and temp storage

New crate **`couchside-transcode`** (depends on `couchside-core` and `couchside-db` only):

- `TranscodeManager` in `AppState`: a map of sessions, each owning its ffmpeg `Child`
  (`kill_on_drop`), its folder, the last time a client requested anything, and its mode.
- Ended by: the player calling `DELETE /api/v1/transcode/{session}` on leaving the page (plus
  `navigator.sendBeacon` on `pagehide`), no requests for 2 minutes while ffmpeg is still running,
  or no requests for 30 minutes at all (paused overnight). Server shutdown kills everything.
- Crash safety: each ffmpeg is put in a Windows **Job Object** with kill-on-close, so if
  Couchside itself crashes Windows kills the ffmpeg processes too. That needs the
  `Win32_System_JobObjects` feature on the existing `windows` crate (question 6).
- Temp files: `%ProgramData%\Couchside\transcodes\<session-id>\` (configurable). A session's
  folder is deleted when it ends; the whole folder is emptied at startup; a size cap (default
  20 GB) ends the oldest idle sessions first if it's ever reached.
- No permanent "optimised versions" cache in this plan. Every conversion is temporary.

**Limits.** Defaults: at most **2 full transcodes** and **4 remux/audio-convert** sessions at
once. One viewer gets one session; starting a new play from the same device ends that device's
previous session. Over the limit, the playback call returns a clear message ("2 videos are
already being converted") rather than starting a stream that stutters.

**New config section** (documented in `config.example.toml`):

```toml
[transcode]
# "nvenc" = NVIDIA GPU, "none" = CPU only. Falls back to CPU if the GPU fails.
hardware = "none"
max_transcodes = 2
max_remuxes = 4
# temp_dir = 'D:\CouchsideTranscodes'
temp_max_gb = 20
```

The default stays `"none"` per rule 7.5 (opt-in); LJ's own config would set `"nvenc"`.

**New API** (added to `docs/api.md`):

| Route | Purpose |
|---|---|
| `POST /items/{id}/playback` | Decide the mode; returns `{ mode, url, session_id?, reason }` |
| `GET /transcode/{session}/main.m3u8` | Playlist |
| `GET /transcode/{session}/init.mp4`, `/seg/{n}.m4s` | Segments (waits for ffmpeg if needed) |
| `DELETE /transcode/{session}` | Stop |
| `GET /transcode/sessions` (admin) | Active streams for the Settings page |

Session ids are random and tied to the user who started them; segment names are generated
server-side, so no client-supplied paths reach the file system.

## 9. UI

- **Player:** a small badge in the top bar: "Direct play", "Converting audio", "Converting video
  (GPU)". A gear menu with Quality (Auto / Original / 1080p / 720p / 480p) and, once streams are
  probed, Audio track. Changing quality restarts the session at the current position.
- **Fewer dead ends:** the "use VLC" message only appears if transcoding itself fails (or ffmpeg
  is missing), with the real reason.
- **Settings → Playback:** "Use NVIDIA GPU" switch (with the self-test result), limits, temp
  folder usage, and an **Active streams** list (who, what, mode, Stop button).
- **Settings → This device:** "TV mode" switch and default quality, stored per device.
- **Item page:** the existing Video line gains audio info (e.g. "DTS 5.1, will convert to AAC").

## 10. Phases

Each phase is its own PR, buildable and useful on its own.

| Phase | Delivers | Main work |
|---|---|---|
| **0. Groundwork** | No visible change | Migration 0003 + richer probe and one-time re-probe; `couchside-transcode` crate with the decision function and tests; `[transcode]` config; temp folder cleanup at startup; ADR 0002 "Transcoding". |
| **1. Remux + audio convert** | MKV/AC3/DTS files play on phones, PC and TV | Capability profile in `app.js`; `/playback`; keyframe index; HLS session manager with process tracking and Job Objects; bundled hls.js; runtime fallback; player badge. |
| **2. Full transcode** | AVI, WMV, 10-bit, HEVC-on-TV, 4K-to-1080p all play | NVENC + libx264 fallback; fixed-segment playlists; seek-restart; quality menu; NVENC self-test; HDR tone mapping (if needed). |
| **3. Polish** | Day-to-day comfort | TV mode + per-device defaults; Active streams in Settings; audio track choice; embedded text subtitles as WebVTT; image subtitle burn-in. |

**Testing.** Tiny fixture clips generated with ffmpeg at test time (rule 13.3): H.264/AC3 MKV,
HEVC 10-bit, MPEG-2 in AVI, a PGS subtitle sample. Unit tests for the decision table, playlist
generation and keyframe parsing; an integration test that starts a remux session and fetches the
playlist and first segment; a check that ending a session leaves no ffmpeg process and no temp
folder. Manual check on the Samsung TV and a phone before each phase is called done.

## 11. Decisions (LJ, 2026-10-14)

1. **Delivery:** HLS, with hls.js bundled into Couchside (approved new dependency).
2. **GPU:** use the RTX 3070 (NVENC) for video conversion, CPU as fallback. LJ's config sets
   `hardware = "nvenc"`; the shipped default stays `"none"`.
3. **HDR:** possible but uncommon in the library. Tone mapping stays in Phase 2 but last in line;
   the Phase 0 re-probe will show how many HDR files there are.
4. **Audio:** converted audio is stereo AAC (the recommended option).
5. **Limit:** 2 full transcodes at once (`max_transcodes = 2`).
6. **Crash safety:** enable `Win32_System_JobObjects` on the existing `windows` crate (approved).
7. **TV:** Samsung from roughly 2019–2022 (Tizen 5–6.5). TV mode stays H.264/AAC by default;
   whether HEVC can go direct to it gets tested on the TV in Phase 2 before it's allowed.

# ADR 0002: Transcoding

- Status: accepted
- Date: 2026-10-14

## Context

Couchside only plays files as they are, so MKVs with AC3/DTS audio, AVI/WMV, 10-bit H.264 and
HEVC on the Samsung TV fail in the browser. The full plan, with the reasoning and the owner's
answers, is in [docs/transcoding-plan.md](../transcoding-plan.md). The server PC has an NVIDIA
RTX 3070 and a 4-core i7-7700K.

## Decisions

1. **Cheapest mode first** (rule 7.4): direct play, then remux, then audio convert (video copied,
   audio to stereo AAC), then full transcode to H.264 8-bit + AAC. The choice is a pure function
   (`couchside_transcode::decide`) of the probed streams and a capability profile the player
   sends. Unknown formats a client claims are ignored, and TV mode narrows the profile to
   H.264/AAC in MP4 at up to 1080p.
2. **HLS with fragmented MP4 segments** for everything except direct play, so seeking works
   without custom player controls. hls.js is bundled into the executable for desktop Chrome
   (approved new dependency, Apache-2.0); the TV and Android Chrome play HLS natively.
3. **NVENC for video, libx264 as fallback.** `[transcode] hardware = "nvenc"` is opt-in (rule
   7.5); the shipped default is `"none"`.
4. **Per-stream probe data in `media_streams`** (migration 0003), so decisions can see bit depth,
   HDR transfer, channel counts, languages and subtitle types. The migration resets `probed`, so
   every file is read once more by ffprobe after upgrading.
5. **Temporary files only.** Segments live in `transcodes\cs-<session>\` in the data folder (or
   `[transcode] temp_dir`). Only `cs-` folders are ever deleted, at startup and when a session
   ends, so a temp folder pointed somewhere shared can't lose other files.
6. **Limits:** 2 full transcodes and 4 remux/audio-convert sessions at once by default.
7. **Crash safety:** ffmpeg processes go in a Windows Job Object with kill-on-close (approved
   `Win32_System_JobObjects` feature on the existing `windows` crate) on top of `kill_on_drop`.

## Consequences

- Phase 0 (this ADR's first change) adds no visible behaviour: richer probe data, the decision
  function, config, and the temp folder cleanup. Playback still uses `/items/{id}/stream`.
- The one-time re-probe after upgrading takes a while on big libraries; it runs in the
  background like any other probe pass.

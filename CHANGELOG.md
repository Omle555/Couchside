# Changelog

All notable changes to Couchside are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Linux support for running in a Proxmox LXC container: `deploy/proxmox/couchside-lxc.sh` creates
  or updates a Debian 12 container, and `deploy/linux/install.sh` builds Couchside and installs it
  as the `couchside` systemd service (config in `/etc/couchside`, data in `/var/lib/couchside`).
- Network discovery on Linux through an Avahi service file, removed again when Couchside stops.
- `deploy/proxmox/container-setup.sh`, run by the Proxmox script: sets a random root password
  when root has none, creates the first `admin` account with a random password when Couchside has
  no users, saves both in `/root/couchside-credentials.txt`, and shows Couchside's address on the
  container console (`/etc/issue`) and after signing in.
- Network discovery: Couchside announces itself as `_couchside._tcp` over mDNS through the
  Windows DNS-SD API when bound to a non-loopback address (`[server] discovery`, on by default),
  with TXT entries `version`, `api` and `server`.
- `GET /api/v1/activity` (admin): library totals, scan state, open converted streams and what is
  playing now (positions saved in the last 30 seconds).
- Home Assistant custom integration, kept in its own repository ([couchside-homeassistant](https://github.com/Omle555/couchside-homeassistant)), with zeroconf
  discovery, an admin sign-in config flow, re-authentication, and sensors for now playing,
  converting streams, library totals and scans.

### Changed
- On Linux, Delete moves items into a `.couchside-trash` folder beside them instead of deleting
  them permanently, and the Delete log records where each one went.
- The PC name (for discovery and `/activity`) falls back to `/etc/hostname` when no `COMPUTERNAME`
  or `HOSTNAME` variable is set, as under systemd.

### Planned
- Embedded subtitle tracks
- System tray icon and Windows installer (MSI)

## [0.1.0] - 2026-10-15

First public release. It bundles the first build from 2026-10-11 (listed last) and everything
added since.

### Changed
- Home spotlight halved in height (33vh, 320-380px) with a smaller title and a two-line
  description (hidden on phones); removed the top-bar greeting.
- Redesigned the web UI: dark theme with a red accent and serif headings, a left sidebar (bottom
  tab bar on phones) with a live server-connection indicator, and a top bar with greeting, search
  and avatar. The home page has a spotlight carousel (movies with backdrops, same picks all day),
  a "Pick up where you left off" row and genre-based mood chips over a poster row. "TV Shows" is
  now "Series" in the UI. New red couch logo. The CSS no longer uses flexbox `gap`,
  `aspect-ratio` or `inset`, so layouts work on older TV browsers (Tizen 2019-2022).
- Renamed the project from Solidfin to Couchside: crates (`couchside-*`), `couchside.exe`, the
  Windows service and firewall rule, the web UI, the session cookie and the default data folder
  (`%ProgramData%\Couchside`). On start, an existing `%ProgramData%\Solidfin` folder is moved to
  the new location and `solidfin.db` is renamed to `couchside.db`. Users must sign in again.

### Added
- Full video conversion (transcoding phase 2): `transcode` playback mode re-encodes video to
  H.264 8-bit with NVENC (`h264_nvenc`, CUDA decode) or libx264, scaled to the viewer's limit, with
  stereo AAC. Fixed 6 s HLS segments (keyframes forced on segment boundaries only), exact seeking
  on restart, NVENC self-test at startup and per-stream fallback to x264 (rule 7.5). HDR (PQ/HLG)
  is tone-mapped with libplacebo through Vulkan on the GPU path, zscale + tonemap on the CPU path.
  `[transcode] max_transcodes` limits conversions separately from `max_remuxes`.
- `quality` in `POST /items/{id}/playback` (`auto`, `original`, `1080p`, `720p`, `480p`), a
  Quality menu in the player, `gpu` in the playback response and `video_encoder` in `GET /setup`.
- Repackaged playback (transcoding phase 1): `POST /items/{id}/playback` takes the device's
  supported formats and picks direct play, remux (copy into fragmented MP4) or audio convert
  (video copied, audio to stereo AAC). Repackaged streams are HLS with a full VOD playlist built
  from the file's keyframe times (read once with ffprobe, cached in the new `keyframes` table);
  ffmpeg cuts a segment at every keyframe, runs at up to 4x real time and is restarted at the
  requested segment after a seek. Copied HEVC is tagged `hvc1`. Streams end when the viewer
  leaves, after 30 idle minutes or at shutdown (ffmpeg stops after 2 idle minutes); their folders
  are deleted, and idle streams are ended if `temp_max_gb` is exceeded. ffmpeg runs in a Windows
  job object, so it is killed even if Couchside crashes. Limit: `[transcode] max_remuxes`
- Web player: sends a format profile (TV mode on smart-TV browsers), plays HLS with bundled
  hls.js 1.7.3 (Apache-2.0, `/vendor/hls.min.js`, loaded only when needed) or natively on TVs,
  shows a Direct play / Repackaging / Converting audio label, and falls back to a repackaged
  stream when the original file fails to play
- Groundwork for transcoding ([plan](docs/transcoding-plan.md), [ADR 0002](docs/adr/0002-transcoding.md)):
  ffprobe now records every video, audio and subtitle stream (codec, profile, bit depth, HDR,
  channels, language, default/forced) and the overall bitrate in a new `media_streams` table;
  existing files are re-probed once after upgrading. New `couchside-transcode` crate with the
  playback decision (direct play, remux, audio convert or transcode) and a `[transcode]` config
  section (`hardware`, `max_transcodes`, `max_remuxes`, `temp_dir`, `temp_max_gb`). Leftover
  transcode folders are removed at startup. Playback itself is unchanged for now
- Folder watching: library folders are watched for added, removed and renamed files, and only
  the changed movie or show folder is rescanned (then media info and metadata are fetched for
  anything new). A folder is rescanned once it has been quiet for `watch_delay_seconds`
  (default 10) and no video in it is still being copied. On by default; new
  `[scan] watch_folders` and `[scan] watch_delay_seconds` settings. Offline drives and shares
  are retried every minute, and new or removed libraries are picked up straight away
- Start-up splash screen in the web UI: the Couchside logo, a thin progress bar that follows
  the real start-up steps (connecting, signing in, loading the first page) and a short status
  line, then a fade into the app. If the server is still starting it waits and retries instead
  of showing an error. No extra files or libraries; honours "reduce motion"
- The splash now opens as soon as `couchside.exe` starts: the port opens first and the splash
  shows each start-up step (opening the database, looking for FFmpeg, loading settings,
  starting the library) before the app takes over. If start-up fails, the page says so
- Thin progress bar along the top of every page while a library scan runs; `GET /scan` now
  reports `step` and `steps`
- Movies are matched to TMDB by scoring every search result on title similarity, year (±1) and
  popularity; weak matches are rejected instead of guessed
- Full movie details: tagline, genres, rating, release date, age rating (for your region),
  director, cast, original title and IMDb id
- Exact matching from `{tmdb-123}`, `[tmdbid=123]` or `{imdb-tt123}` tags in file/folder names
- "Identify" dialog to fix a wrong match; manual choices are kept on future scans
- "Scan video for title" in the Identify dialog: grabs frames from the first 10 minutes with
  ffmpeg, reads their on-screen text with Windows' built-in OCR and shows the frames with that
  text. Clicking a piece of text fills in the search; nothing is matched automatically.
  New `[ffmpeg] ffmpeg_path` setting (found next to ffprobe by default)
- "Clean up" button on a movie's page: deletes the movie's stored details and artwork and
  downloads them again for the movie it is identified as. With the new
  "Allow file renames and deletes" switch under Settings → Libraries (or `[library] allow_file_renames` in
  config.toml; off by default) it can also rename the file to
  `Title (Year)/Title (Year) 1080p.ext`, together with the subtitles, .nfo and artwork named
  after it, and move the images and subtitles in the movie's folder (only its own ones in a
  shared folder, never videos) to `removed\` in the data folder. Every change is shown first, nothing is overwritten, watch progress is kept
  and every rename is listed in `logs\renames.log`
- "Delete" button on a movie's page (when file changes are allowed): sends the movie's folder,
  or just its own files in a shared folder, to the Recycle Bin after showing exactly what goes,
  and removes it from Couchside
- TMDB key and language can be set (and are validated) under Settings → Metadata
- Database migration 0002 (existing movies are re-matched once with the new matcher)
- `[server] lan_only` setting (on by default): only devices on the local network and this PC can
  connect, everyone else gets `403 Forbidden`
- The addresses to open from other devices on your network are shown at startup when
  `bind = "0.0.0.0"`
- `install-service` adds a Windows Firewall rule for the configured port (private networks, local
  subnet only); `uninstall-service` removes it

### Fixed
- Seeking far ahead in a repackaged or converted stream: segments from a restarted ffmpeg now
  carry absolute fragment times (`movflags=+frag_discont`), so they fit the init segment the
  player already has
- MKV on iPhone/Safari: Safari no longer claims MKV support, and when direct play fails the
  fallback converts audio that can't be copied into MP4 (Vorbis) instead of failing
- After Identify, the movie page and lists show the new match's poster and backdrop straight
  away instead of the browser's cached artwork from the old match

### First build (2026-10-11)
- Movie and TV libraries with incremental scanning (local drives, mapped drives, UNC shares)
- File-name parser for common movie and episode naming styles
- Optional TMDB metadata with a local image cache
- Optional ffprobe media info (duration, resolution, codecs)
- Web UI: home, movies, TV shows, seasons, search, item details, player, settings
- Direct play with HTTP Range seeking, resume position, continue watching, watched marks
- External subtitles (SRT, VTT, ASS/SSA) converted to WebVTT
- Multiple users with Argon2id passwords and cookie sessions; first-run admin setup
- Windows service install/uninstall
- SQLite database with automatic migrations and pre-migration backups
- Daily rolling log files

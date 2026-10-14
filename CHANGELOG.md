# Changelog

All notable changes to Couchside are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- Renamed the project from Solidfin to Couchside: crates (`couchside-*`), `couchside.exe`, the
  Windows service and firewall rule, the web UI, the session cookie and the default data folder
  (`%ProgramData%\Couchside`). On start, an existing `%ProgramData%\Solidfin` folder is moved to
  the new location and `solidfin.db` is renamed to `couchside.db`. Users must sign in again.

### Added
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
- After Identify, the movie page and lists show the new match's poster and backdrop straight
  away instead of the browser's cached artwork from the old match

### Planned
- On-the-fly transcoding / remuxing with FFmpeg for formats browsers can't play
- Embedded subtitle tracks
- System tray icon and Windows installer (MSI)
- File-system watching for instant updates

## [0.1.0] - 2026-10-11

### Added
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

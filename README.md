# Couchside

A lightweight, self-hosted movie and TV server for Windows, written in Rust.
Point it at your movie and TV folders and watch them in any web browser on your PC or home network,
much like Jellyfin or Plex but much smaller: Couchside is a single `couchside.exe`.

## What's new

**2026-10-14**
- The project has a new name: **Solidfin is now Couchside**. The program is now `couchside.exe`,
  the Windows service is "Couchside" and data lives in `%ProgramData%\Couchside`. On first start
  the old `%ProgramData%\Solidfin` folder (database, artwork, settings, logs) is moved over
  automatically. You'll need to sign in again once.
- Libraries now update by themselves: Couchside watches your library folders and, as soon as a
  movie or episode is added, removed or renamed, rescans just that folder (after it has been
  quiet for 10 seconds, and never while a video is still being copied). Turn it off with
  `[scan] watch_folders = false`.
- Start-up splash screen: the Couchside logo with a progress bar that follows the real start-up
  steps, then a fade into the app. It appears as soon as `couchside.exe` starts (while the database,
  FFmpeg check and settings load) and says so if start-up fails.
- Thin progress bar along the top of every page while a library scan runs; `GET /scan` now
  reports `step` and `steps`.

## Features (v0.1)

- **Libraries** of movies and TV shows on local drives, USB drives or network shares (`\\NAS\Video`)
- **Automatic scanning** on startup, every hour and on demand; only new or changed files are processed
- **Folder watching**: new, removed and renamed files show up within seconds; only the changed
  movie or show folder is rescanned
- **Smart file-name parsing**: `The Matrix (1999).mkv`, `The.Matrix.1999.1080p.BluRay.mkv`, `Show/Season 01/Show - S01E02 - Title.mkv`, `S01E01E02`, `1x02`
- **Automatic TMDB matching** for movies: each file name is matched to The Movie Database using the
  title, year and popularity, then the poster, backdrop, description, tagline, genres, rating,
  age rating, director and cast are downloaded and cached locally (optional, free API key)
- **Exact matches from file names** with Jellyfin/Plex-style tags: `Heat (1995) {tmdb-949}.mkv`,
  `[tmdbid=949]` or `{imdb-tt0113277}`
- **Identify**: fix a wrong match by searching TMDB and picking the right movie; your choice is kept.
  For files with meaningless names, **Scan video for title** shows frames from the first 10 minutes
  with the text Windows reads on them, so you can search for the title shown on screen
- **Media info** (duration, resolution, codecs) via `ffprobe` (optional)
- **Web player** with seeking, resume where you left off, "Continue watching" and watched marks
- **Subtitles**: external `.srt`, `.vtt`, `.ass`, `.ssa` files next to the video (converted on the fly)
- **Multiple users**, each with their own watch progress; administrator accounts manage libraries
- **Runs as a console app or a Windows service** that starts with Windows
- **Clean up**: rebuilds a movie's details and artwork from the movie it is identified as. If you
  switch on **Allow file renames and deletes** in Settings, it can also rename the file to
  `Title (Year)\Title (Year) 1080p.mkv` and clear out the images and subtitles next to it, after
  showing you every change
- **Hands off your files**: Couchside never moves, changes or deletes your media files, and only
  renames them, or moves images and subtitles aside, through Clean up, or sends a movie to the Recycle Bin with **Delete**, when you allow it

Not yet included: transcoding (files play as-is, so the browser must support the format), a tray icon,
and apps for TVs/phones. See [CHANGELOG.md](CHANGELOG.md).

## Building from source

1. Install **Visual Studio Build Tools** with the *Desktop development with C++* workload:
   https://visualstudio.microsoft.com/visual-cpp-build-tools/
2. Install **Rust** from https://rustup.rs (accept the defaults).
3. Open a new terminal in this folder and run:

   ```powershell
   cargo build --release
   ```

   The first build downloads dependencies and takes a few minutes. The program is created at
   `target\release\couchside.exe`.

To run the tests: `cargo test --workspace`.

## First run

```powershell
.\target\release\couchside.exe
```

Your browser opens at <http://localhost:8096>. Create the administrator account, then go to
**Settings → Libraries** and add your folders, for example:

| Name     | Type      | Folder                     |
|----------|-----------|----------------------------|
| Movies   | Movies    | `D:\Movies`                |
| TV Shows | TV Shows  | `\\NAS\Video\TV`           |

Scanning starts straight away. Press `Ctrl+C` in the console window to stop the server.

### Optional: posters, descriptions and cast (TMDB)

1. Create a free account at https://www.themoviedb.org and request an API key under
   *Settings → API* (either the "API Key" or the "API Read Access Token" works).
2. In Couchside, open **Settings → Metadata (TMDB)**, paste the key, pick your language and click
   **Save**. Couchside checks the key and starts fetching details straight away.

You can also set `tmdb_api_key` and `language` under `[metadata]` in `config.toml`; a key saved in
Settings takes priority.

**How movies are matched:** Couchside reads the title and year from the file name (or its folder),
searches TMDB, and scores every result by how closely the title matches, whether the year agrees
(±1 year is allowed) and how well known the film is. Only a confident match is used; otherwise the
movie keeps its file name and you can pick it with **Identify…** on the movie's page. To skip the
search entirely, put the id in the name: `Heat (1995) {tmdb-949}.mkv`.

### Optional: durations and codec info (FFmpeg)

Download an FFmpeg build for Windows (e.g. from https://www.gyan.dev/ffmpeg/builds/) and either put
`ffprobe.exe` next to `couchside.exe`, add FFmpeg's `bin` folder to your `PATH`, or set
`ffprobe_path` in `config.toml`. "Scan video for title" also needs `ffmpeg.exe`, which Couchside
looks for next to `ffprobe.exe` (or set `ffmpeg_path`).

## Folder naming

Couchside follows the same conventions as Jellyfin, Plex and Kodi.

```
D:\Movies\
  The Matrix (1999)\The Matrix (1999).mkv
  The Matrix (1999)\The Matrix (1999).en.srt
  Heat (1995).mp4

\\NAS\Video\TV\
  Firefly (2002)\
    Season 01\
      Firefly - S01E01 - Serenity.mkv
      Firefly - S01E02 - The Train Job.mkv
    Specials\
      Firefly - S00E01 - Here's How It Was.mkv
```

Files named `*-sample.*` or `*-trailer.*` and folders such as `Extras`, `Featurettes`, `$RECYCLE.BIN`
and `System Volume Information` are ignored.

## Watching on other devices

By default Couchside only accepts connections from the same PC. To use it from phones, laptops or TVs
on your home network:

1. Open `C:\ProgramData\Couchside\config.toml` and under `[server]` set `bind = "0.0.0.0"`.
2. Restart Couchside. The console (and the log) shows the address to use on other devices, for
   example `http://192.168.1.20:8096`. Type it exactly, including `http://` and `:8096`. Smart TVs and
   phones need this number address; `http://<your-pc-name>:8096` usually only works from other Windows PCs.
3. Allow Couchside through Windows Firewall. When Windows asks the first time, tick **Private networks**
   only. `install-service` adds the rule for you. To add it by hand, run this in an Administrator terminal
   (change 8096 if you changed `port`):

   ```powershell
   netsh advfirewall firewall add rule name="Couchside Media Server" dir=in action=allow protocol=TCP localport=8096 profile=private remoteip=localsubnet
   ```

Your PC's network must be set to **Private** in Windows settings (Network & Internet → your
connection → Network profile type) for the rule to apply.

Only devices on the same home network can connect: `lan_only = true` (the default) answers
`403 Forbidden` to any address that is not private (10.x, 172.16–31.x, 192.168.x, 169.254.x,
IPv6 `fc00::/7` and `fe80::/10`) or this PC. Accounts and passwords still apply on top of that.
Don't forward Couchside's port on your router.

## Running as a Windows service

From an **Administrator** terminal:

```powershell
.\couchside.exe install-service     # installs, starts, and starts with Windows
.\couchside.exe uninstall-service   # stops and removes it
```

The service runs as the *Local System* account. It can read local drives, but network shares need
permission for the computer account. Alternatively, open *Services*, select *Couchside Media Server*
→ *Properties* → *Log On* and choose your own Windows account.

## Where Couchside keeps its data

Everything lives in `C:\ProgramData\Couchside\` (change it with `data_dir` in `config.toml`):

| Path                 | Contents                                            |
|----------------------|-----------------------------------------------------|
| `config.toml`        | Settings                                            |
| `couchside.db`        | Libraries, users, watch progress (SQLite)           |
| `images\`            | Cached posters and backdrops                        |
| `logs\`              | Daily log files (14 days kept)                      |

## Browser support

Couchside streams the original file. Chrome and Edge play MP4 and most MKV files with H.264 video and
AAC/MP3/Opus audio. HEVC (H.265) plays in Edge on Windows with the HEVC Video Extensions installed.
Files with DTS/AC3 audio or AVI/WMV containers usually need transcoding, which is planned for a
later version.

## Project layout

```
crates/
  couchside-core       shared types, config, file-name parsing, path safety
  couchside-db         SQLite schema, migrations and queries
  couchside-library    folder scanning and watching, ffprobe and ffmpeg frame grabs
  couchside-metadata   TMDB client and image cache
  couchside-server     couchside.exe: HTTP API, web UI, Windows service
docs/                 API reference and architecture decisions
Rules.md              project rules for contributors
```

See [docs/api.md](docs/api.md) for the HTTP API.

## License

MIT OR Apache-2.0

# Couchside

A lightweight, self-hosted movie and TV server for Windows, written in Rust.
Point it at your movie and TV folders and watch them in any web browser on your PC or home network,
much like Jellyfin or Plex but much smaller: Couchside is a single `couchside.exe`.

## What's new

**2026-10-16: Couchside runs on Proxmox (LXC container)**
- Couchside now also runs on Linux. One script on your Proxmox host,
  `deploy/proxmox/couchside-lxc.sh`, creates a Debian 12 container, builds Couchside in it and
  starts it as a service on boot. Running it again updates Couchside. See
  [Running on Proxmox](#running-on-proxmox-lxc-container).
- On Linux, Couchside announces itself through Avahi, so Home Assistant still discovers it, and
  **Delete** moves a movie into a hidden `.couchside-trash` folder next to it instead of the
  Windows Recycle Bin.

**2026-10-15: Home Assistant can find Couchside**
- Couchside now announces itself on your home network (mDNS, as `_couchside._tcp`) using the
  announcer built into Windows, whenever other devices can connect (`bind = "0.0.0.0"`). Turn it off
  with `discovery = false` under `[server]`.
- New Home Assistant integration in its own repository, [couchside-homeassistant](https://github.com/Omle555/couchside-homeassistant): once installed in
  Home Assistant it shows **Discovered: Couchside**, signs in with an admin account and adds sensors
  for what is playing now, converting streams, movie, show and episode totals, and library scans.
  See [Home Assistant](#home-assistant).
- New `GET /api/v1/activity` endpoint (admin) with those totals and what is playing now.

**2026-10-15: version 0.1.0 released**
- Couchside 0.1.0 is the first release on GitHub. Download `Couchside-v0.1.0-windows-x64.zip`
  from the Releases page, unzip it and run `couchside.exe`. It includes everything listed below.

**2026-10-15 (later)**
- The home page spotlight is half as tall, so your rows show up sooner, and the "Your evening,
  curated." greeting has been removed from the top bar.

**2026-10-15**
- **New look for the web app.** A near-black and red theme with a sidebar (Home, Movies, Series,
  Settings), a "Server connected" light, and a greeting bar with search. The logo is now a red couch.
- **New home page:** a full-width "Tonight's spotlight" of up to four of your movies (play or view
  details, with a 01 / 04 pager), a "Pick up where you left off" row with progress bars and resume
  buttons, and "Made for your mood" chips (Feel-good, Edge of your seat, Mind-bending...) that
  filter your movies by genre. The recently added rows are still below.
- On phones the sidebar becomes a tab bar along the bottom, and Sign out is also in Settings.
- Layout fixes for 2019-2022 Samsung TVs, whose browsers lacked some newer CSS (poster boxes and
  spacing now work there too).

**2026-10-14 (later)**
- **Every video now plays: Couchside converts the picture itself when it has to.** AVI, WMV,
  MPEG-2, 10-bit H.264, HEVC on devices without HEVC and 4K on smaller screens are converted to
  H.264 while they play, on the NVIDIA graphics card when `[transcode] hardware = "nvenc"` (on the
  processor if the card fails or is switched off). 4K HDR films are tone-mapped on the card so
  they don't look washed out. The player label says "Converting video (GPU)".
- **Quality menu in the player:** Auto, Original, 1080p, 720p, 480p, remembered per device.
  Changing it carries on from the same spot.
- **Seeking in repackaged and converted streams fixed.** Jumping far ahead could stall or jump,
  because the restarted stream's timing didn't match the start of the stream.
- **iPhone fix:** MKV files with Vorbis sound (and other MKVs) now play on iPhones: Safari is no
  longer told it can open MKV, and the fallback converts sound that can't be repackaged.
- At most 2 videos are converted at once (`max_transcodes`); Settings shows whether conversion uses
  the graphics card or the processor.

**2026-10-14**
- **Movies with AC3 or DTS sound now play in the browser and on the TV.** When a device can't
  play a file's sound or file type, Couchside repackages it on the fly: the picture is copied
  untouched (no quality loss, almost no CPU) and only the sound is converted to stereo. A small
  label on the player says "Direct play", "Repackaging" or "Converting audio". Seeking works
  normally. If a file still won't play directly, the player switches to the repackaged version
  by itself. FFmpeg stops when you leave the player, when Couchside stops, and even if Couchside
  crashes. The first play of a repackaged movie takes a moment longer while Couchside reads
  where its keyframes are. Up to 4 of these streams can run at once (`[transcode] max_remuxes`).
  Full video conversion (AVI, WMV, 10-bit, 4K) is next.
- Transcoding, step 1 of 4 (behind the scenes, nothing to see yet): Couchside now notes every
  video, audio and subtitle track inside each file, which it needs to decide later whether a
  file can play as-is or must be converted. After updating, it re-reads every file once in the
  background. There is a new, optional `[transcode]` section for `config.toml` (see
  `config.example.toml`); it does nothing until conversion itself arrives.
- Transcoding is planned: [docs/transcoding-plan.md](docs/transcoding-plan.md) covers how Couchside
  will convert files your browser or TV can't play (audio-only conversion first, then full video
  conversion on the NVIDIA GPU), in four phases. Nothing has changed in the app yet.
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

Not yet included: full video conversion (AVI/WMV, 10-bit H.264, HEVC on devices without HEVC, 4K
down to 1080p), a tray icon, and apps for TVs/phones. See [CHANGELOG.md](CHANGELOG.md).

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

## Home Assistant

With `bind = "0.0.0.0"`, Couchside announces itself on your network so Home Assistant can find it
(`discovery = true` under `[server]`, the default; it needs Windows 10 version 1809 or later and the
network profile set to **Private**). To add it to Home Assistant:

1. Install the integration from [couchside-homeassistant](https://github.com/Omle555/couchside-homeassistant), with HACS (add it as a
   custom repository of type Integration) or by copying its `custom_components/couchside` folder
   into your Home Assistant configuration.
2. Restart Home Assistant. **Settings → Devices & services** shows **Discovered: Couchside**. Click
   **Add** and sign in with a Couchside administrator account. If it isn't discovered, click
   **Add integration**, search for Couchside and type the PC's address (for example
   `192.168.1.20`) and port.

You get a **Playing** on/off sensor (handy for dimming the lights), **Now playing** (how many things
are playing, with the titles and viewers as attributes), **Converting streams**, **Movies**,
**TV shows**, **Episodes** and **Library scan**. Home Assistant polls every 15 seconds and keeps its
own Couchside session; if it is signed out it asks you to sign in again. Home Assistant must be on
the same home network as Couchside.

## Running as a Windows service

From an **Administrator** terminal:

```powershell
.\couchside.exe install-service     # installs, starts, and starts with Windows
.\couchside.exe uninstall-service   # stops and removes it
```

The service runs as the *Local System* account. It can read local drives, but network shares need
permission for the computer account. Alternatively, open *Services*, select *Couchside Media Server*
→ *Properties* → *Log On* and choose your own Windows account.

## Running on Proxmox (LXC container)

Couchside can also run in a Linux container on a Proxmox VE host. On the host, as root:

```bash
git clone https://github.com/Omle555/Couchside
MEDIA=/mnt/pve/nas/media bash Couchside/deploy/proxmox/couchside-lxc.sh
```

Set `MEDIA` to the host folder with your movies and shows. It appears read-only inside the
container as `/media`, so add libraries such as `/media/Movies` in Couchside's Settings. The script
creates an unprivileged Debian 12 container (4 cores, 4 GB RAM, 16 GB disk, DHCP on `vmbr0`;
change these with `CORES`, `MEMORY`, `DISK_GB`, `BRIDGE`, `STORAGE` and `CTID`), installs FFmpeg
and Avahi, builds Couchside (the first build takes a while) and starts it. When it finishes, it
prints the address to open, for example `http://192.168.1.30:8096`.

- **Update:** `git -C Couchside pull`, then run the script again with `CTID=<your container id>`.
  Settings, the database and the media mount are kept.
- **Settings:** `/etc/couchside/config.toml` in the container (it listens on the network with
  `lan_only = true`). Data and logs live in `/var/lib/couchside`. Restart with
  `systemctl restart couchside` and follow logs with `journalctl -u couchside -f`.
- **Network shares:** mount the share on the Proxmox host (for example as a Proxmox storage under
  `/mnt/pve/`) and point `MEDIA` at it.
- **Delete** (when file changes are allowed) has no Recycle Bin on Linux: it moves the movie into a
  hidden `.couchside-trash` folder in the same folder, where you can move it back or delete it for
  good. With the read-only `/media` mount, Clean up and Delete can't change files at all; leave out
  `ro=1` (`pct set <id> --mp0 /path,mp=/media`) if you want them to.
- Hardware video conversion (NVENC) isn't set up in the container, so conversions use the processor.

Couchside on any other Debian or Ubuntu machine installs the same way: run
`deploy/linux/install.sh` as root from a copy of the repository.

## Where Couchside keeps its data

Everything lives in `C:\ProgramData\Couchside\` (change it with `data_dir` in `config.toml`):

| Path                 | Contents                                            |
|----------------------|-----------------------------------------------------|
| `config.toml`        | Settings                                            |
| `couchside.db`        | Libraries, users, watch progress (SQLite)           |
| `images\`            | Cached posters and backdrops                        |
| `logs\`              | Daily log files (14 days kept)                      |
| `transcodes\`        | Temporary pieces of repackaged streams (deleted automatically) |

## Browser support

Couchside plays the original file whenever the device can. Before playing, the player tells
Couchside which formats the device supports. If only the sound or the file type is the problem (an
MKV with AC3, E-AC3, DTS or TrueHD sound, or an MKV on a TV), Couchside repackages the file on the
fly with FFmpeg: the picture is copied untouched and the sound is converted to stereo AAC. The
label in the corner of the player says which is happening. Seeking works as usual. On Samsung and
other smart-TV browsers Couchside sticks to H.264/AAC MP4, which every TV plays.

Files whose picture itself can't be played (AVI/WMV/MPEG-2, 10-bit H.264, HEVC on devices
without HEVC, 4K on a device that wants less) are converted to H.264 while they play, on an NVIDIA
graphics card when `[transcode] hardware = "nvenc"` (falling back to the processor if the card
fails) or on the processor otherwise. HDR films are tone-mapped (on the graphics card) so they
don't look washed out.
The Quality menu in the player (Auto, Original, 1080p, 720p, 480p) is remembered per device;
picking a size below the film's converts it down. At most `max_transcodes` (default 2) videos are
converted at once.

## Project layout

```
crates/
  couchside-core       shared types, config, file-name parsing, path safety
  couchside-db         SQLite schema, migrations and queries
  couchside-library    folder scanning and watching, ffprobe and ffmpeg frame grabs
  couchside-metadata   TMDB client and image cache
  couchside-server     couchside.exe: HTTP API, web UI, Windows service
  couchside-transcode  playback decisions (direct play, remux, transcode) and transcode temp files
docs/                 API reference and architecture decisions
Rules.md              project rules for contributors
```

See [docs/api.md](docs/api.md) for the HTTP API.

## License

MIT OR Apache-2.0

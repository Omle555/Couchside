# Solidfin — Project Rules

Solidfin is a Windows-first, self-hosted movie and TV media server (in the spirit of Jellyfin and Plex), written in Rust. These rules apply to all code, docs and contributions in this repository — including work done by AI assistants.

When a rule here conflicts with a convenience, the rule wins. When a rule is wrong, change this file in its own commit with a reason.

---

## 1. Project scope

1. **Primary platform is Windows 10/11 (x64).** Everything must build, run and be tested on Windows first. Cross-platform support (Linux/macOS) is welcome but must never break or complicate the Windows path.
2. **Core features, in priority order:**
   1. Media library scanning (movies, TV shows/seasons/episodes)
   2. Metadata fetching (TMDB/TVDB/OMDb) with local caching
   3. HTTP API + web UI for browsing
   4. Direct play / direct stream
   5. On-the-fly transcoding via FFmpeg
   6. Multi-user accounts, watch progress, "continue watching"
   7. Subtitles (embedded + external `.srt`/`.ass`/`.vtt`)
   8. Running as a Windows Service with a tray app
3. **Out of scope until the core is solid:** live TV/DVR, music, photos, plugins, remote/cloud relay.
4. No feature is merged without a clear user-facing purpose.

## 2. Language and toolchain

1. **Rust stable only.** No nightly features. Pin the toolchain in `rust-toolchain.toml`.
2. **Edition 2021** (or newer once adopted project-wide — never mixed).
3. Target `x86_64-pc-windows-msvc`. Do not depend on MinGW/GNU toolchains.
4. Use a **Cargo workspace** with focused crates, for example:
   - `solidfin-server` — binary, wiring, config, startup
   - `solidfin-api` — HTTP routes and DTOs
   - `solidfin-library` — scanning, file watching, naming parsers
   - `solidfin-metadata` — external metadata providers
   - `solidfin-transcode` — FFmpeg integration
   - `solidfin-db` — database schema, migrations, queries
   - `solidfin-core` — shared types and errors
5. Crates depend inward on `solidfin-core`; no circular dependencies.
6. Current layout (v0.1): `solidfin-api` lives as the `api` module inside `solidfin-server`, and `solidfin-transcode` will be added with transcoding (see `docs/adr/0001-architecture.md`).

## 3. Approved core stack

Changing any of these requires a written reason in the PR.

| Concern | Choice |
|---|---|
| Async runtime | `tokio` |
| HTTP server | `axum` (+ `tower`, `tower-http`) |
| Database | SQLite via `sqlx` (parameterized runtime queries, each covered by tests; migrations in `crates/solidfin-db/migrations/`; see ADR 0001) |
| Serialization | `serde`, `serde_json` |
| Config | TOML via `serde` + `toml` |
| Logging/tracing | `tracing`, `tracing-subscriber`, rolling file logs |
| Errors | `thiserror` in libraries, `anyhow` only in the binary |
| HTTP client | `reqwest` (rustls) |
| File watching | `notify` |
| Windows service | `windows-service` |
| Password hashing | `argon2` |
| Media probing/transcoding | External `ffmpeg` / `ffprobe` binaries (not linked) |

## 4. Code quality

1. **`cargo fmt` is mandatory.** Unformatted code is not merged.
2. **`cargo clippy --all-targets --all-features -- -D warnings` must pass.**
3. **No `unwrap()` or `expect()` in non-test code**, except where an invariant is provably true — and then use `expect("reason this can't fail")`.
4. **No `unsafe`** unless required for Windows FFI. Every `unsafe` block needs a `// SAFETY:` comment explaining why it is sound. Prefer the `windows` crate over raw FFI.
5. No `println!` / `eprintln!` for diagnostics — use `tracing`.
6. Public items in library crates have doc comments (`///`).
7. Functions do one thing. If a function needs a comment to explain its sections, split it.
8. No dead code, commented-out code or `TODO`s without a linked issue.
9. Prefer strong types over primitives: `MediaId`, `UserId`, `LibraryId` newtypes instead of bare `i64`/`String`.

## 5. Error handling

1. Library crates define their own error enums with `thiserror`.
2. Errors are propagated with `?`, not swallowed. If an error is intentionally ignored, log it and say why.
3. API errors map to proper HTTP status codes with a consistent JSON body: `{ "error": "code", "message": "human readable" }`.
4. Never leak internal paths, stack traces or SQL in API responses.
5. A single bad media file must never crash the scanner or the server.

## 6. Windows-specific rules

1. **Paths:** always use `std::path::Path`/`PathBuf`. Never build paths with string concatenation or hard-coded `/` or `\`.
2. Support **long paths** (`\\?\` prefix) and **Unicode file names** — test with non-ASCII titles.
3. Support **UNC/network shares** (`\\NAS\Movies`) and mapped drives as library roots.
4. File comparisons are **case-insensitive** on Windows.
5. Default data locations:
   - Program data: `%ProgramData%\Solidfin\` (database, cache, logs, transcodes)
   - Config: `%ProgramData%\Solidfin\config.toml`
   - Never write into the install directory or the user's media folders.
6. **Never modify, move or delete user media files.** Solidfin is read-only against libraries, with one opt-in exception: "Clean up" may rename a movie file (and the subtitle, NFO and artwork files named after it) to the standard layout in 8.1, and move the images and subtitles in the movie's folder (only its own ones when the folder holds other movies; never a video file) into `%ProgramData%\Solidfin\removed\` instead of deleting them; and "Delete" may send a movie's own folder (or, in a shared folder, only the movie and the files named after it) to the Recycle Bin, only when file changes are allowed ("Allow file renames and deletes" in Settings or `[library] allow_file_renames = true`; off by default), only after showing the old and new names, never overwriting anything, never leaving the library root, keeping watch progress, listing every rename and removal in the preview, and logging each one to `logs\renames.log` so it can be undone. Artwork/NFO writing, if ever added, must be opt-in too. *(Changed 2026-10-13 at the owner's request: tidy names were wanted and an off-by-default setting with a preview keeps files safe from a bad match. Image and subtitle removal and the Delete button added the same day at the owner's request, both recoverable: a move and the Recycle Bin.)*
7. The server must run both as a console app (for development) and as a Windows Service.
8. Respect Windows Firewall — the installer requests a rule only for the configured port.

## 7. Media and FFmpeg

1. FFmpeg/ffprobe are invoked as child processes via `tokio::process`. Paths are configurable; auto-detect from `PATH` as a fallback.
2. **Always pass arguments as a list** — never build a shell command string (prevents injection via file names).
3. Every spawned FFmpeg process must be tracked and killed when the client disconnects, the session ends or the server shuts down. No orphaned processes.
4. Prefer **direct play > remux > transcode**. Only transcode when the client can't play the source.
5. Hardware acceleration (NVENC, QSV, AMF/D3D11VA) is opt-in and must fall back to software on failure.
6. Transcode temp files live in the transcode cache and are cleaned up on session end and at startup.
7. Probe results are cached in the database keyed by path + size + modified time.

## 8. Library scanning and naming

1. Follow common naming conventions (compatible with Jellyfin/Plex/Kodi):
   - Movies: `Movie Title (Year)/Movie Title (Year).ext`
   - TV: `Show Name (Year)/Season 01/Show Name - S01E02 - Episode Title.ext`
2. The name parser is a pure function with extensive unit tests — every supported pattern has a test case.
3. Scans are incremental: only new/changed/removed files are processed.
4. Scanning runs in the background and must not block API requests.
5. Unmatched items are still shown (with file name) rather than hidden.

## 9. API design

1. REST-style JSON API under `/api/v1/`. Breaking changes require a new version prefix.
2. Streaming endpoints support **HTTP Range requests** for seeking.
3. Every route except login, health and static assets requires authentication.
4. Pagination for any list endpoint that can exceed 100 items (`?limit=&offset=`).
5. Request and response types live in DTO structs — never serialize database rows directly.
6. Document endpoints (OpenAPI via `utoipa` or a maintained `docs/api.md`).

## 10. Security

1. Passwords hashed with Argon2id. Never stored or logged in plain text.
2. Auth tokens are random, stored hashed, and revocable.
3. **Validate every path from a client:** resolve it and confirm it is inside a configured library root before reading (prevents path traversal).
4. API keys for metadata providers live in config, never in source control.
5. Bind to `127.0.0.1` by default; LAN exposure is an explicit setting.
6. No telemetry or outbound calls except to configured metadata providers and update checks the user enabled.
7. Run `cargo audit` (or `cargo deny`) in CI; no known-vulnerable dependencies.

## 11. Database

1. All schema changes are `sqlx` migrations — never edit an applied migration.
2. Use parameterized queries only. No string-built SQL.
3. Enable WAL mode and foreign keys.
4. Back up the database automatically before running migrations.

## 12. Performance

1. Never do blocking I/O on the async runtime — use `tokio::fs` or `spawn_blocking`.
2. Image resizing and hashing run in `spawn_blocking` or a worker pool.
3. Target: library of 10,000 items browses with sub-200 ms API responses on modest hardware.
4. Cache metadata images locally; never hot-link provider images to clients.

## 13. Testing

1. Every crate has unit tests; parsers and path validation have exhaustive tests.
2. Integration tests spin up the server against a temporary SQLite database and a small fixture library.
3. Do not commit real copyrighted media as fixtures — generate tiny test clips with FFmpeg or use public-domain samples.
4. `cargo test --workspace` must pass on Windows before merging.
5. Bugs get a regression test before the fix.

## 14. Dependencies

1. Prefer well-maintained, widely used crates. Check recent releases and download counts before adding.
2. Every new dependency is justified in the PR description.
3. Enable only the features you use (`default-features = false` where sensible).
4. Licenses must be compatible with the project license (MIT/Apache-2.0/BSD/Zlib preferred; no GPL linking).

## 15. Git workflow

1. `main` is always buildable and releasable.
2. Work on branches: `feature/…`, `fix/…`, `docs/…`, `chore/…`.
3. **Conventional Commits:** `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `perf:`.
4. Small, focused commits and PRs — one concern per PR.
5. CI (GitHub Actions on `windows-latest`) runs fmt, clippy, tests and audit on every PR.
6. Never commit secrets, `config.toml` with real keys, databases, logs or `target/`.

## 16. Versioning and releases

1. Semantic Versioning (`MAJOR.MINOR.PATCH`).
2. Keep a `CHANGELOG.md` (Keep a Changelog format).
3. Releases ship a Windows installer (MSI via `cargo-wix` or Inno Setup) plus a portable zip.
4. Release builds use `--release` with LTO enabled and debug symbols kept separately.

## 17. Documentation

1. `README.md` covers: what Solidfin is, install, first run, adding a library, building from source.
2. Architecture decisions are recorded as short ADRs in `docs/adr/NNNN-title.md`.
3. User-facing settings are documented next to their defaults in `config.example.toml`.

## 18. Rules for AI-assisted work

1. Follow every rule in this file; read it before starting work.
2. Do not add dependencies, change the core stack or restructure crates without asking first.
3. Make the smallest change that solves the task; don't refactor unrelated code.
4. Code must compile and pass `fmt`, `clippy` and tests before being presented as done. If something couldn't be verified, say so.
5. Never touch or generate code that modifies user media files, except the opt-in "Clean up" rename, image/subtitle removal and "Delete" described in 6.6 and its safeguards.
6. Explain non-obvious decisions in the PR/commit, not in long code comments.

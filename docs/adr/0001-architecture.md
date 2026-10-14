# ADR 0001: Overall architecture for v0.1

- Status: accepted
- Date: 2026-10-11

## Context

Couchside should be a lightweight Windows media server: easy to install, a single executable,
and gentle on resources. Rules.md fixes the core stack (tokio, axum, SQLite via sqlx, tracing).

## Decisions

1. **Single executable.** The web UI (plain HTML/CSS/JS, no build step) is embedded with
   `include_str!`. No Node toolchain is needed to build Couchside.
2. **Direct play only in v0.1.** Files are streamed as-is with HTTP Range support. Transcoding is
   the next major feature and will live in a new `couchside-transcode` crate.
3. **Runtime-checked SQL.** Queries use `sqlx::query_as` with parameters instead of the
   compile-time `query!` macros, so building does not require a live database or `.sqlx` offline
   files. Every query is covered by tests against an in-memory database.
4. **API and server in one crate.** The `couchside-api` crate suggested in Rules.md is a module of
   `couchside-server` for now; it can be split out when a second consumer appears.
5. **Cookie sessions.** `<video>` and `<img>` tags can't send `Authorization` headers, so the
   web UI uses an HttpOnly, SameSite=Strict cookie. API clients can use `Authorization: Bearer`.
   Only SHA-256 hashes of session tokens are stored.
6. **Scanning is incremental and conservative.** Files are identified by path; unchanged size and
   modified time skip re-processing. If a library root is unreachable, or any sub-folder cannot be
   read, nothing is removed from the database.
7. **Background work is serialised.** One scan → probe → metadata run at a time; API handlers
   never wait on it.

## Consequences

- Browsers can't play every file (e.g. DTS audio, AVI). The UI warns about likely-unsupported
  files until transcoding lands.
- Without compile-time query checking, SQL typos surface in tests rather than at build time, so
  new queries must come with tests.

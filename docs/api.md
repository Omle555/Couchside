# Solidfin HTTP API (v1)

Base path: `/api/v1`. All bodies are JSON. Errors look like:

```json
{ "error": "bad_request", "message": "Human readable explanation" }
```

Authentication: sign in with `POST /auth/login`. The response sets an HttpOnly `solidfin_session`
cookie and also returns `token`, which non-browser clients send as `Authorization: Bearer <token>`.
Sessions expire after 30 days without use. Endpoints marked **admin** need an administrator.

With `[server] lan_only = true` (the default), every request (API and web UI) from an address outside
the local network gets `403` with `{ "error": "outside_local_network", ... }`.

## Setup & auth

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/health` | none | `{ status, version }` |
| GET | `/setup` | none | `{ needs_setup, version, ffprobe, ffmpeg, tmdb }` |
| POST | `/setup` | none | `{ username, password }` creates the first admin (only when no users exist) |
| POST | `/auth/login` | none | `{ username, password }` → `{ token, user }` |
| POST | `/auth/logout` | any | Ends the session |
| GET | `/auth/me` | user | Current user |

## Libraries & scanning

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/libraries` | user | List libraries |
| POST | `/libraries` | admin | `{ name, kind: "movies" \| "shows", path }`; starts a scan |
| DELETE | `/libraries/{id}` | admin | Forget a library (files untouched) |
| GET | `/scan` | user | `{ running, phase, last_started, last_finished, errors[] }` |
| POST | `/scan` | admin | Start a full scan → `{ started }` |
| POST | `/metadata/refresh` | admin | Re-fetch all TMDB metadata |
| GET | `/settings/metadata` | admin | `{ has_key, key_source: "app"\|"config"\|null, key_hint, language }` |
| PUT | `/settings/metadata` | admin | `{ tmdb_api_key?, language? }`; the key is checked with TMDB first; `""` removes it |

## Browsing

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/home` | user | `{ continue_watching[], recent_movies[], recent_episodes[] }` |
| GET | `/movies?library_id=&sort=title\|added\|year&limit=&offset=` | user | Page of movies |
| GET | `/shows?library_id=&limit=&offset=` | user | Page of shows |
| GET | `/shows/{id}` | user | `{ show, seasons: [{ season, episodes[] }] }` |
| GET | `/items/{id}` | user | `{ item, show_title, file_name, file_size, browser_playable, subtitles[] }` |
| GET | `/search?q=` | user | `{ movies[], shows[] }` |
| PUT | `/items/{id}/progress` | user | `{ position_seconds, played? }`; auto-marks watched at 90% |
| GET | `/items/{id}/identify?q=&year=` | admin | TMDB search results for a movie (defaults to the parsed file name) |
| POST | `/items/{id}/identify` | admin | `{ tmdb_id }` uses that movie and keeps it on future refreshes |
| GET, PUT | `/settings/library` | admin | `{ allow_file_renames }`: lets Clean up rename movie files. Saved in the database and overrides `[library] allow_file_renames` in config.toml |
| GET | `/items/{id}/cleanup` | admin | What Clean up would do: `{ renames_enabled, current_name, new_name, other_steps[{ from, to }], rename_note, remove_files[] }`. Names are relative to the library folder; `new_name` is only set when file renames are allowed and the file isn't already named `Title (Year)/Title (Year) 1080p.ext`. 400 if the movie isn't matched to TMDB |
| POST | `/items/{id}/cleanup` | admin | `{ rename_to?, remove_files? }` deletes the movie's stored details and artwork and fetches them again for its current TMDB match. Renames the file (and its subtitles, .nfo and artwork) only if renaming is switched on and `rename_to` equals the new name the preview gave. Moves the listed images and subtitles (only ones the preview would also list) to `<data>\removed\`. Returns `{ renamed, message }`. 409 while a library scan runs |
| GET | `/items/{id}/delete` | admin | What deleting a movie removes: `{ folder, files[] }` (its own folder, or in a shared folder just the movie and the files named after it), relative to the library folder. 400 unless file changes are allowed |
| POST | `/items/{id}/delete` | admin | Body: the preview `{ folder, files }`. Sends exactly those to the Recycle Bin and removes the movie from Solidfin (204). 409 if the files changed since the preview or a scan is running |
| POST | `/items/{id}/title-scan` | admin | Reads on-screen text from the first 10 minutes of a movie: `{ frames[{ index, time_seconds, lines[] }], suggestions[], ocr_available, frames_read }`. Never changes the match. 409 while another scan runs |
| GET | `/items/{id}/title-scan/frames/{index}` | admin | JPEG of a frame from that movie's last title scan |
| GET | `/tmdb/image?path=` | user | Small TMDB poster for the Identify dialog, fetched through the server |

Movie items also include `tmdb_id`, `imdb_id`, `original_title`, `tagline`, `genres[]`, `rating`,
`release_date`, `certification`, `cast[]`, `directors[]` and `match_locked`.

Pages look like `{ items, total, limit, offset }`. `limit` defaults to 100 (max 500).

## Media files

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/items/{id}/stream` | user | Original file, supports `Range` |
| GET | `/items/{id}/image/poster\|backdrop` | user | Cached artwork (episodes fall back to the show poster); the web UI adds `?v={tmdb_id}`, which the server ignores, so a new match isn't hidden by the browser cache |
| GET | `/shows/{id}/image/poster\|backdrop` | user | Cached show artwork |
| GET | `/subtitles/{id}` | user | External subtitle converted to WebVTT |

## Users

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/users` | admin | List users |
| POST | `/users` | admin | `{ username, password, is_admin }` |
| DELETE | `/users/{id}` | admin | Delete (not yourself, not the last admin) |
| PUT | `/users/{id}/password` | self or admin | `{ password }`; signs that user out everywhere |

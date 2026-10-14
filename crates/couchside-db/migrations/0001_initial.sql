-- Solidfin initial schema.

CREATE TABLE libraries (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL,
    kind        TEXT    NOT NULL CHECK (kind IN ('movies', 'shows')),
    path        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    created_at  INTEGER NOT NULL
);

CREATE TABLE users (
    id             INTEGER PRIMARY KEY,
    username       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    password_hash  TEXT    NOT NULL,
    is_admin       INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash  TEXT    PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);

CREATE TABLE shows (
    id                INTEGER PRIMARY KEY,
    library_id        INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    folder_path       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    parsed_title      TEXT    NOT NULL,
    parsed_year       INTEGER,
    title             TEXT    NOT NULL,
    sort_title        TEXT    NOT NULL,
    year              INTEGER,
    overview          TEXT,
    poster_file       TEXT,
    backdrop_file     TEXT,
    tmdb_id           INTEGER,
    metadata_fetched  INTEGER NOT NULL DEFAULT 0,
    added_at          INTEGER NOT NULL
);
CREATE INDEX idx_shows_library ON shows(library_id);
CREATE INDEX idx_shows_sort ON shows(sort_title);

CREATE TABLE media_items (
    id                INTEGER PRIMARY KEY,
    library_id        INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    kind              TEXT    NOT NULL CHECK (kind IN ('movie', 'episode')),
    path              TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    file_size         INTEGER NOT NULL,
    file_mtime        INTEGER NOT NULL,
    parsed_title      TEXT    NOT NULL,
    parsed_year       INTEGER,
    title             TEXT    NOT NULL,
    sort_title        TEXT    NOT NULL,
    year              INTEGER,
    show_id           INTEGER REFERENCES shows(id) ON DELETE CASCADE,
    season            INTEGER,
    episode           INTEGER,
    episode_end       INTEGER,
    overview          TEXT,
    poster_file       TEXT,
    backdrop_file     TEXT,
    tmdb_id           INTEGER,
    runtime_seconds   REAL,
    container         TEXT,
    video_codec       TEXT,
    audio_codec       TEXT,
    width             INTEGER,
    height            INTEGER,
    probed            INTEGER NOT NULL DEFAULT 0,
    metadata_fetched  INTEGER NOT NULL DEFAULT 0,
    added_at          INTEGER NOT NULL
);
CREATE INDEX idx_media_library ON media_items(library_id);
CREATE INDEX idx_media_kind_sort ON media_items(kind, sort_title);
CREATE INDEX idx_media_kind_added ON media_items(kind, added_at);
CREATE INDEX idx_media_show ON media_items(show_id, season, episode);

CREATE TABLE subtitles (
    id        INTEGER PRIMARY KEY,
    media_id  INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    path      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    language  TEXT,
    format    TEXT    NOT NULL
);
CREATE INDEX idx_subtitles_media ON subtitles(media_id);

CREATE TABLE watch_progress (
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id          INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    position_seconds  REAL    NOT NULL DEFAULT 0,
    played            INTEGER NOT NULL DEFAULT 0,
    updated_at        INTEGER NOT NULL,
    PRIMARY KEY (user_id, media_id)
);
CREATE INDEX idx_progress_recent ON watch_progress(user_id, updated_at);

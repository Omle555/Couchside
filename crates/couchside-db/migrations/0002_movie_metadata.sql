-- Richer movie metadata, ids from file names, manual matches and app settings.

-- Ids written in file/folder names, e.g. "Movie (2010) {tmdb-27205}".
ALTER TABLE media_items ADD COLUMN hint_tmdb_id INTEGER;
ALTER TABLE media_items ADD COLUMN hint_imdb_id TEXT;
-- Set when the file name must be parsed again even though the file is unchanged.
ALTER TABLE media_items ADD COLUMN needs_reparse INTEGER NOT NULL DEFAULT 0;
-- Set when an administrator picked the TMDB match by hand ("Identify").
ALTER TABLE media_items ADD COLUMN match_locked INTEGER NOT NULL DEFAULT 0;

ALTER TABLE media_items ADD COLUMN original_title TEXT;
ALTER TABLE media_items ADD COLUMN tagline TEXT;
ALTER TABLE media_items ADD COLUMN genres TEXT;          -- JSON array of names
ALTER TABLE media_items ADD COLUMN rating REAL;          -- TMDB average, 0-10
ALTER TABLE media_items ADD COLUMN release_date TEXT;    -- YYYY-MM-DD
ALTER TABLE media_items ADD COLUMN certification TEXT;   -- e.g. M, PG-13
ALTER TABLE media_items ADD COLUMN imdb_id TEXT;
ALTER TABLE media_items ADD COLUMN cast_names TEXT;      -- JSON array of names
ALTER TABLE media_items ADD COLUMN directors TEXT;       -- JSON array of names
ALTER TABLE media_items ADD COLUMN tmdb_runtime_minutes INTEGER;

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- Re-read every file name once (to pick up id tags) and re-match movies with
-- the improved matcher so the new details are filled in.
UPDATE media_items SET needs_reparse = 1;
UPDATE media_items SET metadata_fetched = 0 WHERE kind = 'movie';

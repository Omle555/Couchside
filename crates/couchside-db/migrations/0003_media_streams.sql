-- Per-stream details from ffprobe, used to decide how a file can be played
-- (direct play, remux, audio convert or full transcode).

ALTER TABLE media_items ADD COLUMN bitrate INTEGER;  -- overall bits per second

CREATE TABLE media_streams (
    media_id        INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    stream_index    INTEGER NOT NULL,                -- ffprobe stream index
    kind            TEXT    NOT NULL CHECK (kind IN ('video', 'audio', 'subtitle')),
    codec           TEXT,
    profile         TEXT,
    pix_fmt         TEXT,
    bit_depth       INTEGER,
    color_transfer  TEXT,                            -- smpte2084 = HDR10, arib-std-b67 = HLG
    channels        INTEGER,
    language        TEXT,
    title           TEXT,
    is_default      INTEGER NOT NULL DEFAULT 0,
    is_forced       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (media_id, stream_index)
);

-- Read every file once more so the stream details are filled in.
UPDATE media_items SET probed = 0;

-- Keyframe times of a file's video, read by ffprobe the first time the file is repackaged
-- for streaming. Kept only while the file's size and modified time are unchanged.
CREATE TABLE keyframes (
    media_id    INTEGER PRIMARY KEY REFERENCES media_items(id) ON DELETE CASCADE,
    file_size   INTEGER NOT NULL,
    file_mtime  INTEGER NOT NULL,
    times_ms    TEXT    NOT NULL   -- JSON array of milliseconds from the start of the video
);

//! Media item, show and subtitle queries.

use crate::{Db, DbResult, MediaItem, Show, Subtitle};
use couchside_core::naming::sort_title;
use couchside_core::{now_unix, LibraryId, MediaId, MediaKind, ShowId, SubtitleId};

/// Columns selected for [`MediaItem`]; the table must be aliased as `m`.
pub(crate) const MEDIA_COLUMNS: &str = "m.id, m.library_id, m.kind, m.path, m.file_size, m.title, \
    m.sort_title, m.year, m.show_id, m.season, m.episode, m.episode_end, m.overview, m.poster_file, \
    m.backdrop_file, m.tmdb_id, m.runtime_seconds, m.container, m.video_codec, m.audio_codec, \
    m.width, m.height, m.added_at, m.original_title, m.tagline, m.genres, m.rating, \
    m.release_date, m.certification, m.imdb_id, m.cast_names, m.directors, m.match_locked";

/// Columns selected for [`Show`]; the table must be aliased as `s`.
const SHOW_COLUMNS: &str = "s.id, s.library_id, s.folder_path, s.title, s.sort_title, s.year, \
    s.overview, s.poster_file, s.backdrop_file, s.tmdb_id, s.added_at, \
    (SELECT COUNT(*) FROM media_items e WHERE e.show_id = s.id) AS episode_count";

/// A scanned file to insert or update.
#[derive(Debug, Clone)]
pub struct NewMedia {
    /// Owning library.
    pub library_id: LibraryId,
    /// Movie or episode.
    pub kind: MediaKind,
    /// File path.
    pub path: String,
    /// Size in bytes.
    pub file_size: i64,
    /// Modified time (Unix seconds).
    pub file_mtime: i64,
    /// Title parsed from the file name.
    pub parsed_title: String,
    /// Year parsed from the file name.
    pub parsed_year: Option<i64>,
    /// Parent show for episodes.
    pub show_id: Option<ShowId>,
    /// Season number for episodes.
    pub season: Option<i64>,
    /// Episode number for episodes.
    pub episode: Option<i64>,
    /// Last episode number for multi-episode files.
    pub episode_end: Option<i64>,
    /// TMDB id written in the file or folder name.
    pub hint_tmdb_id: Option<i64>,
    /// IMDb id written in the file or folder name.
    pub hint_imdb_id: Option<String>,
}

/// What the scanner knows about a file from the last scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileState {
    /// Item id.
    pub id: MediaId,
    /// Size in bytes at the last scan.
    pub size: i64,
    /// Modified time at the last scan.
    pub mtime: i64,
    /// The name must be parsed again (e.g. after a parser upgrade).
    pub needs_reparse: bool,
}

/// An external subtitle file found next to a video.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NewSubtitle {
    /// File path.
    pub path: String,
    /// Language tag from the file name.
    pub language: Option<String>,
    /// Lowercase extension.
    pub format: String,
}

/// Technical details read by ffprobe.
#[derive(Debug, Clone, Default)]
pub struct ProbeData {
    /// Duration in seconds.
    pub runtime_seconds: Option<f64>,
    /// Container format name.
    pub container: Option<String>,
    /// First video codec.
    pub video_codec: Option<String>,
    /// First audio codec.
    pub audio_codec: Option<String>,
    /// Video width.
    pub width: Option<i64>,
    /// Video height.
    pub height: Option<i64>,
}

/// Online metadata for a movie, show or episode. `None` fields keep the current value.
#[derive(Debug, Clone, Default)]
pub struct ItemMetadata {
    /// TMDB id.
    pub tmdb_id: Option<i64>,
    /// Official title.
    pub title: Option<String>,
    /// Release / first-air year.
    pub year: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// Cached poster file name.
    pub poster_file: Option<String>,
    /// Cached backdrop or still file name.
    pub backdrop_file: Option<String>,
}

/// Alias kept for readability at call sites dealing with shows.
pub type ShowMetadata = ItemMetadata;

/// Sort orders for the movie list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MovieSort {
    /// Alphabetical by sort title.
    #[default]
    Title,
    /// Newest additions first.
    Added,
    /// Newest release year first.
    Year,
}

impl MovieSort {
    fn order_by(self) -> &'static str {
        match self {
            Self::Title => "m.sort_title ASC, m.year ASC",
            Self::Added => "m.added_at DESC, m.id DESC",
            Self::Year => "m.year IS NULL, m.year DESC, m.sort_title ASC",
        }
    }
}

/// Escapes `%`, `_` and `\` for a `LIKE ... ESCAPE '\'` pattern.
fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl Db {
    // ----- Scanner -----------------------------------------------------------

    /// Size, mtime and re-parse flag last recorded for a file path.
    pub async fn file_state(&self, path: &str) -> DbResult<Option<FileState>> {
        let row = sqlx::query_as::<_, (MediaId, i64, i64, bool)>(
            "SELECT id, file_size, file_mtime, needs_reparse FROM media_items WHERE path = ?",
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, size, mtime, needs_reparse)| FileState {
            id,
            size,
            mtime,
            needs_reparse,
        }))
    }

    /// Inserts or updates a scanned file. Metadata is kept unless the parsed
    /// title, year or id tags changed (e.g. the file was renamed to fix a bad match).
    pub async fn upsert_media(&self, m: &NewMedia) -> DbResult<MediaId> {
        const SAME: &str = "media_items.parsed_title = excluded.parsed_title \
                            AND media_items.parsed_year IS excluded.parsed_year \
                            AND media_items.hint_tmdb_id IS excluded.hint_tmdb_id \
                            AND media_items.hint_imdb_id IS excluded.hint_imdb_id";
        const SAME_FILE: &str = "media_items.file_size = excluded.file_size \
                                 AND media_items.file_mtime = excluded.file_mtime";
        let sql = format!(
            "INSERT INTO media_items (library_id, kind, path, file_size, file_mtime, parsed_title,
                 parsed_year, title, sort_title, year, show_id, season, episode, episode_end,
                 hint_tmdb_id, hint_imdb_id, added_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET
                 library_id = excluded.library_id,
                 probed = CASE WHEN {SAME_FILE} THEN media_items.probed ELSE 0 END,
                 file_size = excluded.file_size,
                 file_mtime = excluded.file_mtime,
                 needs_reparse = 0,
                 match_locked = CASE WHEN {SAME} THEN media_items.match_locked ELSE 0 END,
                 show_id = excluded.show_id,
                 season = excluded.season,
                 episode = excluded.episode,
                 episode_end = excluded.episode_end,
                 metadata_fetched = CASE WHEN {SAME} THEN media_items.metadata_fetched ELSE 0 END,
                 title = CASE WHEN {SAME} THEN media_items.title ELSE excluded.title END,
                 sort_title = CASE WHEN {SAME} THEN media_items.sort_title ELSE excluded.sort_title END,
                 year = CASE WHEN {SAME} THEN media_items.year ELSE excluded.year END,
                 parsed_title = excluded.parsed_title,
                 parsed_year = excluded.parsed_year,
                 hint_tmdb_id = excluded.hint_tmdb_id,
                 hint_imdb_id = excluded.hint_imdb_id
             RETURNING id"
        );
        Ok(sqlx::query_scalar::<_, MediaId>(&sql)
            .bind(m.library_id)
            .bind(m.kind)
            .bind(&m.path)
            .bind(m.file_size)
            .bind(m.file_mtime)
            .bind(&m.parsed_title)
            .bind(m.parsed_year)
            .bind(&m.parsed_title)
            .bind(sort_title(&m.parsed_title))
            .bind(m.parsed_year)
            .bind(m.show_id)
            .bind(m.season)
            .bind(m.episode)
            .bind(m.episode_end)
            .bind(m.hint_tmdb_id)
            .bind(&m.hint_imdb_id)
            .bind(now_unix())
            .fetch_one(&self.pool)
            .await?)
    }

    /// Inserts or updates a show folder and returns its id.
    pub async fn upsert_show(
        &self,
        library_id: LibraryId,
        folder_path: &str,
        parsed_title: &str,
        parsed_year: Option<i64>,
    ) -> DbResult<ShowId> {
        const SAME: &str =
            "shows.parsed_title = excluded.parsed_title AND shows.parsed_year IS excluded.parsed_year";
        let sql = format!(
            "INSERT INTO shows (library_id, folder_path, parsed_title, parsed_year, title, sort_title, year, added_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(folder_path) DO UPDATE SET
                 library_id = excluded.library_id,
                 metadata_fetched = CASE WHEN {SAME} THEN shows.metadata_fetched ELSE 0 END,
                 title = CASE WHEN {SAME} THEN shows.title ELSE excluded.title END,
                 sort_title = CASE WHEN {SAME} THEN shows.sort_title ELSE excluded.sort_title END,
                 year = CASE WHEN {SAME} THEN shows.year ELSE excluded.year END,
                 parsed_title = excluded.parsed_title,
                 parsed_year = excluded.parsed_year
             RETURNING id"
        );
        Ok(sqlx::query_scalar::<_, ShowId>(&sql)
            .bind(library_id)
            .bind(folder_path)
            .bind(parsed_title)
            .bind(parsed_year)
            .bind(parsed_title)
            .bind(sort_title(parsed_title))
            .bind(parsed_year)
            .bind(now_unix())
            .fetch_one(&self.pool)
            .await?)
    }

    /// Every file path currently recorded for a library.
    pub async fn library_media_paths(
        &self,
        library_id: LibraryId,
    ) -> DbResult<Vec<(MediaId, String)>> {
        Ok(sqlx::query_as::<_, (MediaId, String)>(
            "SELECT id, path FROM media_items WHERE library_id = ?",
        )
        .bind(library_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Removes items whose files have disappeared.
    pub async fn delete_media(&self, ids: &[MediaId]) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        for id in ids {
            sqlx::query("DELETE FROM media_items WHERE id = ?")
                .bind(*id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Removes shows that no longer have any episode files.
    pub async fn delete_empty_shows(&self, library_id: LibraryId) -> DbResult<u64> {
        let result = sqlx::query(
            "DELETE FROM shows WHERE library_id = ?
             AND NOT EXISTS (SELECT 1 FROM media_items m WHERE m.show_id = shows.id)",
        )
        .bind(library_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Subtitle file paths currently linked to an item, sorted.
    pub async fn subtitle_paths(&self, media_id: MediaId) -> DbResult<Vec<String>> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT path FROM subtitles WHERE media_id = ? ORDER BY path",
        )
        .bind(media_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Unlinks subtitle files that "Clean up" moved out of the library.
    pub async fn delete_subtitles(&self, media_id: MediaId, paths: &[String]) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        for path in paths {
            sqlx::query("DELETE FROM subtitles WHERE media_id = ? AND path = ?")
                .bind(media_id)
                .bind(path)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Replaces the external subtitles linked to an item.
    pub async fn replace_subtitles(&self, media_id: MediaId, subs: &[NewSubtitle]) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM subtitles WHERE media_id = ?")
            .bind(media_id)
            .execute(&mut *tx)
            .await?;
        for sub in subs {
            sqlx::query("INSERT OR IGNORE INTO subtitles (media_id, path, language, format) VALUES (?, ?, ?, ?)")
                .bind(media_id)
                .bind(&sub.path)
                .bind(&sub.language)
                .bind(&sub.format)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    // ----- Probing -----------------------------------------------------------

    /// Items not yet examined by ffprobe.
    pub async fn items_needing_probe(&self, limit: i64) -> DbResult<Vec<(MediaId, String)>> {
        Ok(sqlx::query_as::<_, (MediaId, String)>(
            "SELECT id, path FROM media_items WHERE probed = 0 ORDER BY added_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Stores ffprobe results. `None` marks the item as probed without data (probe failed).
    pub async fn set_probe_result(&self, id: MediaId, data: Option<&ProbeData>) -> DbResult<()> {
        let empty = ProbeData::default();
        let d = data.unwrap_or(&empty);
        sqlx::query(
            "UPDATE media_items SET probed = 1, runtime_seconds = ?, container = ?, video_codec = ?,
                 audio_codec = ?, width = ?, height = ?
             WHERE id = ?",
        )
        .bind(d.runtime_seconds)
        .bind(&d.container)
        .bind(&d.video_codec)
        .bind(&d.audio_codec)
        .bind(d.width)
        .bind(d.height)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ----- Metadata ----------------------------------------------------------

    /// Shows whose online metadata has not been fetched yet.
    pub async fn shows_needing_metadata(&self, limit: i64) -> DbResult<Vec<Show>> {
        let sql = format!(
            "SELECT {SHOW_COLUMNS} FROM shows s WHERE s.metadata_fetched = 0 ORDER BY s.added_at DESC LIMIT ?"
        );
        Ok(sqlx::query_as::<_, Show>(&sql)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Episodes needing metadata whose show has already been matched on TMDB.
    pub async fn episodes_needing_metadata(&self, limit: i64) -> DbResult<Vec<MediaItem>> {
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM media_items m JOIN shows s ON s.id = m.show_id
             WHERE m.kind = 'episode' AND m.metadata_fetched = 0 AND s.tmdb_id IS NOT NULL
             ORDER BY m.show_id, m.season, m.episode LIMIT ?"
        );
        Ok(sqlx::query_as::<_, MediaItem>(&sql)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Saves metadata for a movie or episode and marks it fetched.
    pub async fn set_item_metadata(&self, id: MediaId, meta: &ItemMetadata) -> DbResult<()> {
        sqlx::query(
            "UPDATE media_items SET metadata_fetched = 1,
                 tmdb_id = COALESCE(?, tmdb_id),
                 title = COALESCE(?, title),
                 sort_title = COALESCE(?, sort_title),
                 year = COALESCE(?, year),
                 overview = COALESCE(?, overview),
                 poster_file = COALESCE(?, poster_file),
                 backdrop_file = COALESCE(?, backdrop_file)
             WHERE id = ?",
        )
        .bind(meta.tmdb_id)
        .bind(&meta.title)
        .bind(meta.title.as_deref().map(sort_title))
        .bind(meta.year)
        .bind(&meta.overview)
        .bind(&meta.poster_file)
        .bind(&meta.backdrop_file)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Saves metadata for a show and marks it fetched.
    pub async fn set_show_metadata(&self, id: ShowId, meta: &ShowMetadata) -> DbResult<()> {
        sqlx::query(
            "UPDATE shows SET metadata_fetched = 1,
                 tmdb_id = COALESCE(?, tmdb_id),
                 title = COALESCE(?, title),
                 sort_title = COALESCE(?, sort_title),
                 year = COALESCE(?, year),
                 overview = COALESCE(?, overview),
                 poster_file = COALESCE(?, poster_file),
                 backdrop_file = COALESCE(?, backdrop_file)
             WHERE id = ?",
        )
        .bind(meta.tmdb_id)
        .bind(&meta.title)
        .bind(meta.title.as_deref().map(sort_title))
        .bind(meta.year)
        .bind(&meta.overview)
        .bind(&meta.poster_file)
        .bind(&meta.backdrop_file)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Marks everything for a fresh metadata lookup on the next scan.
    pub async fn reset_metadata(&self) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE shows SET metadata_fetched = 0")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE media_items SET metadata_fetched = 0")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // ----- Browsing ----------------------------------------------------------

    /// A page of movies plus the total count.
    pub async fn list_movies(
        &self,
        library: Option<LibraryId>,
        sort: MovieSort,
        limit: i64,
        offset: i64,
    ) -> DbResult<(Vec<MediaItem>, i64)> {
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM media_items m
             WHERE m.kind = 'movie' AND (? IS NULL OR m.library_id = ?)
             ORDER BY {} LIMIT ? OFFSET ?",
            sort.order_by()
        );
        let items = sqlx::query_as::<_, MediaItem>(&sql)
            .bind(library)
            .bind(library)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM media_items WHERE kind = 'movie' AND (? IS NULL OR library_id = ?)",
        )
        .bind(library)
        .bind(library)
        .fetch_one(&self.pool)
        .await?;
        Ok((items, total))
    }

    /// A page of shows (alphabetical) plus the total count.
    pub async fn list_shows(
        &self,
        library: Option<LibraryId>,
        limit: i64,
        offset: i64,
    ) -> DbResult<(Vec<Show>, i64)> {
        let sql = format!(
            "SELECT {SHOW_COLUMNS} FROM shows s WHERE (? IS NULL OR s.library_id = ?)
             ORDER BY s.sort_title LIMIT ? OFFSET ?"
        );
        let shows = sqlx::query_as::<_, Show>(&sql)
            .bind(library)
            .bind(library)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM shows WHERE (? IS NULL OR library_id = ?)",
        )
        .bind(library)
        .bind(library)
        .fetch_one(&self.pool)
        .await?;
        Ok((shows, total))
    }

    /// One media item.
    pub async fn get_item(&self, id: MediaId) -> DbResult<Option<MediaItem>> {
        let sql = format!("SELECT {MEDIA_COLUMNS} FROM media_items m WHERE m.id = ?");
        Ok(sqlx::query_as::<_, MediaItem>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// One show.
    pub async fn get_show(&self, id: ShowId) -> DbResult<Option<Show>> {
        let sql = format!("SELECT {SHOW_COLUMNS} FROM shows s WHERE s.id = ?");
        Ok(sqlx::query_as::<_, Show>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// All episodes of a show in season/episode order.
    pub async fn show_episodes(&self, show_id: ShowId) -> DbResult<Vec<MediaItem>> {
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM media_items m WHERE m.show_id = ?
             ORDER BY m.season, m.episode, m.sort_title"
        );
        Ok(sqlx::query_as::<_, MediaItem>(&sql)
            .bind(show_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// External subtitles for an item.
    pub async fn subtitles_for(&self, media_id: MediaId) -> DbResult<Vec<Subtitle>> {
        Ok(sqlx::query_as::<_, Subtitle>(
            "SELECT id, media_id, path, language, format FROM subtitles WHERE media_id = ? ORDER BY language, path",
        )
        .bind(media_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// One subtitle file.
    pub async fn get_subtitle(&self, id: SubtitleId) -> DbResult<Option<Subtitle>> {
        Ok(sqlx::query_as::<_, Subtitle>(
            "SELECT id, media_id, path, language, format FROM subtitles WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Case-insensitive title search over movies and shows.
    pub async fn search(&self, query: &str, limit: i64) -> DbResult<(Vec<MediaItem>, Vec<Show>)> {
        let pattern = like_pattern(query.trim());
        let movie_sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM media_items m
             WHERE m.kind = 'movie' AND m.title LIKE ? ESCAPE '\\' ORDER BY m.sort_title LIMIT ?"
        );
        let movies = sqlx::query_as::<_, MediaItem>(&movie_sql)
            .bind(&pattern)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        let show_sql = format!(
            "SELECT {SHOW_COLUMNS} FROM shows s WHERE s.title LIKE ? ESCAPE '\\' ORDER BY s.sort_title LIMIT ?"
        );
        let shows = sqlx::query_as::<_, Show>(&show_sql)
            .bind(&pattern)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok((movies, shows))
    }

    /// Most recently added items of a kind.
    pub async fn recently_added(&self, kind: MediaKind, limit: i64) -> DbResult<Vec<MediaItem>> {
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM media_items m WHERE m.kind = ?
             ORDER BY m.added_at DESC, m.id DESC LIMIT ?"
        );
        Ok(sqlx::query_as::<_, MediaItem>(&sql)
            .bind(kind)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use couchside_core::LibraryKind;

    fn movie(library_id: LibraryId, path: &str, title: &str, year: Option<i64>) -> NewMedia {
        NewMedia {
            library_id,
            kind: MediaKind::Movie,
            path: path.to_string(),
            file_size: 100,
            file_mtime: 1,
            parsed_title: title.to_string(),
            parsed_year: year,
            show_id: None,
            season: None,
            episode: None,
            episode_end: None,
            hint_tmdb_id: None,
            hint_imdb_id: None,
        }
    }

    #[tokio::test]
    async fn upsert_keeps_metadata_until_rename() {
        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, "/m")
            .await
            .expect("library");

        let id = db
            .upsert_media(&movie(lib.id, "/m/a.mkv", "Alien", Some(1979)))
            .await
            .expect("insert");
        db.set_item_metadata(
            id,
            &ItemMetadata {
                title: Some("Alien (Director's Cut)".into()),
                ..Default::default()
            },
        )
        .await
        .expect("meta");

        // Same parsed title: metadata title kept, same id returned.
        let again = db
            .upsert_media(&movie(lib.id, "/m/a.mkv", "Alien", Some(1979)))
            .await
            .expect("update");
        assert_eq!(again, id);
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(item.title, "Alien (Director's Cut)");
        assert!(db.movies_needing_metadata(10).await.expect("q").is_empty());

        // Parsed title changed: metadata reset.
        db.upsert_media(&movie(lib.id, "/m/a.mkv", "Aliens", Some(1986)))
            .await
            .expect("rename");
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(item.title, "Aliens");
        assert_eq!(db.movies_needing_metadata(10).await.expect("q").len(), 1);
    }

    #[tokio::test]
    async fn list_search_and_delete() {
        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, "/m")
            .await
            .expect("library");
        for (p, t) in [
            ("/m/1.mkv", "The Matrix"),
            ("/m/2.mkv", "Heat"),
            ("/m/3.mkv", "100% Wolf"),
        ] {
            db.upsert_media(&movie(lib.id, p, t, None))
                .await
                .expect("insert");
        }
        let (items, total) = db
            .list_movies(None, MovieSort::Title, 10, 0)
            .await
            .expect("list");
        assert_eq!(total, 3);
        let titles: Vec<_> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["100% Wolf", "Heat", "The Matrix"]);

        let (found, _) = db.search("matrix", 10).await.expect("search");
        assert_eq!(found.len(), 1);
        let (found, _) = db.search("%", 10).await.expect("search");
        assert_eq!(found.len(), 1, "% is matched literally");

        assert!(db.delete_library(lib.id).await.expect("delete"));
        let (_, total) = db
            .list_movies(None, MovieSort::Title, 10, 0)
            .await
            .expect("list");
        assert_eq!(total, 0, "items cascade with their library");
    }

    #[tokio::test]
    async fn duplicate_library_path_is_conflict() {
        let db = Db::open_in_memory().await.expect("db");
        db.create_library("A", LibraryKind::Movies, r"D:\Movies")
            .await
            .expect("first");
        let err = db
            .create_library("B", LibraryKind::Movies, r"d:\movies")
            .await
            .expect_err("duplicate");
        assert!(matches!(err, crate::DbError::Conflict(_)));
    }
}

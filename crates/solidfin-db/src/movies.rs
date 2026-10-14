//! Movie metadata queries (TMDB matching and details).

use crate::{Db, DbResult, MovieLookup};
use solidfin_core::naming::sort_title;
use solidfin_core::MediaId;

const LOOKUP_COLUMNS: &str =
    "id, parsed_title, parsed_year, hint_tmdb_id, hint_imdb_id, tmdb_id, match_locked";

/// Full TMDB details for one movie, ready to store.
#[derive(Debug, Clone, Default)]
pub struct MovieMetadata {
    /// TMDB id.
    pub tmdb_id: i64,
    /// Localized title.
    pub title: String,
    /// Original-language title.
    pub original_title: Option<String>,
    /// Release year.
    pub year: Option<i64>,
    /// Release date, `YYYY-MM-DD`.
    pub release_date: Option<String>,
    /// Plot summary.
    pub overview: Option<String>,
    /// Tagline.
    pub tagline: Option<String>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Average rating out of 10.
    pub rating: Option<f64>,
    /// Runtime in minutes according to TMDB.
    pub runtime_minutes: Option<i64>,
    /// Age rating.
    pub certification: Option<String>,
    /// IMDb id.
    pub imdb_id: Option<String>,
    /// Top cast.
    pub cast: Vec<String>,
    /// Directors.
    pub directors: Vec<String>,
    /// Cached poster file name (`None` = keep/clear, see [`Db::set_movie_metadata`]).
    pub poster_file: Option<String>,
    /// Cached backdrop file name.
    pub backdrop_file: Option<String>,
}

/// A movie file's new location and the name parsed from it, after "Clean up" renamed it.
#[derive(Debug, Clone, Default)]
pub struct RenamedFile {
    /// New file path.
    pub path: String,
    /// Title parsed from the new name.
    pub parsed_title: String,
    /// Year parsed from the new name.
    pub parsed_year: Option<i64>,
    /// TMDB id tagged in the new name or folder.
    pub hint_tmdb_id: Option<i64>,
    /// IMDb id tagged in the new name or folder.
    pub hint_imdb_id: Option<String>,
}

fn json_list(values: &[String]) -> Option<String> {
    if values.is_empty() {
        None
    } else {
        serde_json::to_string(values).ok()
    }
}

impl Db {
    /// Movies whose online metadata has not been fetched yet.
    pub async fn movies_needing_metadata(&self, limit: i64) -> DbResult<Vec<MovieLookup>> {
        let sql = format!(
            "SELECT {LOOKUP_COLUMNS} FROM media_items
             WHERE kind = 'movie' AND metadata_fetched = 0 ORDER BY added_at DESC LIMIT ?"
        );
        Ok(sqlx::query_as::<_, MovieLookup>(&sql)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Matching information for one movie (`None` if it isn't a movie).
    pub async fn movie_lookup(&self, id: MediaId) -> DbResult<Option<MovieLookup>> {
        let sql =
            format!("SELECT {LOOKUP_COLUMNS} FROM media_items WHERE id = ? AND kind = 'movie'");
        Ok(sqlx::query_as::<_, MovieLookup>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Stores full TMDB details and marks the movie as fetched.
    ///
    /// `locked` keeps this match on future refreshes (manual "Identify").
    /// When `same_match` is true (re-fetching the movie it already had), images
    /// that failed to download keep their previous file; otherwise they are cleared
    /// so a poster from a wrong match never lingers.
    pub async fn set_movie_metadata(
        &self,
        id: MediaId,
        meta: &MovieMetadata,
        locked: bool,
        same_match: bool,
    ) -> DbResult<()> {
        sqlx::query(
            "UPDATE media_items SET
                 metadata_fetched = 1,
                 match_locked = ?,
                 tmdb_id = ?,
                 title = ?,
                 sort_title = ?,
                 year = COALESCE(?, year),
                 original_title = ?,
                 release_date = ?,
                 overview = ?,
                 tagline = ?,
                 genres = ?,
                 rating = ?,
                 tmdb_runtime_minutes = ?,
                 certification = ?,
                 imdb_id = ?,
                 cast_names = ?,
                 directors = ?,
                 poster_file = CASE WHEN ? THEN COALESCE(?, poster_file) ELSE ? END,
                 backdrop_file = CASE WHEN ? THEN COALESCE(?, backdrop_file) ELSE ? END
             WHERE id = ?",
        )
        .bind(locked)
        .bind(meta.tmdb_id)
        .bind(&meta.title)
        .bind(sort_title(&meta.title))
        .bind(meta.year)
        .bind(&meta.original_title)
        .bind(&meta.release_date)
        .bind(&meta.overview)
        .bind(&meta.tagline)
        .bind(json_list(&meta.genres))
        .bind(meta.rating)
        .bind(meta.runtime_minutes)
        .bind(&meta.certification)
        .bind(&meta.imdb_id)
        .bind(json_list(&meta.cast))
        .bind(json_list(&meta.directors))
        .bind(same_match)
        .bind(&meta.poster_file)
        .bind(&meta.poster_file)
        .bind(same_match)
        .bind(&meta.backdrop_file)
        .bind(&meta.backdrop_file)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// "Clean up": removes every stored detail and the artwork of a movie, keeping only
    /// what comes from the file itself and its TMDB match (so it can be fetched again).
    /// Returns the artwork file names that were in use.
    pub async fn clear_movie_metadata(&self, id: MediaId) -> DbResult<Vec<String>> {
        let Some((parsed_title, poster, backdrop)) =
            sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                "SELECT parsed_title, poster_file, backdrop_file FROM media_items
                 WHERE id = ? AND kind = 'movie'",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(Vec::new());
        };
        sqlx::query(
            "UPDATE media_items SET
                 metadata_fetched = 0,
                 title = parsed_title,
                 sort_title = ?,
                 year = parsed_year,
                 original_title = NULL,
                 release_date = NULL,
                 overview = NULL,
                 tagline = NULL,
                 genres = NULL,
                 rating = NULL,
                 tmdb_runtime_minutes = NULL,
                 certification = NULL,
                 imdb_id = NULL,
                 cast_names = NULL,
                 directors = NULL,
                 poster_file = NULL,
                 backdrop_file = NULL
             WHERE id = ?",
        )
        .bind(sort_title(&parsed_title))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(poster.into_iter().chain(backdrop).collect())
    }

    /// Records that "Clean up" renamed a movie file. The new path and the name parsed from
    /// it are stored together so the next scan sees the same item, keeping its details,
    /// match and watch progress. `subtitles` maps old subtitle paths to new ones.
    pub async fn rename_media_file(
        &self,
        id: MediaId,
        renamed: &RenamedFile,
        subtitles: &[(String, String)],
    ) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE media_items SET path = ?, parsed_title = ?, parsed_year = ?,
                 hint_tmdb_id = ?, hint_imdb_id = ?
             WHERE id = ?",
        )
        .bind(&renamed.path)
        .bind(&renamed.parsed_title)
        .bind(renamed.parsed_year)
        .bind(renamed.hint_tmdb_id)
        .bind(&renamed.hint_imdb_id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| crate::conflict(e, "Another item already uses that file name."))?;
        for (old, new) in subtitles {
            sqlx::query("UPDATE subtitles SET path = ? WHERE media_id = ? AND path = ?")
                .bind(new)
                .bind(id)
                .bind(old)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Records that a lookup found nothing, so it isn't retried on every scan.
    /// Existing details are kept. "Refresh all metadata" tries again.
    pub async fn mark_movie_unmatched(&self, id: MediaId) -> DbResult<()> {
        sqlx::query("UPDATE media_items SET metadata_fetched = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Db, NewMedia};
    use solidfin_core::{LibraryKind, MediaKind};

    async fn db_with_movie(name: &str) -> (Db, solidfin_core::MediaId) {
        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, "/m")
            .await
            .expect("library");
        let id = db
            .upsert_media(&NewMedia {
                library_id: lib.id,
                kind: MediaKind::Movie,
                path: format!("/m/{name}.mkv"),
                file_size: 10,
                file_mtime: 1,
                parsed_title: name.to_string(),
                parsed_year: Some(1999),
                show_id: None,
                season: None,
                episode: None,
                episode_end: None,
                hint_tmdb_id: Some(603),
                hint_imdb_id: None,
            })
            .await
            .expect("insert");
        (db, id)
    }

    #[tokio::test]
    async fn stores_and_reads_details() {
        let (db, id) = db_with_movie("The Matrix").await;
        let lookups = db.movies_needing_metadata(10).await.expect("lookups");
        assert_eq!(lookups.len(), 1);
        assert_eq!(lookups[0].hint_tmdb_id, Some(603));
        assert!(!lookups[0].match_locked);

        let meta = super::MovieMetadata {
            tmdb_id: 603,
            title: "The Matrix".into(),
            year: Some(1999),
            genres: vec!["Action".into(), "Science Fiction".into()],
            rating: Some(8.2),
            certification: Some("M".into()),
            cast: vec!["Keanu Reeves".into()],
            directors: vec!["Lana Wachowski".into()],
            poster_file: Some("movie-1-poster.jpg".into()),
            ..Default::default()
        };
        db.set_movie_metadata(id, &meta, true, false)
            .await
            .expect("store");

        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(item.tmdb_id, Some(603));
        assert_eq!(
            item.genres.as_deref(),
            Some(r#"["Action","Science Fiction"]"#)
        );
        assert_eq!(item.certification.as_deref(), Some("M"));
        assert!(item.match_locked);
        assert!(db.movies_needing_metadata(10).await.expect("q").is_empty());

        // Re-fetch of the same match where the poster download failed keeps the old poster.
        let refetch = super::MovieMetadata {
            poster_file: None,
            ..meta.clone()
        };
        db.set_movie_metadata(id, &refetch, true, true)
            .await
            .expect("store");
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(item.poster_file.as_deref(), Some("movie-1-poster.jpg"));

        // A different match without a poster clears it.
        let other = super::MovieMetadata {
            tmdb_id: 604,
            title: "The Matrix Reloaded".into(),
            poster_file: None,
            ..Default::default()
        };
        db.set_movie_metadata(id, &other, true, false)
            .await
            .expect("store");
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(item.poster_file, None);
        assert_eq!(item.title, "The Matrix Reloaded");
    }

    #[tokio::test]
    async fn rescanning_unchanged_file_keeps_lock_and_probe() {
        let (db, id) = db_with_movie("Heat").await;
        db.set_probe_result(id, None).await.expect("probe");
        db.set_movie_metadata(
            id,
            &super::MovieMetadata {
                tmdb_id: 949,
                title: "Heat".into(),
                ..Default::default()
            },
            true,
            false,
        )
        .await
        .expect("store");

        let state = db
            .file_state("/m/Heat.mkv")
            .await
            .expect("q")
            .expect("exists");
        assert!(!state.needs_reparse);
        // Same file, same parsed name: lock, metadata and probe state survive.
        db.upsert_media(&NewMedia {
            library_id: db.list_libraries().await.expect("libs")[0].id,
            kind: MediaKind::Movie,
            path: "/m/Heat.mkv".into(),
            file_size: 10,
            file_mtime: 1,
            parsed_title: "Heat".into(),
            parsed_year: Some(1999),
            show_id: None,
            season: None,
            episode: None,
            episode_end: None,
            hint_tmdb_id: Some(603),
            hint_imdb_id: None,
        })
        .await
        .expect("upsert");
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert!(item.match_locked);
        assert!(db.items_needing_probe(10).await.expect("q").is_empty());
    }

    #[tokio::test]
    async fn clean_up_clears_details_and_follows_renames() {
        let (db, id) = db_with_movie("The Matrix").await;
        let meta = super::MovieMetadata {
            tmdb_id: 603,
            title: "The Matrix".into(),
            year: Some(1999),
            overview: Some("Neo".into()),
            genres: vec!["Action".into()],
            poster_file: Some("movie-1-603-poster.jpg".into()),
            backdrop_file: Some("movie-1-603-backdrop.jpg".into()),
            ..Default::default()
        };
        db.set_movie_metadata(id, &meta, true, false)
            .await
            .expect("store");
        let user = db.create_user("admin", "hash", true).await.expect("user");
        db.set_progress(user.id, id, 120.0, false)
            .await
            .expect("progress");
        db.replace_subtitles(
            id,
            &[crate::NewSubtitle {
                path: "/m/The Matrix.en.srt".into(),
                language: Some("en".into()),
                format: "srt".into(),
            }],
        )
        .await
        .expect("subs");

        let images = db.clear_movie_metadata(id).await.expect("clear");
        assert_eq!(
            images,
            vec![
                "movie-1-603-poster.jpg".to_string(),
                "movie-1-603-backdrop.jpg".to_string()
            ]
        );
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(item.overview, None);
        assert_eq!(item.genres, None);
        assert_eq!(item.poster_file, None);
        assert_eq!(item.tmdb_id, Some(603), "the identification is kept");
        assert!(item.match_locked);
        assert_eq!(db.movies_needing_metadata(10).await.expect("q").len(), 1);

        db.rename_media_file(
            id,
            &super::RenamedFile {
                path: "/m/The Matrix (1999)/The Matrix (1999) 1080p.mkv".into(),
                parsed_title: "The Matrix".into(),
                parsed_year: Some(1999),
                ..Default::default()
            },
            &[(
                "/m/The Matrix.en.srt".into(),
                "/m/The Matrix (1999)/The Matrix (1999) 1080p.en.srt".into(),
            )],
        )
        .await
        .expect("rename");
        let item = db.get_item(id).await.expect("get").expect("exists");
        assert_eq!(
            item.path,
            "/m/The Matrix (1999)/The Matrix (1999) 1080p.mkv"
        );
        assert_eq!(
            db.subtitle_paths(id).await.expect("subs"),
            vec!["/m/The Matrix (1999)/The Matrix (1999) 1080p.en.srt".to_string()]
        );
        let progress = db
            .get_progress(user.id, id)
            .await
            .expect("q")
            .expect("progress kept");
        assert!((progress.position_seconds - 120.0).abs() < f64::EPSILON);
        assert!(db
            .file_state("/m/The Matrix (1999)/The Matrix (1999) 1080p.mkv")
            .await
            .expect("q")
            .is_some());
    }

    #[tokio::test]
    async fn unmatched_is_not_retried() {
        let (db, id) = db_with_movie("Home Video").await;
        db.mark_movie_unmatched(id).await.expect("mark");
        assert!(db.movies_needing_metadata(10).await.expect("q").is_empty());
        db.reset_metadata().await.expect("reset");
        assert_eq!(db.movies_needing_metadata(10).await.expect("q").len(), 1);
    }
}

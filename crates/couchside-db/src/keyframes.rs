//! Cached keyframe times, used to cut streams into HLS segments.

use crate::{Db, DbResult};
use couchside_core::MediaId;

impl Db {
    /// Keyframe times (milliseconds from the start of the video) cached for an item, if
    /// they were read from the file as it is now (same size and modified time).
    pub async fn keyframes(&self, media_id: MediaId) -> DbResult<Option<Vec<i64>>> {
        let json = sqlx::query_scalar::<_, String>(
            "SELECT k.times_ms FROM keyframes k JOIN media_items m ON m.id = k.media_id
             WHERE k.media_id = ? AND k.file_size = m.file_size AND k.file_mtime = m.file_mtime",
        )
        .bind(media_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Caches an item's keyframe times against its current size and modified time.
    pub async fn set_keyframes(&self, media_id: MediaId, times_ms: &[i64]) -> DbResult<()> {
        let json = serde_json::to_string(times_ms).unwrap_or_else(|_| "[]".to_string());
        sqlx::query(
            "INSERT OR REPLACE INTO keyframes (media_id, file_size, file_mtime, times_ms)
             SELECT id, file_size, file_mtime, ? FROM media_items WHERE id = ?",
        )
        .bind(json)
        .bind(media_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Db, NewMedia};
    use couchside_core::{LibraryKind, MediaKind};

    fn movie(library_id: couchside_core::LibraryId, file_mtime: i64) -> NewMedia {
        NewMedia {
            library_id,
            kind: MediaKind::Movie,
            path: "/m/a.mkv".into(),
            file_size: 100,
            file_mtime,
            parsed_title: "Alien".into(),
            parsed_year: Some(1979),
            show_id: None,
            season: None,
            episode: None,
            episode_end: None,
            hint_tmdb_id: None,
            hint_imdb_id: None,
        }
    }

    #[tokio::test]
    async fn keyframes_follow_the_file() {
        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, "/m")
            .await
            .expect("library");
        let id = db.upsert_media(&movie(lib.id, 1)).await.expect("insert");
        assert_eq!(db.keyframes(id).await.expect("get"), None);

        db.set_keyframes(id, &[0, 2002, 4004]).await.expect("set");
        assert_eq!(
            db.keyframes(id).await.expect("get"),
            Some(vec![0, 2002, 4004])
        );

        // The file changed on disk: the cached times no longer apply.
        db.upsert_media(&movie(lib.id, 2)).await.expect("update");
        assert_eq!(db.keyframes(id).await.expect("get"), None);
    }
}

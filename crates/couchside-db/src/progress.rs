//! Watch progress queries.

use crate::media::MEDIA_COLUMNS;
use crate::{Db, DbResult, MediaItem, Progress};
use couchside_core::{now_unix, MediaId, UserId};
use std::collections::HashMap;

impl Db {
    /// Progress for one item.
    pub async fn get_progress(&self, user: UserId, media: MediaId) -> DbResult<Option<Progress>> {
        Ok(sqlx::query_as::<_, Progress>(
            "SELECT media_id, position_seconds, played, updated_at FROM watch_progress
             WHERE user_id = ? AND media_id = ?",
        )
        .bind(user)
        .bind(media)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Progress for many items at once, keyed by item id.
    pub async fn progress_map(
        &self,
        user: UserId,
        ids: &[MediaId],
    ) -> DbResult<HashMap<MediaId, Progress>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let raw: Vec<i64> = ids.iter().map(|id| id.0).collect();
        let json = serde_json::to_string(&raw).unwrap_or_else(|_| "[]".to_string());
        let rows = sqlx::query_as::<_, Progress>(
            "SELECT media_id, position_seconds, played, updated_at FROM watch_progress
             WHERE user_id = ? AND media_id IN (SELECT value FROM json_each(?))",
        )
        .bind(user)
        .bind(json)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|p| (p.media_id, p)).collect())
    }

    /// Saves a playback position and watched flag.
    pub async fn set_progress(
        &self,
        user: UserId,
        media: MediaId,
        position: f64,
        played: bool,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO watch_progress (user_id, media_id, position_seconds, played, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(user_id, media_id) DO UPDATE SET
                 position_seconds = excluded.position_seconds,
                 played = excluded.played,
                 updated_at = excluded.updated_at",
        )
        .bind(user)
        .bind(media)
        .bind(position.max(0.0))
        .bind(played)
        .bind(now_unix())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Partly watched items, most recent first.
    pub async fn continue_watching(&self, user: UserId, limit: i64) -> DbResult<Vec<MediaItem>> {
        let sql = format!(
            "SELECT {MEDIA_COLUMNS} FROM watch_progress p JOIN media_items m ON m.id = p.media_id
             WHERE p.user_id = ? AND p.played = 0 AND p.position_seconds > 30
             ORDER BY p.updated_at DESC LIMIT ?"
        );
        Ok(sqlx::query_as::<_, MediaItem>(&sql)
            .bind(user)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }
}

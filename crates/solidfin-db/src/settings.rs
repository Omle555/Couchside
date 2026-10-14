//! Key/value settings changed from the web UI (they override config.toml).

use crate::{Db, DbResult};

/// Setting key for the TMDB API key.
pub const TMDB_API_KEY: &str = "tmdb_api_key";
/// Setting key for the metadata language.
pub const METADATA_LANGUAGE: &str = "metadata_language";
/// Setting key for letting "Clean up" rename movie files (`true`/`false`).
pub const ALLOW_FILE_RENAMES: &str = "allow_file_renames";

impl Db {
    /// Reads a setting.
    pub async fn get_setting(&self, key: &str) -> DbResult<Option<String>> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
                .bind(key)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Writes a setting; `None` removes it.
    pub async fn set_setting(&self, key: &str, value: Option<&str>) -> DbResult<()> {
        match value {
            Some(v) => {
                sqlx::query(
                    "INSERT INTO settings (key, value) VALUES (?, ?)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                )
                .bind(key)
                .bind(v)
                .execute(&self.pool)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM settings WHERE key = ?")
                    .bind(key)
                    .execute(&self.pool)
                    .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip() {
        let db = Db::open_in_memory().await.expect("db");
        assert_eq!(db.get_setting(TMDB_API_KEY).await.expect("get"), None);
        db.set_setting(TMDB_API_KEY, Some("abc"))
            .await
            .expect("set");
        db.set_setting(TMDB_API_KEY, Some("def"))
            .await
            .expect("set");
        assert_eq!(
            db.get_setting(TMDB_API_KEY).await.expect("get").as_deref(),
            Some("def")
        );
        db.set_setting(TMDB_API_KEY, None).await.expect("clear");
        assert_eq!(db.get_setting(TMDB_API_KEY).await.expect("get"), None);
    }
}

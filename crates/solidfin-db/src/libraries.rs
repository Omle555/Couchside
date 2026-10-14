//! Library queries.

use crate::{conflict, Db, DbResult, Library};
use solidfin_core::{now_unix, LibraryId, LibraryKind};

impl Db {
    /// All libraries ordered by name.
    pub async fn list_libraries(&self) -> DbResult<Vec<Library>> {
        Ok(sqlx::query_as::<_, Library>(
            "SELECT id, name, kind, path, created_at FROM libraries ORDER BY name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// One library by id.
    pub async fn get_library(&self, id: LibraryId) -> DbResult<Option<Library>> {
        Ok(sqlx::query_as::<_, Library>(
            "SELECT id, name, kind, path, created_at FROM libraries WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Adds a library. Fails with `Conflict` if the path is already a library.
    pub async fn create_library(
        &self,
        name: &str,
        kind: LibraryKind,
        path: &str,
    ) -> DbResult<Library> {
        sqlx::query_as::<_, Library>(
            "INSERT INTO libraries (name, kind, path, created_at) VALUES (?, ?, ?, ?)
             RETURNING id, name, kind, path, created_at",
        )
        .bind(name)
        .bind(kind)
        .bind(path)
        .bind(now_unix())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| conflict(e, "that folder is already a library"))
    }

    /// Removes a library and everything scanned from it. Returns false if it did not exist.
    pub async fn delete_library(&self, id: LibraryId) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM libraries WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

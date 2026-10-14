//! User and session queries.

use crate::{conflict, Db, DbResult, User};
use solidfin_core::{now_unix, UserId};

const USER_COLUMNS: &str = "id, username, password_hash, is_admin, created_at";

impl Db {
    /// Number of user accounts (0 means first-run setup is needed).
    pub async fn count_users(&self) -> DbResult<i64> {
        Ok(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?)
    }

    /// Creates a user. Fails with `Conflict` if the name is taken.
    pub async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> DbResult<User> {
        let sql = format!(
            "INSERT INTO users (username, password_hash, is_admin, created_at) VALUES (?, ?, ?, ?)
             RETURNING {USER_COLUMNS}"
        );
        sqlx::query_as::<_, User>(&sql)
            .bind(username)
            .bind(password_hash)
            .bind(is_admin)
            .bind(now_unix())
            .fetch_one(&self.pool)
            .await
            .map_err(|e| conflict(e, "that username is already taken"))
    }

    /// Looks up a user by name (case-insensitive).
    pub async fn user_by_username(&self, username: &str) -> DbResult<Option<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE username = ?");
        Ok(sqlx::query_as::<_, User>(&sql)
            .bind(username)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// All users ordered by name.
    pub async fn list_users(&self) -> DbResult<Vec<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users ORDER BY username COLLATE NOCASE");
        Ok(sqlx::query_as::<_, User>(&sql)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Number of administrator accounts.
    pub async fn count_admins(&self) -> DbResult<i64> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE is_admin = 1")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Looks up a user by id.
    pub async fn get_user(&self, id: UserId) -> DbResult<Option<User>> {
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?");
        Ok(sqlx::query_as::<_, User>(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Deletes a user and their sessions/progress. Returns false if missing.
    pub async fn delete_user(&self, id: UserId) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Replaces a user's password hash and signs out their existing sessions.
    pub async fn set_password(&self, id: UserId, password_hash: &str) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(password_hash)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stores a new session (only the SHA-256 hash of the token is kept).
    pub async fn create_session(&self, token_hash: &str, user_id: UserId) -> DbResult<()> {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, last_seen) VALUES (?, ?, ?, ?)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns the user for a session token hash, refreshing `last_seen`.
    /// Sessions unused for `max_idle_secs` are rejected and deleted.
    pub async fn user_for_session(
        &self,
        token_hash: &str,
        max_idle_secs: i64,
    ) -> DbResult<Option<User>> {
        let now = now_unix();
        sqlx::query("DELETE FROM sessions WHERE last_seen < ?")
            .bind(now - max_idle_secs)
            .execute(&self.pool)
            .await?;
        let user = sqlx::query_as::<_, User>(
            "SELECT u.id, u.username, u.password_hash, u.is_admin, u.created_at
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        if user.is_some() {
            sqlx::query("UPDATE sessions SET last_seen = ? WHERE token_hash = ?")
                .bind(now)
                .bind(token_hash)
                .execute(&self.pool)
                .await?;
        }
        Ok(user)
    }

    /// Deletes a session (logout).
    pub async fn delete_session(&self, token_hash: &str) -> DbResult<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

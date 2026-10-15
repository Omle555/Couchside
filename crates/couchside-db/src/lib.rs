//! SQLite storage for Couchside.
//!
//! All queries are parameterized. Schema changes live in `migrations/` and are
//! applied automatically on startup, after a backup of the existing database.

mod activity;
mod keyframes;
mod libraries;
mod media;
mod models;
mod movies;
mod progress;
mod settings;
mod users;

pub use activity::{LibraryCounts, NowPlaying};
pub use media::{
    FileState, ItemMetadata, MovieSort, NewMedia, NewSubtitle, ProbeData, ShowMetadata,
};
pub use models::{Library, MediaItem, MediaStream, MovieLookup, Progress, Show, Subtitle, User};
pub use movies::{MovieMetadata, RenamedFile};
pub use settings::{ALLOW_FILE_RENAMES, METADATA_LANGUAGE, TMDB_API_KEY};

use couchside_core::now_unix;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use std::path::Path;
use std::time::Duration;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Errors returned by database operations.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Any SQLite / driver error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A migration failed to apply.
    #[error("database migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A unique constraint was violated (e.g. duplicate username or library path).
    #[error("{0}")]
    Conflict(String),
}

/// Result alias for database operations.
pub type DbResult<T> = Result<T, DbError>;

/// Maps unique-constraint violations to [`DbError::Conflict`] with a friendly message.
fn conflict(err: sqlx::Error, message: &str) -> DbError {
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.is_unique_violation() {
            return DbError::Conflict(message.to_string());
        }
    }
    DbError::Sqlx(err)
}

/// Handle to the Couchside database. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Opens (or creates) the database file, backs it up if migrations are pending,
    /// then applies migrations.
    pub async fn open(path: &Path) -> DbResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;
        let db = Self { pool };
        db.backup_before_migrations(path).await?;
        MIGRATOR.run(&db.pool).await?;
        Ok(db)
    }

    /// Opens a private in-memory database (used by tests).
    pub async fn open_in_memory() -> DbResult<Self> {
        let options = "sqlite::memory:"
            .parse::<SqliteConnectOptions>()?
            .foreign_keys(true);
        // A single connection so every query sees the same in-memory database.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        MIGRATOR.run(&pool).await?;
        Ok(Self { pool })
    }

    /// Copies the database with `VACUUM INTO` when an existing database has pending migrations.
    async fn backup_before_migrations(&self, db_path: &Path) -> DbResult<()> {
        let applied: Result<i64, sqlx::Error> =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(&self.pool)
                .await;
        let Ok(applied) = applied else {
            // Table missing: brand-new database, nothing to back up.
            return Ok(());
        };
        let total = i64::try_from(MIGRATOR.iter().count()).unwrap_or(i64::MAX);
        if applied >= total {
            return Ok(());
        }
        let backup = db_path.with_extension(format!("db.bak-{}", now_unix()));
        tracing::info!(backup = %backup.display(), "backing up database before migrations");
        sqlx::query("VACUUM INTO ?")
            .bind(backup.to_string_lossy().into_owned())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Closes all connections, flushing the WAL.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

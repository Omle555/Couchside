//! Library totals and what is playing now, for home-automation dashboards.

use crate::{Db, DbResult};
use couchside_core::{now_unix, MediaId, MediaKind};
use sqlx::FromRow;

/// How many movies, shows and episodes the libraries hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct LibraryCounts {
    /// Movies.
    pub movies: i64,
    /// TV shows.
    pub shows: i64,
    /// TV episodes.
    pub episodes: i64,
}

/// Something a user is watching right now.
#[derive(Debug, Clone, FromRow)]
pub struct NowPlaying {
    /// Who is watching.
    pub username: String,
    /// The item being played.
    pub media_id: MediaId,
    /// Movie or episode.
    pub kind: MediaKind,
    /// Movie or episode title.
    pub title: String,
    /// Show title, for episodes.
    pub show_title: Option<String>,
    /// Season number, for episodes.
    pub season: Option<i64>,
    /// Episode number, for episodes.
    pub episode: Option<i64>,
    /// Last saved position in seconds.
    pub position_seconds: f64,
    /// Length in seconds, when known.
    pub runtime_seconds: Option<f64>,
    /// When the position was last saved (Unix seconds).
    pub updated_at: i64,
}

impl Db {
    /// Totals across all libraries.
    pub async fn library_counts(&self) -> DbResult<LibraryCounts> {
        Ok(sqlx::query_as::<_, LibraryCounts>(
            "SELECT
                (SELECT COUNT(*) FROM media_items WHERE kind = 'movie') AS movies,
                (SELECT COUNT(*) FROM shows) AS shows,
                (SELECT COUNT(*) FROM media_items WHERE kind = 'episode') AS episodes",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// Items whose watch position was saved in the last `within_secs` seconds and that
    /// aren't finished, newest first. Players save every few seconds while playing.
    pub async fn now_playing(&self, within_secs: i64) -> DbResult<Vec<NowPlaying>> {
        Ok(sqlx::query_as::<_, NowPlaying>(
            "SELECT u.username, m.id AS media_id, m.kind, m.title, s.title AS show_title,
                    m.season, m.episode, p.position_seconds, m.runtime_seconds, p.updated_at
             FROM watch_progress p
             JOIN users u ON u.id = p.user_id
             JOIN media_items m ON m.id = p.media_id
             LEFT JOIN shows s ON s.id = m.show_id
             WHERE p.updated_at >= ? AND p.played = 0
             ORDER BY p.updated_at DESC",
        )
        .bind(now_unix() - within_secs)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Db, NewMedia};
    use couchside_core::{LibraryKind, MediaKind};

    #[tokio::test]
    async fn counts_and_now_playing() {
        let db = Db::open_in_memory().await.expect("db");
        let lib = db
            .create_library("Movies", LibraryKind::Movies, "/m")
            .await
            .expect("library");
        let movie = |path: &str, title: &str| NewMedia {
            library_id: lib.id,
            kind: MediaKind::Movie,
            path: path.to_string(),
            file_size: 100,
            file_mtime: 1,
            parsed_title: title.to_string(),
            parsed_year: None,
            show_id: None,
            season: None,
            episode: None,
            episode_end: None,
            hint_tmdb_id: None,
            hint_imdb_id: None,
        };
        let alien = db
            .upsert_media(&movie("/m/a.mkv", "Alien"))
            .await
            .expect("a");
        let heat = db
            .upsert_media(&movie("/m/h.mkv", "Heat"))
            .await
            .expect("h");
        let user = db.create_user("lj", "hash", true).await.expect("user");

        let counts = db.library_counts().await.expect("counts");
        assert_eq!((counts.movies, counts.shows, counts.episodes), (2, 0, 0));
        assert!(db.now_playing(30).await.expect("none").is_empty());

        db.set_progress(user.id, alien, 120.0, false)
            .await
            .expect("progress");
        db.set_progress(user.id, heat, 0.0, true)
            .await
            .expect("watched");
        let playing = db.now_playing(30).await.expect("playing");
        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].title, "Alien");
        assert_eq!(playing[0].username, "lj");
        assert_eq!(playing[0].position_seconds, 120.0);
    }
}

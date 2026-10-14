//! Background library work: scanning, probing and metadata fetching.
//! Only one run happens at a time; API requests never wait on it.

use serde::Serialize;
use solidfin_core::{now_unix, MediaId, ShowId};
use solidfin_db::{Db, DbError, ItemMetadata, Library, MediaItem, MovieLookup, MovieMetadata};
use solidfin_library::{
    files_in_use, scan_library, scan_target, Change, ChangeQueue, Ffprobe, FolderWatcher,
    ScanTarget, WatchEvent,
};
use solidfin_metadata::{EpisodeInfo, MetadataError, MovieDetails, Tmdb};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::{Notify, OwnedMutexGuard};

const BATCH: i64 = 50;
/// How often folder watching re-reads the library list (and retries offline folders).
const WATCH_REFRESH: Duration = Duration::from_secs(60);

/// What the background worker is doing, shown on the settings page.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanStatus {
    /// A run is in progress.
    pub running: bool,
    /// Human readable current step.
    pub phase: String,
    /// When the last run started (Unix seconds).
    pub last_started: Option<i64>,
    /// When the last run finished (Unix seconds).
    pub last_finished: Option<i64>,
    /// Problems from the last run, if any.
    pub errors: Vec<String>,
    /// Steps finished in the current run (one per library, then media info and metadata).
    pub step: u32,
    /// Steps in the current run; 0 until the libraries have been listed.
    pub steps: u32,
}

/// Errors from metadata work. In the background run they stop the metadata
/// phase early (it is retried on the next run).
#[derive(Debug, thiserror::Error)]
pub enum MetadataTaskError {
    /// A database error.
    #[error(transparent)]
    Db(#[from] DbError),
    /// A TMDB error (network, bad key, rate limit).
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    /// No TMDB key is configured.
    #[error("add a TMDB API key under Settings → Metadata first")]
    NoTmdb,
    /// The item is not a movie.
    #[error("only movies can be identified")]
    NotAMovie,
    /// TMDB doesn't know the chosen id.
    #[error("TMDB has no movie with that id")]
    UnknownTmdbId,
    /// "Clean up" needs to know which movie this is.
    #[error("this movie isn't matched to TMDB yet; use Identify first")]
    NotIdentified,
}

/// Coordinates scans, ffprobe and TMDB lookups.
pub struct LibraryManager {
    db: Db,
    ffprobe: Option<Ffprobe>,
    tmdb: RwLock<Option<Arc<Tmdb>>>,
    status: Mutex<ScanStatus>,
    run_lock: Arc<tokio::sync::Mutex<()>>,
    libraries_changed: Notify,
}

impl LibraryManager {
    /// Creates the manager. `ffprobe` and `tmdb` are optional features.
    pub fn new(db: Db, ffprobe: Option<Ffprobe>, tmdb: Option<Tmdb>) -> Arc<Self> {
        Arc::new(Self {
            db,
            ffprobe,
            tmdb: RwLock::new(tmdb.map(Arc::new)),
            status: Mutex::new(ScanStatus::default()),
            run_lock: Arc::new(tokio::sync::Mutex::new(())),
            libraries_changed: Notify::new(),
        })
    }

    /// Whether ffprobe was found.
    pub fn has_ffprobe(&self) -> bool {
        self.ffprobe.is_some()
    }

    /// Whether a TMDB key is configured.
    pub fn has_tmdb(&self) -> bool {
        self.tmdb().is_some()
    }

    /// The current TMDB client, if a key is configured.
    pub fn tmdb(&self) -> Option<Arc<Tmdb>> {
        self.tmdb.read().ok().and_then(|t| t.clone())
    }

    /// Replaces the TMDB client after the key or language changed in Settings.
    pub fn set_tmdb(&self, tmdb: Option<Tmdb>) {
        if let Ok(mut current) = self.tmdb.write() {
            *current = tmdb.map(Arc::new);
        }
    }

    /// A snapshot of the current status.
    pub fn status(&self) -> ScanStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn update_status(&self, f: impl FnOnce(&mut ScanStatus)) {
        if let Ok(mut status) = self.status.lock() {
            f(&mut status);
        }
    }

    fn record_error(&self, message: String) {
        tracing::warn!("{message}");
        self.update_status(|s| s.errors.push(message));
    }

    /// Blocks background runs while a "Clean up" changes an item. `None` if a run is going.
    pub fn try_exclusive(&self) -> Option<OwnedMutexGuard<()>> {
        Arc::clone(&self.run_lock).try_lock_owned().ok()
    }

    /// Reads a file's media info (length, codecs, resolution) again. Does nothing
    /// without ffprobe.
    pub async fn refresh_media_info(&self, id: MediaId, file: &Path) -> Result<(), DbError> {
        let Some(ffprobe) = &self.ffprobe else {
            return Ok(());
        };
        match ffprobe.probe(file).await {
            Ok(data) => self.db.set_probe_result(id, Some(&data)).await,
            Err(err) => {
                tracing::warn!(path = %file.display(), error = %err, "could not read media info");
                self.db.set_probe_result(id, None).await
            }
        }
    }

    /// Title and year of a movie's current TMDB match, fetched fresh from TMDB.
    pub async fn matched_title(
        &self,
        id: MediaId,
    ) -> Result<(String, Option<i64>), MetadataTaskError> {
        let (_, lookup, details) = self.current_match(id).await?;
        let title = if details.title.is_empty() {
            lookup.parsed_title
        } else {
            details.title
        };
        Ok((title, details.year))
    }

    async fn current_match(
        &self,
        id: MediaId,
    ) -> Result<(Arc<Tmdb>, MovieLookup, MovieDetails), MetadataTaskError> {
        let tmdb = self.tmdb().ok_or(MetadataTaskError::NoTmdb)?;
        let lookup = self
            .db
            .movie_lookup(id)
            .await?
            .ok_or(MetadataTaskError::NotAMovie)?;
        let tmdb_id = lookup.tmdb_id.ok_or(MetadataTaskError::NotIdentified)?;
        let details = tmdb
            .movie_details(tmdb_id)
            .await?
            .ok_or(MetadataTaskError::UnknownTmdbId)?;
        Ok((tmdb, lookup, details))
    }

    /// "Clean up": deletes all of a movie's stored details and artwork and builds them again
    /// from its current TMDB match, which is then kept on future refreshes. TMDB is asked
    /// first, so nothing is deleted if it can't be reached. Returns the artwork files that
    /// were in use before (the caller removes the ones no longer used).
    pub async fn rebuild_movie(&self, id: MediaId) -> Result<Vec<String>, MetadataTaskError> {
        let (tmdb, lookup, details) = self.current_match(id).await?;
        let old_images = self.db.clear_movie_metadata(id).await?;
        self.store_movie(&tmdb, &lookup, details, true).await?;
        tracing::info!(item = %id, "movie details rebuilt by Clean up");
        Ok(old_images)
    }

    /// Starts a full run in the background. Returns false if one is already running.
    pub fn trigger(self: &Arc<Self>) -> bool {
        let Ok(guard) = Arc::clone(&self.run_lock).try_lock_owned() else {
            return false;
        };
        let this = Arc::clone(self);
        tokio::spawn(async move { this.run(guard).await });
        true
    }

    /// Runs a scan now and then every `interval_minutes` (0 = never).
    pub fn spawn_schedule(self: &Arc<Self>, interval_minutes: u64, scan_on_startup: bool) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            if scan_on_startup {
                this.trigger();
            }
            if interval_minutes == 0 {
                return;
            }
            let period = Duration::from_secs(interval_minutes.saturating_mul(60));
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                this.trigger();
            }
        });
    }

    /// Re-reads the library list for folder watching (after a library was added or removed).
    pub fn libraries_changed(&self) {
        self.libraries_changed.notify_one();
    }

    /// Watches the library folders and, once a changed folder has been quiet for `delay`,
    /// rescans just that folder.
    pub fn spawn_watcher(self: &Arc<Self>, delay: Duration) {
        let this = Arc::clone(self);
        tokio::spawn(async move { this.watch(delay).await });
    }

    async fn watch(self: Arc<Self>, delay: Duration) {
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = match FolderWatcher::new(events) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::warn!(error = %err, "folder watching is off; libraries update on scans only");
                return;
            }
        };
        let mut libraries = Vec::new();
        let mut queue = ChangeQueue::new(delay);
        let mut refresh = tokio::time::interval(WATCH_REFRESH);
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let due = queue.next_due().map(tokio::time::Instant::from_std);
            tokio::select! {
                _ = refresh.tick() => {}
                () = self.libraries_changed.notified() => {}
                event = received.recv() => {
                    let Some(event) = event else { return };
                    record_event(&mut queue, &libraries, event);
                    continue;
                }
                () = sleep_until(due) => {
                    self.start_changes(&mut queue, &libraries).await;
                    continue;
                }
            }
            let Some(updated) = self.watch_libraries(watcher).await else {
                return;
            };
            (watcher, libraries) = updated;
        }
    }

    /// Points the watcher at the current library folders.
    async fn watch_libraries(
        &self,
        mut watcher: FolderWatcher,
    ) -> Option<(FolderWatcher, Vec<Library>)> {
        let libraries = match self.db.list_libraries().await {
            Ok(libraries) => libraries,
            Err(err) => {
                tracing::warn!(error = %err, "could not list libraries to watch");
                Vec::new()
            }
        };
        let roots: Vec<PathBuf> = libraries.iter().map(|l| PathBuf::from(&l.path)).collect();
        match tokio::task::spawn_blocking(move || {
            watcher.watch_only(&roots);
            watcher
        })
        .await
        {
            Ok(watcher) => Some((watcher, libraries)),
            Err(err) => {
                tracing::warn!(error = %err, "folder watching stopped");
                None
            }
        }
    }

    /// Rescans the changes that are ready. Folders with a video still being copied, and
    /// everything while another run is going, wait for another quiet period.
    async fn start_changes(self: &Arc<Self>, queue: &mut ChangeQueue, libraries: &[Library]) {
        let now = std::time::Instant::now();
        let mut work = Vec::new();
        for change in queue.take_ready(now) {
            let Some(library) = libraries.iter().find(|l| l.id == change.library) else {
                continue;
            };
            if let Some(target) = &change.target {
                let (root, check) = (PathBuf::from(&library.path), target.clone());
                let busy = tokio::task::spawn_blocking(move || files_in_use(&root, &check))
                    .await
                    .unwrap_or(false);
                if busy {
                    tracing::debug!(library = %library.name, ?target, "files still being copied; waiting");
                    queue.record(change, now);
                    continue;
                }
            }
            work.push((library.clone(), change));
        }
        if work.is_empty() {
            return;
        }
        let Ok(guard) = Arc::clone(&self.run_lock).try_lock_owned() else {
            for (_, change) in work {
                queue.record(change, now);
            }
            return;
        };
        let this = Arc::clone(self);
        tokio::spawn(async move { this.run_changes(guard, work).await });
    }

    /// Rescans the folders where files changed, then reads media info and metadata for
    /// anything new.
    async fn run_changes(&self, _guard: OwnedMutexGuard<()>, work: Vec<(Library, Change)>) {
        self.begin_run("Updating libraries");
        let steps = u32::try_from(work.len())
            .unwrap_or(u32::MAX)
            .saturating_add(self.later_steps());
        self.update_status(|s| s.steps = steps);
        for (library, change) in work {
            tracing::info!(library = %library.name, target = ?change.target, "files changed on disk; rescanning");
            self.update_status(|s| s.phase = format!("Updating {}", library.name));
            let result = match &change.target {
                Some(target) => scan_target(&self.db, &library, target).await,
                None => scan_library(&self.db, &library).await,
            };
            if let Err(err) = result {
                self.record_error(format!("{}: {err}", library.name));
            }
            self.update_status(|s| s.step += 1);
        }
        self.finish_run().await;
    }

    async fn run(&self, _guard: OwnedMutexGuard<()>) {
        self.begin_run("Scanning libraries");
        self.scan_all().await;
        self.finish_run().await;
    }

    fn begin_run(&self, phase: &str) {
        self.update_status(|s| {
            s.running = true;
            s.phase = phase.into();
            s.last_started = Some(now_unix());
            s.errors.clear();
            s.step = 0;
            s.steps = 0;
        });
    }

    /// Steps after scanning: media info and metadata, when available.
    fn later_steps(&self) -> u32 {
        u32::from(self.ffprobe.is_some()) + u32::from(self.tmdb().is_some())
    }

    /// Reads media info and metadata for new items, then goes idle.
    async fn finish_run(&self) {
        if self.ffprobe.is_some() {
            self.update_status(|s| s.phase = "Reading media info".into());
            if let Err(err) = self.probe_pending().await {
                self.record_error(format!("media info: {err}"));
            }
            self.update_status(|s| s.step += 1);
        }

        if let Some(tmdb) = self.tmdb() {
            self.update_status(|s| s.phase = "Fetching metadata".into());
            if let Err(err) = self.fetch_metadata(&tmdb).await {
                self.record_error(format!("metadata: {err}"));
            }
            self.update_status(|s| s.step += 1);
        }

        self.update_status(|s| {
            s.running = false;
            s.phase = "Idle".into();
            s.last_finished = Some(now_unix());
        });
    }

    async fn scan_all(&self) {
        let libraries = match self.db.list_libraries().await {
            Ok(libs) => libs,
            Err(err) => {
                self.record_error(format!("could not list libraries: {err}"));
                return;
            }
        };
        let steps = u32::try_from(libraries.len())
            .unwrap_or(u32::MAX)
            .saturating_add(self.later_steps());
        self.update_status(|s| s.steps = steps);
        for library in libraries {
            self.update_status(|s| s.phase = format!("Scanning {}", library.name));
            if let Err(err) = scan_library(&self.db, &library).await {
                self.record_error(format!("{}: {err}", library.name));
            }
            self.update_status(|s| s.step += 1);
        }
    }

    async fn probe_pending(&self) -> Result<(), DbError> {
        let Some(ffprobe) = &self.ffprobe else {
            return Ok(());
        };
        loop {
            let batch = self.db.items_needing_probe(BATCH).await?;
            if batch.is_empty() {
                return Ok(());
            }
            for (id, path) in batch {
                match ffprobe.probe(Path::new(&path)).await {
                    Ok(data) => self.db.set_probe_result(id, Some(&data)).await?,
                    Err(err) => {
                        tracing::warn!(path = %path, error = %err, "could not read media info");
                        self.db.set_probe_result(id, None).await?;
                    }
                }
            }
        }
    }

    /// Downloads an image, logging (not failing) on error.
    async fn image(
        &self,
        tmdb: &Tmdb,
        tmdb_path: Option<&str>,
        size: &str,
        stem: String,
    ) -> Option<String> {
        let path = tmdb_path?;
        match tmdb.download_image(path, size, &stem).await {
            Ok(file) => Some(file),
            Err(err) => {
                tracing::warn!(image = path, error = %err, "could not download image");
                None
            }
        }
    }

    async fn fetch_metadata(&self, tmdb: &Tmdb) -> Result<(), MetadataTaskError> {
        self.fetch_movies(tmdb).await?;
        self.fetch_shows(tmdb).await?;
        self.fetch_episodes(tmdb).await
    }

    async fn fetch_movies(&self, tmdb: &Tmdb) -> Result<(), MetadataTaskError> {
        loop {
            let batch = self.db.movies_needing_metadata(BATCH).await?;
            if batch.is_empty() {
                return Ok(());
            }
            for lookup in batch {
                if !self.match_movie(tmdb, &lookup).await? {
                    tracing::info!(title = %lookup.parsed_title, year = ?lookup.parsed_year, "no confident TMDB match for movie");
                    self.db.mark_movie_unmatched(lookup.id).await?;
                }
            }
        }
    }

    /// Finds and stores the TMDB match for one movie. In order: a match chosen
    /// by hand, an id in the file name (`{tmdb-603}`, `{imdb-tt0133093}`), then
    /// a scored title/year search. Returns false if nothing confident was found.
    async fn match_movie(
        &self,
        tmdb: &Tmdb,
        lookup: &MovieLookup,
    ) -> Result<bool, MetadataTaskError> {
        if lookup.match_locked {
            if let Some(id) = lookup.tmdb_id {
                if self.apply_movie(tmdb, lookup, id, true).await? {
                    return Ok(true);
                }
            }
        }
        if let Some(id) = lookup.hint_tmdb_id {
            if self.apply_movie(tmdb, lookup, id, false).await? {
                return Ok(true);
            }
        }
        if let Some(imdb) = &lookup.hint_imdb_id {
            if let Some(id) = tmdb.find_movie_by_imdb(imdb).await? {
                if self.apply_movie(tmdb, lookup, id, false).await? {
                    return Ok(true);
                }
            }
        }
        match tmdb
            .match_movie(&lookup.parsed_title, lookup.parsed_year)
            .await?
        {
            Some(candidate) => {
                tracing::debug!(
                    title = %lookup.parsed_title,
                    matched = %candidate.title,
                    score = candidate.score,
                    "matched movie"
                );
                self.apply_movie(tmdb, lookup, candidate.tmdb_id, false)
                    .await
            }
            None => Ok(false),
        }
    }

    /// Downloads full details and artwork for `tmdb_id` and stores them.
    /// Returns false if TMDB doesn't know the id.
    async fn apply_movie(
        &self,
        tmdb: &Tmdb,
        lookup: &MovieLookup,
        tmdb_id: i64,
        locked: bool,
    ) -> Result<bool, MetadataTaskError> {
        let Some(details) = tmdb.movie_details(tmdb_id).await? else {
            return Ok(false);
        };
        self.store_movie(tmdb, lookup, details, locked).await?;
        Ok(true)
    }

    /// Downloads artwork for `details` and stores everything for the movie.
    async fn store_movie(
        &self,
        tmdb: &Tmdb,
        lookup: &MovieLookup,
        details: MovieDetails,
        locked: bool,
    ) -> Result<(), MetadataTaskError> {
        // The TMDB id is part of the file name so browsers don't show a cached
        // poster from a previous (wrong) match.
        let stem = format!("movie-{}-{}", lookup.id, details.tmdb_id);
        let poster_file = self
            .image(
                tmdb,
                details.poster_path.as_deref(),
                "w500",
                format!("{stem}-poster"),
            )
            .await;
        let backdrop_file = self
            .image(
                tmdb,
                details.backdrop_path.as_deref(),
                "w1280",
                format!("{stem}-backdrop"),
            )
            .await;
        let same_match = lookup.tmdb_id == Some(details.tmdb_id);
        let meta = movie_metadata(details, &lookup.parsed_title, poster_file, backdrop_file);
        self.db
            .set_movie_metadata(lookup.id, &meta, locked, same_match)
            .await?;
        Ok(())
    }

    /// Sets a movie's TMDB match by hand ("Identify") and keeps it on future refreshes.
    pub async fn identify_movie(&self, id: MediaId, tmdb_id: i64) -> Result<(), MetadataTaskError> {
        let tmdb = self.tmdb().ok_or(MetadataTaskError::NoTmdb)?;
        let lookup = self
            .db
            .movie_lookup(id)
            .await?
            .ok_or(MetadataTaskError::NotAMovie)?;
        if self.apply_movie(&tmdb, &lookup, tmdb_id, true).await? {
            tracing::info!(item = %id, tmdb_id, "movie identified by hand");
            Ok(())
        } else {
            Err(MetadataTaskError::UnknownTmdbId)
        }
    }

    async fn fetch_shows(&self, tmdb: &Tmdb) -> Result<(), MetadataTaskError> {
        loop {
            let batch = self.db.shows_needing_metadata(BATCH).await?;
            if batch.is_empty() {
                return Ok(());
            }
            for show in batch {
                let meta = match tmdb.search_show(&show.title, show.year).await? {
                    Some(info) => ItemMetadata {
                        tmdb_id: Some(info.tmdb_id),
                        poster_file: self
                            .image(
                                tmdb,
                                info.poster_path.as_deref(),
                                "w500",
                                format!("show-{}-poster", show.id),
                            )
                            .await,
                        backdrop_file: self
                            .image(
                                tmdb,
                                info.backdrop_path.as_deref(),
                                "w1280",
                                format!("show-{}-backdrop", show.id),
                            )
                            .await,
                        title: Some(info.title),
                        year: info.year,
                        overview: info.overview,
                    },
                    None => {
                        tracing::info!(title = %show.title, "no TMDB match for show");
                        ItemMetadata::default()
                    }
                };
                self.db.set_show_metadata(show.id, &meta).await?;
            }
        }
    }

    async fn fetch_episodes(&self, tmdb: &Tmdb) -> Result<(), MetadataTaskError> {
        let mut show_tmdb: HashMap<ShowId, Option<i64>> = HashMap::new();
        let mut seasons: HashMap<(i64, i64), Vec<EpisodeInfo>> = HashMap::new();
        loop {
            let batch: Vec<MediaItem> = self.db.episodes_needing_metadata(BATCH).await?;
            if batch.is_empty() {
                return Ok(());
            }
            for ep in batch {
                let tmdb_show = match ep.show_id {
                    Some(show_id) => match show_tmdb.get(&show_id) {
                        Some(id) => *id,
                        None => {
                            let id = self.db.get_show(show_id).await?.and_then(|s| s.tmdb_id);
                            show_tmdb.insert(show_id, id);
                            id
                        }
                    },
                    None => None,
                };
                let meta = match (tmdb_show, ep.season, ep.episode) {
                    (Some(show), Some(season), Some(number)) => {
                        if let Entry::Vacant(slot) = seasons.entry((show, season)) {
                            slot.insert(tmdb.season(show, season).await?);
                        }
                        let info = seasons
                            .get(&(show, season))
                            .and_then(|list| list.iter().find(|e| e.episode_number == number))
                            .cloned();
                        self.episode_metadata(tmdb, ep.id, info).await
                    }
                    _ => ItemMetadata::default(),
                };
                self.db.set_item_metadata(ep.id, &meta).await?;
            }
        }
    }

    async fn episode_metadata(
        &self,
        tmdb: &Tmdb,
        id: MediaId,
        info: Option<EpisodeInfo>,
    ) -> ItemMetadata {
        let Some(info) = info else {
            return ItemMetadata::default();
        };
        ItemMetadata {
            tmdb_id: None,
            title: info.name,
            year: None,
            overview: info.overview,
            poster_file: None,
            backdrop_file: self
                .image(
                    tmdb,
                    info.still_path.as_deref(),
                    "w780",
                    format!("episode-{id}-still"),
                )
                .await,
        }
    }
}

/// Converts TMDB details into the database form.
fn movie_metadata(
    d: MovieDetails,
    fallback_title: &str,
    poster_file: Option<String>,
    backdrop_file: Option<String>,
) -> MovieMetadata {
    MovieMetadata {
        tmdb_id: d.tmdb_id,
        title: if d.title.is_empty() {
            fallback_title.to_string()
        } else {
            d.title
        },
        original_title: d.original_title,
        year: d.year,
        release_date: d.release_date,
        overview: d.overview,
        tagline: d.tagline,
        genres: d.genres,
        rating: d.rating,
        runtime_minutes: d.runtime_minutes,
        certification: d.certification,
        imdb_id: d.imdb_id,
        cast: d.cast,
        directors: d.directors,
        poster_file,
        backdrop_file,
    }
}

/// Queues the parts of libraries touched by a file-system event.
fn record_event(queue: &mut ChangeQueue, libraries: &[Library], event: WatchEvent) {
    let now = std::time::Instant::now();
    if event.rescan && event.paths.is_empty() {
        for library in libraries {
            queue.record(
                Change {
                    library: library.id,
                    target: None,
                },
                now,
            );
        }
        return;
    }
    for path in &event.paths {
        let Some(library) = library_for(libraries, path) else {
            continue;
        };
        let target = if event.rescan {
            None
        } else {
            match ScanTarget::for_path(Path::new(&library.path), path) {
                Some(target) => Some(target),
                None => continue,
            }
        };
        queue.record(
            Change {
                library: library.id,
                target,
            },
            now,
        );
    }
}

/// The library whose folder holds `path` (the innermost one if libraries are nested).
fn library_for<'a>(libraries: &'a [Library], path: &Path) -> Option<&'a Library> {
    libraries
        .iter()
        .filter(|library| path.starts_with(&library.path))
        .max_by_key(|library| library.path.len())
}

/// Sleeps until `due`, or forever if nothing is due.
async fn sleep_until(due: Option<tokio::time::Instant>) {
    match due {
        Some(due) => tokio::time::sleep_until(due).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidfin_core::{LibraryId, LibraryKind};

    fn library(id: i64, path: &Path) -> Library {
        Library {
            id: LibraryId(id),
            name: format!("Library {id}"),
            kind: LibraryKind::Movies,
            path: path.to_string_lossy().into_owned(),
            created_at: 0,
        }
    }

    #[test]
    fn events_queue_the_changed_folder_of_their_library() {
        let movies = Path::new("media").join("Movies");
        let shows = Path::new("media").join("TV");
        let libraries = [library(1, &movies), library(2, &shows)];
        let mut queue = ChangeQueue::new(Duration::ZERO);
        let heat = movies.join("Heat (1995)");
        record_event(
            &mut queue,
            &libraries,
            WatchEvent {
                paths: vec![
                    heat.join("Heat (1995).mkv"),
                    heat.join("Heat (1995).en.srt"),
                    Path::new("elsewhere").join("file.mkv"),
                ],
                rescan: false,
            },
        );
        record_event(
            &mut queue,
            &libraries,
            WatchEvent {
                paths: vec![shows.join("Firefly")],
                rescan: true,
            },
        );
        let ready = queue.take_ready(std::time::Instant::now());
        assert_eq!(
            ready,
            vec![
                Change {
                    library: LibraryId(1),
                    target: Some(ScanTarget::Folder(heat)),
                },
                Change {
                    library: LibraryId(2),
                    target: None,
                },
            ]
        );
    }
}

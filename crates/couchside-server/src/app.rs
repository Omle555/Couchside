//! Server startup and shutdown.

use crate::boot::{BootProgress, BootStep};
use crate::manager::LibraryManager;
use crate::title_scan::TitleScanner;
use crate::{api, boot, lan, logging};
use anyhow::Context;
use couchside_core::config::{
    default_config_path, migrate_legacy_data_dir, migrate_legacy_database, DB_FILE,
};
use couchside_core::Config;
use couchside_db::Db;
use couchside_db::{DbResult, ALLOW_FILE_RENAMES, METADATA_LANGUAGE, TMDB_API_KEY};
use couchside_library::{Ffmpeg, Ffprobe};
use couchside_metadata::Tmdb;
use couchside_transcode::{clear_temp_dir, ManagerSettings, TranscodeManager};
use std::future::{Future, IntoFuture};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// How long open streams get to finish after a shutdown request.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// How long the start-up page's connections get to finish before the app takes over.
const HANDOVER_GRACE: Duration = Duration::from_secs(2);
/// How long the start-up page keeps saying start-up failed before Couchside exits.
const FAILED_PAGE_TIME: Duration = Duration::from_secs(3);

/// Shared state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    /// Database handle.
    pub db: Db,
    /// Background scan/metadata worker.
    pub manager: Arc<LibraryManager>,
    /// Folder holding cached artwork.
    pub image_dir: Arc<PathBuf>,
    /// Serialises first-run setup so only one admin can be created.
    pub setup_lock: Arc<tokio::sync::Mutex<()>>,
    /// Settings loaded from config.toml at startup.
    pub config: Arc<Config>,
    /// "Scan video for title" in the Identify dialog.
    pub title_scanner: Arc<TitleScanner>,
    /// Repackaged / converted streams (HLS).
    pub transcoder: Arc<TranscodeManager>,
    /// ffprobe, used to read keyframe times before repackaging.
    pub ffprobe: Option<Ffprobe>,
}

/// Where the TMDB key in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// Entered in Settings → Metadata (stored in the database).
    App,
    /// From `[metadata] tmdb_api_key` in config.toml.
    Config,
}

/// The TMDB key and language in effect: values saved in Settings win over config.toml.
pub async fn effective_metadata_settings(
    db: &Db,
    config: &Config,
) -> DbResult<(Option<(String, KeySource)>, String)> {
    let stored_key = db
        .get_setting(TMDB_API_KEY)
        .await?
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let key = match stored_key {
        Some(k) => Some((k, KeySource::App)),
        None => config
            .tmdb_key()
            .map(|k| (k.to_string(), KeySource::Config)),
    };
    let language = db
        .get_setting(METADATA_LANGUAGE)
        .await?
        .unwrap_or_else(|| config.metadata.language.clone());
    Ok((key, language))
}

/// Whether "Clean up" may rename movie files: the switch in Settings wins over
/// `[library] allow_file_renames` in config.toml.
pub async fn renames_allowed(db: &Db, config: &Config) -> DbResult<bool> {
    Ok(match db.get_setting(ALLOW_FILE_RENAMES).await?.as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => config.library.allow_file_renames,
    })
}

/// Where to look for ffmpeg first: the configured path, then next to ffprobe.
fn ffmpeg_candidates(config: &Config, ffprobe: Option<&Ffprobe>) -> Vec<PathBuf> {
    let exe_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let beside_ffprobe = ffprobe
        .and_then(|f| f.path().parent())
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(exe_name));
    config
        .ffmpeg
        .ffmpeg_path
        .clone()
        .into_iter()
        .chain(beside_ffprobe)
        .collect()
}

/// Runs the server in the foreground until Ctrl+C.
pub fn run_console(config: Option<PathBuf>, open_browser: bool) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    runtime.block_on(run(
        config,
        async {
            if let Err(err) = tokio::signal::ctrl_c().await {
                tracing::error!(error = %err, "could not listen for Ctrl+C");
                std::future::pending::<()>().await;
            }
        },
        true,
        open_browser,
    ))
}

/// Starts everything and serves HTTP until `shutdown` completes.
pub async fn run(
    config_path: Option<PathBuf>,
    shutdown: impl Future<Output = ()> + Send + 'static,
    console: bool,
    open_browser: bool,
) -> anyhow::Result<()> {
    // Before loading the config, so it doesn't create an empty new data folder.
    let moved_dir =
        migrate_legacy_data_dir().context("could not move the old Solidfin data folder")?;
    let config_path = config_path.unwrap_or_else(default_config_path);
    let config = Config::load_or_create(&config_path)?;
    let data_dir = config.data_dir();
    let image_dir = data_dir.join("images");
    std::fs::create_dir_all(&image_dir)
        .with_context(|| format!("could not create data folder {}", data_dir.display()))?;
    let _log_guard = logging::init(&data_dir.join("logs"), console)?;
    if let Some(old) = moved_dir {
        tracing::info!(from = %old.display(), to = %data_dir.display(), "moved the old Solidfin data folder");
    }
    if migrate_legacy_database(&data_dir).context("could not rename the old Solidfin database")? {
        tracing::info!(file = DB_FILE, "renamed the old Solidfin database");
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %config_path.display(),
        data = %data_dir.display(),
        "starting Couchside"
    );

    let bind: IpAddr = config.server.bind.trim().parse().with_context(|| {
        format!(
            "invalid [server] bind address {:?} in config.toml (use 127.0.0.1 or 0.0.0.0)",
            config.server.bind
        )
    })?;
    let addr = SocketAddr::new(bind, config.server.port);
    let socket = std::net::TcpListener::bind(addr)
        .with_context(|| format!("could not listen on {addr} (is Couchside already running?)"))?;
    socket
        .set_nonblocking(true)
        .context("could not set up the network listener")?;
    let browse_host = if bind.is_unspecified() {
        "localhost".to_string()
    } else {
        lan::url_host(bind)
    };
    let url = format!("http://{browse_host}:{}", config.server.port);

    // Serve the splash screen right away so the browser can show start-up progress.
    let progress = BootProgress::default();
    let startup_page = StartupPage::serve(&socket, progress.clone(), config.server.lan_only)?;
    if console && open_browser {
        open_in_browser(&url);
    }
    let state = match start_services(&config, image_dir, &progress).await {
        Ok(state) => state,
        Err(err) => {
            progress.fail();
            tokio::time::sleep(FAILED_PAGE_TIME).await;
            startup_page.stop().await;
            return Err(err);
        }
    };
    startup_page.stop().await;
    let db = state.db.clone();
    let transcoder = Arc::clone(&state.transcoder);
    let router = api::router(state);
    let listener = tokio::net::TcpListener::from_std(socket)
        .context("could not set up the network listener")?;

    let network_urls = lan::network_urls(bind, config.server.port).await;
    tracing::info!(%url, "Couchside is running");
    for network_url in &network_urls {
        tracing::info!(url = %network_url, lan_only = config.server.lan_only, "open from other devices on your network");
    }
    if console {
        println!("\n  Couchside is running at {url}");
        for network_url in &network_urls {
            println!("  On other devices on your network: {network_url}");
        }
        println!("  Press Ctrl+C to stop.\n");
    }

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop_tx.send(true);
    });
    let mut graceful_rx = stop_rx.clone();
    let server = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = graceful_rx.wait_for(|stop| *stop).await;
        tracing::info!("shutting down");
    })
    .into_future();
    let mut force_rx = stop_rx;
    tokio::select! {
        result = server => result.context("server error")?,
        _ = async {
            let _ = force_rx.wait_for(|stop| *stop).await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => tracing::info!("closing remaining connections"),
    }

    transcoder.shutdown().await;
    db.close().await;
    tracing::info!("stopped");
    Ok(())
}

/// Opens the database, finds the tools and starts the library worker,
/// reporting each step to the start-up page.
async fn start_services(
    config: &Config,
    image_dir: PathBuf,
    progress: &BootProgress,
) -> anyhow::Result<AppState> {
    progress.begin(BootStep::Database);
    let db = Db::open(&config.data_dir().join(DB_FILE))
        .await
        .context("could not open the database")?;

    progress.begin(BootStep::Tools);
    let ffprobe = Ffprobe::locate(config.ffmpeg.ffprobe_path.as_deref()).await;
    let ffmpeg = Ffmpeg::locate(ffmpeg_candidates(config, ffprobe.as_ref())).await;
    clear_transcode_dir(config.transcode_dir()).await;

    progress.begin(BootStep::Settings);
    let (key, language) = effective_metadata_settings(&db, config).await?;
    let tmdb = match key {
        Some((key, _)) => Some(Tmdb::new(&key, &language, image_dir.clone())?),
        None => {
            tracing::info!(
                "no TMDB key configured; using file names only (add one under Settings → Metadata)"
            );
            None
        }
    };

    progress.begin(BootStep::Library);
    let transcoder = TranscodeManager::new(ManagerSettings {
        ffmpeg: ffmpeg.as_ref().map(|f| f.path().to_path_buf()),
        temp_dir: config.transcode_dir(),
        max_remuxes: config.transcode.max_remuxes,
        temp_max_bytes: config.transcode.temp_max_gb.saturating_mul(1 << 30),
    });
    let manager = LibraryManager::new(db.clone(), ffprobe.clone(), tmdb);
    manager.spawn_schedule(config.scan.interval_minutes, config.scan.scan_on_startup);
    if config.scan.watch_folders {
        manager.spawn_watcher(std::time::Duration::from_secs(
            config.scan.watch_delay_seconds,
        ));
    }

    Ok(AppState {
        db,
        manager,
        image_dir: Arc::new(image_dir),
        setup_lock: Arc::new(tokio::sync::Mutex::new(())),
        config: Arc::new(config.clone()),
        title_scanner: Arc::new(TitleScanner::new(ffmpeg)),
        transcoder,
        ffprobe,
    })
}

/// Removes transcode folders left by a previous run. A failure is logged, not fatal:
/// playback of original files doesn't need the folder.
async fn clear_transcode_dir(dir: PathBuf) {
    let shown = dir.display().to_string();
    match tokio::task::spawn_blocking(move || clear_temp_dir(&dir)).await {
        Ok(Ok(0)) => {}
        Ok(Ok(removed)) => {
            tracing::info!(dir = %shown, removed, "removed old transcode folders");
        }
        Ok(Err(err)) => {
            tracing::warn!(dir = %shown, error = %err, "could not prepare transcode folder");
        }
        Err(err) => tracing::warn!(error = %err, "transcode folder cleanup did not finish"),
    }
}

/// The start-up page, served on a copy of the listening socket until the app is ready.
struct StartupPage {
    stop: watch::Sender<bool>,
    task: JoinHandle<std::io::Result<()>>,
}

impl StartupPage {
    fn serve(
        socket: &std::net::TcpListener,
        progress: BootProgress,
        lan_only: bool,
    ) -> anyhow::Result<Self> {
        let socket = socket
            .try_clone()
            .and_then(tokio::net::TcpListener::from_std)
            .context("could not set up the network listener")?;
        let (stop, mut stop_rx) = watch::channel(false);
        let app =
            boot::router(progress, lan_only).into_make_service_with_connect_info::<SocketAddr>();
        let server = axum::serve(socket, app).with_graceful_shutdown(async move {
            let _ = stop_rx.wait_for(|stop| *stop).await;
        });
        Ok(Self {
            stop,
            task: tokio::spawn(server.into_future()),
        })
    }

    /// Stops the start-up page so the app can take over the port.
    async fn stop(self) {
        let _ = self.stop.send(true);
        let abort = self.task.abort_handle();
        match tokio::time::timeout(HANDOVER_GRACE, self.task).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(err))) => tracing::warn!(error = %err, "start-up page stopped with an error"),
            Ok(Err(err)) => tracing::warn!(error = %err, "start-up page task failed"),
            Err(_) => {
                abort.abort();
                tracing::debug!("closed slow start-up page connections");
            }
        }
    }
}

/// Opens the default browser (best effort; Windows only).
fn open_in_browser(url: &str) {
    #[cfg(windows)]
    {
        // Arguments are passed as a list; the URL is built by us, not user input.
        if let Err(err) = std::process::Command::new("explorer").arg(url).spawn() {
            tracing::debug!(error = %err, "could not open browser");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
}

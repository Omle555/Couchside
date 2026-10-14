//! Server configuration loaded from `config.toml`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Errors while loading or creating the configuration file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("could not read config file {path}: {source}")]
    Read {
        /// Config file path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The default file could not be written.
    #[error("could not write config file {path}: {source}")]
    Write {
        /// Config file path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not valid TOML or has wrong field types.
    #[error("invalid config file {path}: {source}")]
    Parse {
        /// Config file path.
        path: PathBuf,
        /// Underlying parse error.
        #[source]
        source: Box<toml::de::Error>,
    },
}

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// HTTP listener settings.
    pub server: ServerConfig,
    /// Where Couchside keeps its database, cache and logs.
    pub paths: PathsConfig,
    /// FFmpeg tool locations.
    pub ffmpeg: FfmpegConfig,
    /// Online metadata provider settings.
    pub metadata: MetadataConfig,
    /// Library scanning schedule.
    pub scan: ScanConfig,
    /// What Couchside may do to files in your libraries.
    pub library: LibraryConfig,
    /// Converting files that clients can't play as they are.
    pub transcode: TranscodeConfig,
}

/// HTTP listener settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address to bind. `127.0.0.1` = this PC only, `0.0.0.0` = whole network.
    pub bind: String,
    /// TCP port.
    pub port: u16,
    /// Only answer devices on the local network (private, link-local and loopback addresses).
    pub lan_only: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 8096,
            lan_only: true,
        }
    }
}

/// Data directory override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    /// Overrides the default `%ProgramData%\Couchside` data directory.
    pub data_dir: Option<PathBuf>,
}

/// FFmpeg tool locations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FfmpegConfig {
    /// Full path to `ffprobe.exe`. If unset, Couchside looks next to its own exe and on `PATH`.
    pub ffprobe_path: Option<PathBuf>,
    /// Full path to `ffmpeg.exe`. If unset, Couchside looks next to ffprobe, next to its own exe
    /// and on `PATH`.
    pub ffmpeg_path: Option<PathBuf>,
}

/// Online metadata settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataConfig {
    /// TMDB API key (v3) or read access token (v4). Leave unset to disable online metadata.
    pub tmdb_api_key: Option<String>,
    /// Language for titles and overviews, e.g. `en-US`, `en-AU`, `fr-FR`.
    pub language: String,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self {
            tmdb_api_key: None,
            language: "en-US".to_string(),
        }
    }
}

/// Library scanning schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Minutes between automatic rescans. 0 disables periodic scans.
    pub interval_minutes: u64,
    /// Scan all libraries when the server starts.
    pub scan_on_startup: bool,
    /// Watch library folders and update the library as soon as files are added, removed
    /// or renamed.
    pub watch_folders: bool,
    /// Seconds a changed folder must stay quiet before it is rescanned, so files that are
    /// still being copied are not read half-written.
    pub watch_delay_seconds: u64,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            interval_minutes: 60,
            scan_on_startup: true,
            watch_folders: true,
            watch_delay_seconds: 10,
        }
    }
}

/// What Couchside may do to files in your libraries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryConfig {
    /// Lets "Clean up" rename movie files to `Title (Year)/Title (Year) 1080p.ext`.
    /// Off by default: Couchside never touches your files unless this is switched on.
    pub allow_file_renames: bool,
}

/// Which hardware encodes video when a file must be converted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HardwareAccel {
    /// The processor only (libx264).
    #[default]
    None,
    /// An NVIDIA graphics card (NVENC), falling back to the processor if it fails.
    Nvenc,
}

/// Converting files that clients can't play as they are.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscodeConfig {
    /// Hardware used to encode video. Opt-in; the processor is the default.
    pub hardware: HardwareAccel,
    /// Most full video conversions running at once.
    pub max_transcodes: u32,
    /// Most remux / audio-only conversions running at once.
    pub max_remuxes: u32,
    /// Overrides the default `transcodes` folder inside the data directory.
    pub temp_dir: Option<PathBuf>,
    /// Gigabytes temporary segments may use before idle sessions are stopped.
    pub temp_max_gb: u64,
}

impl Default for TranscodeConfig {
    fn default() -> Self {
        Self {
            hardware: HardwareAccel::None,
            max_transcodes: 2,
            max_remuxes: 4,
            temp_dir: None,
            temp_max_gb: 20,
        }
    }
}

/// Text written when no config file exists yet.
const DEFAULT_CONFIG_TEXT: &str = r#"# Couchside configuration. Restart Couchside after editing.

[server]
# 127.0.0.1 = only this PC can connect. Use 0.0.0.0 to allow other devices on your network.
bind = "127.0.0.1"
port = 8096
# true = only devices on your home network (and this PC) can connect, even with bind = "0.0.0.0".
lan_only = true

[paths]
# data_dir = 'D:\CouchsideData'

[ffmpeg]
# Full path to ffprobe.exe. If not set, Couchside looks next to couchside.exe and on PATH.
# ffprobe_path = 'C:\ffmpeg\bin\ffprobe.exe'
# Full path to ffmpeg.exe (used by Identify's title scan). If not set, Couchside looks next to
# ffprobe.exe, next to couchside.exe and on PATH.
# ffmpeg_path = 'C:\ffmpeg\bin\ffmpeg.exe'

[metadata]
# Free key from https://www.themoviedb.org/settings/api (API key or Read Access Token).
# tmdb_api_key = "your-key-here"
language = "en-US"

[scan]
interval_minutes = 60
scan_on_startup = true
# true = update a library as soon as files are added, removed or renamed in its folders.
watch_folders = true

[library]
# true = the movie page's "Clean up" button can rename a movie file (and the subtitles and
# artwork named after it) to "Title (Year)/Title (Year) 1080p.mkv". You always see the old
# and new names first. Renames are listed in logs\renames.log.
# The "Allow file renames" switch in Settings overrides this once it has been used.
allow_file_renames = false

[transcode]
# Converting files your browser or TV can't play. Full video conversion (hardware,
# max_transcodes) arrives in a later update; repackaging and sound conversion work now.
# "nvenc" = use an NVIDIA graphics card (falls back to the processor), "none" = processor only.
hardware = "none"
max_transcodes = 2
max_remuxes = 4
# temp_dir = 'D:\CouchsideTranscodes'
"#;

impl Config {
    /// Loads the config file, creating it with defaults if it does not exist.
    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                    path: path.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(path, DEFAULT_CONFIG_TEXT).map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }

    /// Effective data directory (database, images, logs).
    pub fn data_dir(&self) -> PathBuf {
        self.paths.data_dir.clone().unwrap_or_else(default_data_dir)
    }

    /// Folder for temporary transcode segments.
    pub fn transcode_dir(&self) -> PathBuf {
        self.transcode
            .temp_dir
            .clone()
            .unwrap_or_else(|| self.data_dir().join("transcodes"))
    }

    /// TMDB key, ignoring blank values.
    pub fn tmdb_key(&self) -> Option<&str> {
        self.metadata
            .tmdb_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
    }
}

/// Default data directory: `%ProgramData%\Couchside` on Windows.
pub fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("Couchside")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/couchside")
    } else {
        PathBuf::from("data")
    }
}

/// Default config file location inside the default data directory.
pub fn default_config_path() -> PathBuf {
    default_data_dir().join("config.toml")
}

/// Database file name inside the data directory.
pub const DB_FILE: &str = "couchside.db";

/// Database file name used before the project was renamed from Solidfin.
const LEGACY_DB_FILE: &str = "solidfin.db";

/// Moves the data folder from before the rename (`%ProgramData%\Solidfin`) to the
/// default data directory, if only the old one exists. Returns the old folder when moved.
pub fn migrate_legacy_data_dir() -> std::io::Result<Option<PathBuf>> {
    migrate_legacy_dir(&default_data_dir())
}

fn migrate_legacy_dir(new_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let legacy = match new_dir.file_name().and_then(|n| n.to_str()) {
        Some("Couchside") => new_dir.with_file_name("Solidfin"),
        Some("couchside") => new_dir.with_file_name("solidfin"),
        _ => return Ok(None),
    };
    if new_dir.exists() || !legacy.is_dir() {
        return Ok(None);
    }
    std::fs::rename(&legacy, new_dir)?;
    Ok(Some(legacy))
}

/// Renames the database from before the rename (`solidfin.db` and its `-wal`/`-shm`
/// files) to [`DB_FILE`] in `data_dir`. Returns true when it was renamed.
pub fn migrate_legacy_database(data_dir: &Path) -> std::io::Result<bool> {
    if data_dir.join(DB_FILE).exists() || !data_dir.join(LEGACY_DB_FILE).is_file() {
        return Ok(false);
    }
    // The main file goes last so an interrupted move is retried on the next start.
    for suffix in ["-wal", "-shm", ""] {
        let old = data_dir.join(format!("{LEGACY_DB_FILE}{suffix}"));
        if old.exists() {
            std::fs::rename(&old, data_dir.join(format!("{DB_FILE}{suffix}")))?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_parses() {
        let cfg: Config = toml::from_str(DEFAULT_CONFIG_TEXT).expect("default config is valid");
        assert_eq!(cfg.server.port, 8096);
        assert_eq!(cfg.server.bind, "127.0.0.1");
        assert!(cfg.server.lan_only);
        assert!(cfg.tmdb_key().is_none());
        assert!(!cfg.library.allow_file_renames);
        assert_eq!(cfg.transcode.hardware, HardwareAccel::None);
        assert_eq!(cfg.transcode.max_transcodes, 2);
    }

    #[test]
    fn transcode_settings() {
        let cfg: Config = toml::from_str(
            "[paths]\ndata_dir = 'D:\\Data'\n[transcode]\nhardware = \"nvenc\"\nmax_transcodes = 1\n",
        )
        .expect("valid");
        assert_eq!(cfg.transcode.hardware, HardwareAccel::Nvenc);
        assert_eq!(cfg.transcode.max_transcodes, 1);
        assert_eq!(cfg.transcode.max_remuxes, 4);
        assert_eq!(
            cfg.transcode_dir(),
            PathBuf::from("D:\\Data").join("transcodes")
        );
        assert!(toml::from_str::<Config>("[transcode]\nhardware = \"quantum\"\n").is_err());
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg: Config = toml::from_str("[server]\nport = 9000\n").expect("valid");
        assert_eq!(cfg.server.port, 9000);
        assert_eq!(cfg.server.bind, "127.0.0.1");
        assert!(cfg.server.lan_only);
        assert_eq!(cfg.scan.interval_minutes, 60);
        assert!(cfg.scan.watch_folders);
        assert_eq!(cfg.scan.watch_delay_seconds, 10);
        assert!(!cfg.library.allow_file_renames);
    }

    #[test]
    fn legacy_data_is_moved_once() {
        let base = std::env::temp_dir().join(format!("couchside-migrate-{}", std::process::id()));
        let old = base.join("Solidfin");
        let new = base.join("Couchside");
        std::fs::create_dir_all(&old).expect("create old dir");
        std::fs::write(old.join(LEGACY_DB_FILE), b"db").expect("write db");
        std::fs::write(old.join("solidfin.db-wal"), b"wal").expect("write wal");

        assert_eq!(
            migrate_legacy_dir(&new).expect("move dir"),
            Some(old.clone())
        );
        assert!(new.is_dir() && !old.exists());
        assert!(migrate_legacy_database(&new).expect("move db"));
        assert!(new.join(DB_FILE).is_file());
        assert!(new.join("couchside.db-wal").is_file());
        assert!(!new.join(LEGACY_DB_FILE).exists());

        // Nothing left to move: both are no-ops.
        assert_eq!(migrate_legacy_dir(&new).expect("no-op"), None);
        assert!(!migrate_legacy_database(&new).expect("no-op"));

        std::fs::remove_dir_all(&base).ok();
    }
}

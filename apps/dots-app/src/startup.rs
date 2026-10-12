//! Everything between process start and a running `Runtime`, kept free of Tauri types so it
//! can be tested headless.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dots_core::settings::{self, Settings};
use dots_core::{Config, Runtime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

/// Passed by the autostart entry: start in the tray without showing the window.
pub const HIDDEN_ARG: &str = "--hidden";
const LOG_FILES_KEPT: usize = 7;

/// Overrides the data directory, for smoke tests and a second development instance.
pub const DATA_DIR_ENV: &str = "DOTS_DATA_DIR";

/// `%LOCALAPPDATA%\dots` (spec §3) given `%LOCALAPPDATA%`, unless `DOTS_DATA_DIR` is set.
pub fn data_dir(local_data: &Path, env_override: Option<OsString>) -> PathBuf {
    match env_override.filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => local_data.join("dots"),
    }
}

pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn start_hidden(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|a| a == HIDDEN_ARG)
}

/// Daily log files in `log_dir`, the last seven kept. `None` if logging could not start;
/// the app runs without a log then. Keep the guard alive for the whole run.
pub fn init_logging(log_dir: &Path) -> Option<WorkerGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("dots")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(log_dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init()
        .ok()?;
    Some(guard)
}

/// Reads the stored settings and starts the runtime with them. The error is the text the
/// fatal-error page shows.
pub async fn start_runtime(data_dir: &Path) -> Result<(Arc<Runtime>, Settings), String> {
    let settings = settings::load(data_dir).await.map_err(|e| {
        format!(
            "Dots could not read its settings in {}: {e}",
            data_dir.display()
        )
    })?;
    let runtime = Runtime::start(Config::from_settings(data_dir, &settings))
        .await
        .map_err(|e| format!("Dots could not start: {e}"))?;
    Ok((Arc::new(runtime), settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_the_hidden_flag() {
        let local = Path::new("C:/Users/u/AppData/Local");
        assert_eq!(data_dir(local, None), local.join("dots"));
        assert_eq!(data_dir(local, Some("".into())), local.join("dots"));
        assert_eq!(
            data_dir(local, Some("D:/smoke".into())),
            PathBuf::from("D:/smoke")
        );
        assert_eq!(
            log_dir(&data_dir(local, None)),
            local.join("dots").join("logs")
        );
        let args = |a: &[&str]| {
            a.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert!(start_hidden(args(&["dots.exe", "--hidden"])));
        assert!(!start_hidden(args(&["dots.exe"])));
    }

    #[tokio::test]
    async fn starts_with_the_stored_settings() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("dots");
        // A port that is free right now, stored the way the Settings screen stores it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let store = dots_core::store::Store::open(&data.join("dots.db"))
            .await
            .unwrap();
        store
            .put_settings(&Settings {
                port,
                ..Settings::default()
            })
            .await
            .unwrap();
        store.close().await;

        let (rt, settings) = start_runtime(&data).await.unwrap();
        assert_eq!(settings.port, port);
        let status = rt.status().await;
        assert_eq!(status.port, port);
        assert_eq!(status.data_dir, data.to_string_lossy());
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_data_dir_that_cannot_be_created_is_a_startup_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let err = start_runtime(&file.join("dots")).await.err().unwrap();
        assert!(err.starts_with("Dots could not read its settings"), "{err}");
    }
}

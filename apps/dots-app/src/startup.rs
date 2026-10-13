//! Everything between process start and a running `Runtime`, kept free of Tauri types so it
//! can be tested headless.

use std::ffi::OsString;
use std::fmt::Display;
use std::io::Write;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dots_core::settings::{self, Settings};
use dots_core::{Config, Runtime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// Passed by the autostart entry: start in the tray without showing the window.
pub const HIDDEN_ARG: &str = "--hidden";
const LOG_FILES_KEPT: usize = 7;

/// Overrides the data directory, for smoke tests and a second development instance.
pub const DATA_DIR_ENV: &str = "DOTS_DATA_DIR";

/// The folder `DOTS_DATA_DIR` names. An empty value counts as not set.
fn data_dir_override(env_override: Option<OsString>) -> Option<OsString> {
    env_override.filter(|v| !v.is_empty())
}

/// `%LOCALAPPDATA%\dots` (spec §3) given `%LOCALAPPDATA%`, unless `DOTS_DATA_DIR` is set; then
/// `local_data` is not needed. The override is made absolute here, from the current folder
/// and without touching the disk: the core hands paths below the data folder to git, which
/// runs in a dot's folder, and compares stored worktree paths with it, so one folder must
/// have one spelling. The error is the text the fatal-error page shows.
pub fn data_dir<E: Display>(
    local_data: Result<PathBuf, E>,
    env_override: Option<OsString>,
) -> Result<PathBuf, String> {
    match data_dir_override(env_override) {
        Some(dir) => std::path::absolute(&dir).map_err(|e| {
            format!(
                "Dots could not use the folder {} from {DATA_DIR_ENV}: {e}",
                Path::new(&dir).display()
            )
        }),
        None => local_data
            .map(|local| local.join("dots"))
            .map_err(|e| format!("Dots could not find the local application data folder: {e}")),
    }
}

/// Whether a second launch hands over to the running app. Not under `DOTS_DATA_DIR`: the
/// plugin knows the app, not its data folder, so a smoke or development instance would hand
/// over to an installed Dots and its real data (spec §12).
pub fn single_instance(env_override: Option<OsString>) -> bool {
    data_dir_override(env_override).is_none()
}

/// Where a panic is logged before Tauri has built the app, from the environment alone:
/// `DOTS_DATA_DIR`, else `%LOCALAPPDATA%` (which is what Tauri's `local_data_dir` reads on
/// Windows, so `setup` finds the same folder). `None` when neither names a folder.
pub fn early_log_dir(
    local_app_data: Option<OsString>,
    env_override: Option<OsString>,
) -> Option<PathBuf> {
    let local = local_app_data
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or("%LOCALAPPDATA% is not set");
    data_dir(local, env_override).ok().map(|d| log_dir(&d))
}

/// The file every running Dots holds a lock on, in its data folder.
pub const LOCK_FILE: &str = "dots.lock";

/// How long a start waits for another process's lock on the data folder: "Restart Dots"
/// starts the new process while the old one is still exiting.
pub const LOCK_WAIT: Duration = Duration::from_secs(3);

/// Takes an exclusive lock on `dots.lock` in `data_dir` and returns the file: the lock lasts
/// as long as it is kept. Two Dots on one data folder would run two runtimes on one database
/// (the second marks the first's live runs as failed). Only a lock held by another process is
/// waited for, up to `wait`; any other error returns at once.
pub fn lock_data_dir(data_dir: &Path, wait: Duration) -> Result<std::fs::File, String> {
    let fail = |what: &str, e: std::io::Error| {
        format!("Dots could not {what} in {}: {e}", data_dir.display())
    };
    std::fs::create_dir_all(data_dir).map_err(|e| fail("create its data folder", e))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join(LOCK_FILE))
        .map_err(|e| fail("open its lock file", e))?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                if started.elapsed() >= wait {
                    return Err(format!(
                        "another Dots is already using the data folder {}. Close it (it may be in the tray), then start Dots again.",
                        data_dir.display()
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(fail("lock its data folder", e)),
        }
    }
}

pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn start_hidden(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|a| a == HIDDEN_ARG)
}

/// The file "Restart Dots" leaves in the data folder for the process it starts. Tauri restarts
/// the app with the arguments it was started with, so after a start at login the new process
/// gets `--hidden` again; the mark tells it that a person asked for it and is waiting.
pub const RESTART_MARK: &str = "show-after-restart";

/// Leaves the restart mark in `data_dir`. A failure is logged and not returned: the restart
/// goes on, and at worst the window stays in the tray as it did before there was a mark.
pub fn mark_restart(data_dir: &Path) {
    if let Err(e) = std::fs::write(data_dir.join(RESTART_MARK), "") {
        tracing::warn!(
            "could not leave the restart mark in {}: {e}",
            data_dir.display()
        );
    }
}

/// Whether "Restart Dots" left its mark in `data_dir`; the mark is removed, so it counts for
/// one start. Call it only while holding the lock on the data folder.
pub fn take_restart_mark(data_dir: &Path) -> bool {
    let mark = data_dir.join(RESTART_MARK);
    if !mark.is_file() {
        return false;
    }
    if let Err(e) = std::fs::remove_file(&mark) {
        tracing::warn!("could not remove the restart mark {}: {e}", mark.display());
    }
    true
}

/// Whether a start shows the main window: always after a failed start (its error page), when
/// Dots was not started hidden, and after "Restart Dots" whatever the arguments say.
pub fn show_window(failed: bool, hidden: bool, restarted: bool) -> bool {
    failed || !hidden || restarted
}

/// Today's log file in `log_dir`: daily files, the last seven kept. The folder is created
/// first: the appender looks for old files before it opens today's, and prints an error when
/// the folder is not there yet.
fn appender(log_dir: &Path) -> Option<RollingFileAppender> {
    std::fs::create_dir_all(log_dir).ok()?;
    Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("dots")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(log_dir)
        .ok()
}

/// Daily log files in `log_dir`, the last seven kept. `None` if logging could not start;
/// the app runs without a log then. Keep the guard alive for the whole run.
pub fn init_logging(log_dir: &Path) -> Option<WorkerGuard> {
    let appender = appender(log_dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init()
        .ok()?;
    Some(guard)
}

/// Writes a panic to today's log file, as a line shaped like the logger's. It goes straight
/// to the file and not through `tracing`: the logger hands its lines to a writer thread, and
/// a panic can end the process before that thread has written them. Nothing here may panic.
pub fn record_panic(
    log_dir: &Path,
    thread: Option<&str>,
    location: Option<&Location<'_>>,
    message: Option<&str>,
) {
    let place = location.map_or_else(|| "an unknown place".to_string(), Location::to_string);
    let line = format!(
        "{} ERROR dots_app: thread '{}' panicked at {place}: {}\n",
        dots_core::util::now(),
        thread.unwrap_or("<unnamed>"),
        message.unwrap_or("(no message)"),
    );
    if let Some(mut log) = appender(log_dir) {
        let _ = log.write_all(line.as_bytes());
    }
}

/// From here on a panic leaves a line in the log. The hook that was installed before still
/// runs afterwards (the default one prints the panic to stderr, which a release build does
/// not have).
pub fn log_panics(log_dir: PathBuf) {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record_panic(
            &log_dir,
            std::thread::current().name(),
            info.location(),
            info.payload_as_str(),
        );
        before(info);
    }));
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
    use crate::state::{runtime_of, AppState};

    /// `%LOCALAPPDATA%` as Tauri found it.
    fn found(local: &Path) -> Result<PathBuf, String> {
        Ok(local.to_path_buf())
    }

    /// The text of the only file in `log_dir`, which must be one of the logger's daily files.
    fn only_log_file(log_dir: &Path) -> String {
        let files: Vec<PathBuf> = std::fs::read_dir(log_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("dots.") && name.ends_with(".log"),
            "{name}"
        );
        std::fs::read_to_string(&files[0]).unwrap()
    }

    #[test]
    fn paths_and_the_hidden_flag() {
        let local = Path::new("C:/Users/u/AppData/Local");
        assert_eq!(data_dir(found(local), None), Ok(local.join("dots")));
        assert_eq!(
            data_dir(found(local), Some("".into())),
            Ok(local.join("dots"))
        );
        assert_eq!(
            log_dir(&local.join("dots")),
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

    #[test]
    fn a_data_dir_override_is_made_absolute() {
        let local = Path::new("C:/Users/u/AppData/Local");
        let cwd = std::env::current_dir().unwrap();
        // Compared as text: the core compares stored worktree paths with this folder.
        let given =
            |dir: &str| data_dir(found(local), Some(dir.into())).map(PathBuf::into_os_string);
        assert_eq!(given("smoke-data"), Ok(cwd.join("smoke-data").into()));
        let absolute = cwd.join("elsewhere");
        assert_eq!(
            given(&absolute.to_string_lossy()),
            Ok(absolute.clone().into())
        );
        if cfg!(windows) {
            // One spelling for one folder, however the variable spells it.
            assert_eq!(given("./smoke-data"), Ok(cwd.join("smoke-data").into()));
            assert_eq!(
                given("nested/../smoke-data"),
                Ok(cwd.join("smoke-data").into())
            );
            assert_eq!(
                given(&absolute.to_string_lossy().replace('\\', "/")),
                Ok(absolute.into())
            );
        }
    }

    #[test]
    fn a_missing_local_data_folder_is_a_startup_error_unless_the_override_names_a_folder() {
        let missing = || Err::<PathBuf, _>("unknown path");
        assert_eq!(
            data_dir(missing(), None),
            Err("Dots could not find the local application data folder: unknown path".into())
        );
        assert_eq!(
            data_dir(missing(), Some("".into())),
            Err("Dots could not find the local application data folder: unknown path".into())
        );
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            data_dir(missing(), Some("smoke-data".into())),
            Ok(cwd.join("smoke-data"))
        );
    }

    #[test]
    fn single_instance_is_skipped_under_a_data_dir_override() {
        assert!(single_instance(None));
        // An empty value is "not set", as it is for `data_dir`.
        assert!(single_instance(Some("".into())));
        assert!(!single_instance(Some("D:/smoke".into())));
        assert!(!single_instance(Some("smoke-data".into())));
    }

    #[test]
    fn a_restart_mark_is_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join(RESTART_MARK);
        // A start that no restart came before finds no mark.
        assert!(!take_restart_mark(dir.path()));
        mark_restart(dir.path());
        assert!(mark.is_file());
        // Marking twice leaves one mark.
        mark_restart(dir.path());
        assert!(take_restart_mark(dir.path()));
        assert!(!mark.exists(), "the mark was not removed");
        assert!(!take_restart_mark(dir.path()));
    }

    #[test]
    fn a_restart_mark_that_cannot_be_written_does_not_stop_the_restart() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-folder");
        mark_restart(&missing);
        assert!(!missing.exists());
        assert!(!take_restart_mark(&missing));
    }

    #[test]
    fn the_window_is_shown_after_a_failed_start_an_open_start_or_a_restart() {
        const FAILED: bool = true;
        const HIDDEN: bool = true;
        const RESTARTED: bool = true;
        // Started at login: in the tray, without a window.
        assert!(!show_window(!FAILED, HIDDEN, !RESTARTED));
        // "Restart Dots" passes `--hidden` on from a login start: the window still comes back.
        assert!(show_window(!FAILED, HIDDEN, RESTARTED));
        // A failed start always shows its error page.
        assert!(show_window(FAILED, HIDDEN, !RESTARTED));
        assert!(show_window(FAILED, HIDDEN, RESTARTED));
        // Started by the person: shown, whatever else is true.
        for failed in [false, true] {
            for restarted in [false, true] {
                assert!(show_window(failed, !HIDDEN, restarted));
            }
        }
    }

    const CHILD_LOG_DIR: &str = "DOTS_TEST_CHILD_LOG_DIR";

    /// Runs one test of this binary, a `child_…` one below, in a process of its own with
    /// `log_dir` handed over: its stderr can be read there, and it may panic for real.
    /// Returns whether it passed, and its stdout and stderr.
    fn run_child(test: &str, log_dir: &Path) -> (bool, String, String) {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("startup::tests::{test}"), "--nocapture"])
            .env(CHILD_LOG_DIR, log_dir)
            .output()
            .unwrap();
        (
            child.status.success(),
            String::from_utf8_lossy(&child.stdout).into_owned(),
            String::from_utf8_lossy(&child.stderr).into_owned(),
        )
    }

    /// Does nothing on its own; see `the_log_folder_is_created_before_the_appender`.
    #[test]
    fn child_opens_the_log() {
        if let Some(dir) = std::env::var_os(CHILD_LOG_DIR) {
            assert!(appender(Path::new(&dir)).is_some());
        }
    }

    #[test]
    fn the_log_folder_is_created_before_the_appender() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("dots").join("logs");
        let (passed, out, err) = run_child("child_opens_the_log", &logs);
        assert!(passed && out.contains("1 passed"), "{out}{err}");
        assert_eq!(only_log_file(&logs), "");
        // The appender looks for old files before it opens today's, and reports a folder
        // that is not there yet as an error.
        assert_eq!(err, "");
    }

    /// Does nothing on its own; see `a_real_panic_reaches_the_log_and_then_the_hook_before`.
    #[test]
    fn child_panics() {
        if let Some(dir) = std::env::var_os(CHILD_LOG_DIR) {
            log_panics(PathBuf::from(dir));
            panic!("boom from the child");
        }
    }

    #[test]
    fn a_real_panic_reaches_the_log_and_then_the_hook_before() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let (passed, out, err) = run_child("child_panics", &logs);
        assert!(!passed && out.contains("1 failed"), "{out}{err}");
        let text = only_log_file(&logs);
        let line = text.strip_suffix('\n').unwrap();
        assert!(!line.contains('\n'), "{text}");
        assert!(line.contains(" ERROR dots_app: thread '"), "{text}");
        assert!(line.contains("startup.rs:"), "{text}");
        assert!(line.ends_with(": boom from the child"), "{text}");
        // The default hook still printed it.
        assert!(
            err.contains("panicked at") && err.contains("boom from the child"),
            "{err}"
        );
    }

    #[test]
    fn a_panic_is_written_to_the_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        // The logger's own file for today, with a line in it.
        appender(&logs).unwrap().write_all(b"earlier\n").unwrap();
        let here = Location::caller();
        record_panic(&logs, Some("main"), Some(here), Some("index out of bounds"));
        record_panic(&logs, None, None, None);
        let text = only_log_file(&logs);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[0], "earlier");
        let (ts, rest) = lines[1].split_once(' ').unwrap();
        assert!(
            ts.ends_with('Z') && ts.len() == "2026-10-13T08:00:00.000Z".len(),
            "{ts}"
        );
        assert_eq!(
            rest,
            format!("ERROR dots_app: thread 'main' panicked at {here}: index out of bounds")
        );
        assert!(
            lines[2].ends_with(
                " ERROR dots_app: thread '<unnamed>' panicked at an unknown place: (no message)"
            ),
            "{text}"
        );
        // A log folder that cannot be made must not turn one panic into two.
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        record_panic(&file.join("logs"), Some("main"), Some(here), Some("boom"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
    }

    #[test]
    fn the_early_log_folder_comes_from_the_environment_alone() {
        let local = || Some(OsString::from("C:/Users/u/AppData/Local"));
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            early_log_dir(local(), None),
            Some(Path::new("C:/Users/u/AppData/Local/dots/logs").to_path_buf())
        );
        assert_eq!(
            early_log_dir(local(), Some("".into())),
            early_log_dir(local(), None)
        );
        assert_eq!(
            early_log_dir(None, Some("smoke-data".into())),
            Some(cwd.join("smoke-data").join("logs"))
        );
        let absolute = cwd.join("elsewhere");
        assert_eq!(
            early_log_dir(local(), Some(absolute.clone().into())),
            Some(absolute.join("logs"))
        );
        assert_eq!(early_log_dir(None, None), None);
        assert_eq!(early_log_dir(None, Some("".into())), None);
    }

    #[test]
    fn the_early_log_folder_is_the_one_setup_computes() {
        // `setup` gets `%LOCALAPPDATA%` from Tauri; the early path reads the variable.
        let local = Path::new("C:/Users/u/AppData/Local");
        for env in [None, Some(OsString::from("smoke-data"))] {
            let in_setup = data_dir(found(local), env.clone())
                .map(|d| log_dir(&d))
                .ok();
            assert_eq!(early_log_dir(Some(local.into()), env), in_setup);
        }
    }

    const NO_WAIT: Duration = Duration::from_millis(150);

    #[test]
    fn the_lock_file_is_made_in_a_folder_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("a").join("dots");
        let _lock = lock_data_dir(&data, NO_WAIT).unwrap();
        assert!(data.join(LOCK_FILE).is_file());
    }

    #[test]
    fn a_second_lock_on_one_folder_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock_data_dir(dir.path(), NO_WAIT).unwrap();
        let started = std::time::Instant::now();
        let err = lock_data_dir(dir.path(), NO_WAIT).unwrap_err();
        assert!(started.elapsed() >= NO_WAIT);
        assert_eq!(
            err,
            format!(
                "another Dots is already using the data folder {}. Close it (it may be in the tray), then start Dots again.",
                dir.path().display()
            )
        );
        drop(first);
        assert!(lock_data_dir(dir.path(), NO_WAIT).is_ok());
    }

    #[test]
    fn a_lock_released_during_the_wait_is_obtained() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock_data_dir(dir.path(), NO_WAIT).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(first);
        });
        assert!(lock_data_dir(dir.path(), Duration::from_secs(2)).is_ok());
        releaser.join().unwrap();
    }

    #[test]
    fn a_lock_error_that_is_not_a_held_lock_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let started = std::time::Instant::now();
        let err = lock_data_dir(&file.join("dots"), Duration::from_secs(5)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!err.starts_with("another Dots"), "{err}");
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
        // What the tray menu and the exit handler get to shut down.
        let state = AppState::new(Ok(rt.clone()), Some(log_dir(&data)));
        assert!(runtime_of(Some(&state)).is_some_and(|r| Arc::ptr_eq(&r, &rt)));
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

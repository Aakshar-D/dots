mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{spec, wait_status};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::settings::{Settings, TESTED_CLAUDE_VERSION};
use dots_core::{Config, Error, Runtime};
use llm::{text, Llm};
use tempfile::TempDir;

fn config(dir: &TempDir) -> Config {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    cfg
}

fn folder_spec(dir: &TempDir, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

#[tokio::test]
async fn a_taken_port_starts_without_the_server_and_fails_claude_runs() {
    let dir = tempfile::tempdir().unwrap();
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = holder.local_addr().unwrap().port();
    let mut cfg = config(&dir);
    cfg.port = taken;
    // The port stays taken for the whole wait, so the default of two seconds would only make
    // this test slow.
    cfg.bind_wait = Duration::from_millis(200);
    let rt = Runtime::start(cfg).await.unwrap();

    let status = rt.status().await;
    assert_eq!(status.port, taken);
    let err = status.server_error.expect("server error reported");
    assert!(err.contains(&taken.to_string()), "{err}");

    let dot = rt
        .create_dot(folder_spec(&dir, "cli"), None, None)
        .await
        .unwrap();
    // No URL on a port that another program owns.
    assert!(matches!(rt.webhook_url(&dot), Err(Error::Conflict(_))));
    assert!(matches!(rt.api_base_url(), Err(Error::Conflict(_))));
    let run = rt.run_now(&dot.id).await.unwrap();
    let failed = wait_status(rt.store(), &run.id, RunStatus::Failed, 15).await;
    assert_eq!(
        failed.error,
        Some(format!(
            "local server is not running: port {taken} is in use. Change the port in Settings \
             and restart Dots."
        ))
    );

    // Local dots do not need the server.
    let llm = Llm::start(vec![text("done locally")]).await;
    let mut s = folder_spec(&dir, "loco");
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "m".into();
    let local = rt.create_dot(s, None, None).await.unwrap();
    let run = rt.run_now(&local.id).await.unwrap();
    let done = wait_status(rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    assert_eq!(done.summary.as_deref(), Some("done locally"));
    rt.shutdown();
}

#[tokio::test]
async fn a_port_freed_a_moment_after_the_start_began_still_gets_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();
    // The process that "Restart Dots" replaces: it lets go of the port a moment later.
    let old_process = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        drop(holder);
    });
    let mut cfg = config(&dir);
    cfg.port = port;
    let rt = Runtime::start(cfg).await.unwrap();
    old_process.join().unwrap();

    let status = rt.status().await;
    assert_eq!((status.port, status.server_error), (port, None));
    let health = reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(health, "ok");
    rt.shutdown();
}

#[tokio::test]
async fn the_claude_version_is_checked_and_can_be_marked_tested() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.claude_env
        .push(("FAKE_CLAUDE_VERSION".into(), "2.1.300".into()));
    let rt = Runtime::start(cfg).await.unwrap();
    let claude = rt.status().await.claude;
    assert!(claude.program.is_some());
    assert_eq!(claude.version.as_deref(), Some("2.1.300"));
    assert_eq!(claude.tested_version, TESTED_CLAUDE_VERSION);
    assert!(claude.mismatch);

    let claude = rt.mark_claude_tested().await.unwrap().claude;
    assert_eq!(claude.tested_version, "2.1.300");
    assert!(!claude.mismatch);
    assert_eq!(
        rt.store()
            .get_settings()
            .await
            .unwrap()
            .tested_claude_version,
        "2.1.300"
    );
    rt.shutdown();
}

#[tokio::test]
async fn the_tested_version_matches_without_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(config(&dir)).await.unwrap();
    let claude = rt.status().await.claude;
    assert_eq!(claude.version.as_deref(), Some(TESTED_CLAUDE_VERSION));
    assert!(!claude.mismatch);
    rt.shutdown();
}

#[tokio::test]
async fn a_missing_claude_is_reported_and_fails_claude_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.claude_path = Some(PathBuf::from("Z:/nope/claude.exe"));
    let rt = Runtime::start(cfg).await.unwrap();
    let claude = rt.status().await.claude;
    assert_eq!((claude.program, claude.version), (None, None));
    assert!(!claude.mismatch);
    assert!(matches!(
        rt.mark_claude_tested().await,
        Err(Error::Invalid(_))
    ));
    let dot = rt
        .create_dot(folder_spec(&dir, "nocli"), None, None)
        .await
        .unwrap();
    let run = rt.run_now(&dot.id).await.unwrap();
    let failed = wait_status(rt.store(), &run.id, RunStatus::Failed, 15).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("claude CLI not found: install Claude Code or set its path in Settings")
    );
    rt.shutdown();
}

#[tokio::test]
async fn updating_settings_reports_when_a_restart_is_needed() {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(config(&dir)).await.unwrap();
    let start = rt.settings().await;
    assert_eq!(start, Settings::default());
    assert!(!rt.status().await.restart_required);

    let mut live = start.clone();
    live.autostart = false;
    live.local_endpoints = vec!["http://127.0.0.1:1234".into()];
    let saved = rt.update_settings(live.clone()).await.unwrap();
    assert!(!saved.restart_required);
    assert_eq!(saved.settings, live);
    assert_eq!(rt.store().get_settings().await.unwrap(), live);

    let mut moved = live.clone();
    moved.port = 40123;
    assert!(rt.update_settings(moved).await.unwrap().restart_required);
    assert!(rt.status().await.restart_required);
    // Going back to the start values needs no restart again.
    assert!(!rt.update_settings(live).await.unwrap().restart_required);

    let mut bad = start.clone();
    bad.max_concurrent_runs = 0;
    assert!(matches!(
        rt.update_settings(bad).await,
        Err(Error::Invalid(_))
    ));
    rt.shutdown();
}

#[tokio::test]
async fn saving_settings_keeps_the_pause_and_the_tested_version() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.claude_env
        .push(("FAKE_CLAUDE_VERSION".into(), "2.1.300".into()));
    let rt = Runtime::start(cfg).await.unwrap();
    // A settings form loads its copy; then the tray pauses and the CLI is marked as tested.
    let mut form = rt.settings().await;
    rt.set_schedules_paused(true).await.unwrap();
    rt.mark_claude_tested().await.unwrap();

    form.autostart = false;
    let saved = rt.update_settings(form).await.unwrap().settings;
    assert!(!saved.autostart);
    assert!(saved.schedules_paused);
    assert_eq!(saved.tested_claude_version, "2.1.300");
    assert_eq!(rt.store().get_settings().await.unwrap(), saved);
    assert!(rt.status().await.schedules_paused);
    rt.shutdown();
}

#[tokio::test]
async fn paused_schedules_fire_nothing_until_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(config(&dir)).await.unwrap();
    assert!(
        rt.set_schedules_paused(true)
            .await
            .unwrap()
            .schedules_paused
    );
    assert!(rt.settings().await.schedules_paused);
    let mut s = folder_spec(&dir, "ticker");
    s.schedule = Some(Schedule::cron("* * * * * *"));
    let dot = rt.create_dot(s, None, None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(rt
        .store()
        .list_runs(Some(&dot.id), 10)
        .await
        .unwrap()
        .is_empty());

    assert!(
        !rt.set_schedules_paused(false)
            .await
            .unwrap()
            .schedules_paused
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rt
        .store()
        .list_runs(Some(&dot.id), 10)
        .await
        .unwrap()
        .is_empty()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "no run after resuming"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    rt.shutdown();
}

#[tokio::test]
async fn pause_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(config(&dir)).await.unwrap();
    rt.set_schedules_paused(true).await.unwrap();
    rt.shutdown();
    drop(rt);
    let rt = Runtime::start(config(&dir)).await.unwrap();
    assert!(rt.status().await.schedules_paused);
    rt.shutdown();
}

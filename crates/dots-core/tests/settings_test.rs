mod common;

use common::temp_store;
use dots_core::settings::{self, Settings, TESTED_CLAUDE_VERSION};
use dots_core::store::Store;
use dots_core::Config;

#[tokio::test]
async fn an_empty_table_gives_the_defaults() {
    let (_dir, store) = temp_store().await;
    let s = store.get_settings().await.unwrap();
    assert_eq!(s, Settings::default());
    assert_eq!(s.port, 47321);
    assert_eq!(s.max_concurrent_runs, 2);
    assert_eq!(s.claude_path, None);
    assert_eq!(s.tested_claude_version, TESTED_CLAUDE_VERSION);
    assert!(s.local_endpoints.is_empty());
    assert!(s.autostart);
    assert!(!s.schedules_paused);
}

#[tokio::test]
async fn settings_round_trip() {
    let (_dir, store) = temp_store().await;
    let s = Settings {
        port: 50000,
        max_concurrent_runs: 4,
        claude_path: Some("C:/tools/claude.exe".into()),
        tested_claude_version: "2.2.0".into(),
        local_endpoints: vec!["http://127.0.0.1:1234".into()],
        autostart: false,
        schedules_paused: true,
    };
    store.put_settings(&s).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), s);
    // A second write replaces the values.
    let back = Settings::default();
    store.put_settings(&back).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), back);
}

#[tokio::test]
async fn bad_and_unknown_rows_are_ignored_one_by_one() {
    let (dir, store) = temp_store().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    for (k, v) in [
        ("port", "\"not a number\""),
        ("max_concurrent_runs", "5"),
        ("autostart", "{broken json"),
        ("theme", "\"dark\""),
        // Right type, but a value `validate` rejects.
        ("claude_path", "\"\""),
        ("tested_claude_version", "\"\""),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(k)
            .bind(v)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
    let s = store.get_settings().await.unwrap();
    assert_eq!(s.port, 47321);
    assert_eq!(s.max_concurrent_runs, 5);
    assert!(s.autostart);
    assert_eq!(s.claude_path, None);
    assert_eq!(s.tested_claude_version, TESTED_CLAUDE_VERSION);
}

#[tokio::test]
async fn invalid_settings_are_rejected_and_not_stored() {
    let (_dir, store) = temp_store().await;
    type Case = (fn(&mut Settings), &'static str);
    let cases: Vec<Case> = vec![
        (|s| s.port = 0, "port"),
        (|s| s.max_concurrent_runs = 0, "max_concurrent_runs"),
        (|s| s.max_concurrent_runs = 17, "max_concurrent_runs"),
        (|s| s.claude_path = Some("  ".into()), "claude_path"),
        (
            |s| s.tested_claude_version = String::new(),
            "tested_claude_version",
        ),
        (
            |s| s.local_endpoints = vec!["https://example.com".into()],
            "https",
        ),
    ];
    for (mutate, needle) in cases {
        let mut s = Settings::default();
        mutate(&mut s);
        let err = store.put_settings(&s).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{err}");
    }
    assert_eq!(store.get_settings().await.unwrap(), Settings::default());
}

#[test]
fn only_port_concurrency_and_claude_path_need_a_restart() {
    let base = Settings::default();
    let mut s = base.clone();
    s.autostart = false;
    s.schedules_paused = true;
    s.tested_claude_version = "9.9.9".into();
    s.local_endpoints = vec!["http://127.0.0.1:1".into()];
    assert!(!base.restart_required(&s));
    for mutate in [
        (|s: &mut Settings| s.port = 1) as fn(&mut Settings),
        |s| s.max_concurrent_runs = 3,
        |s| s.claude_path = Some("claude.exe".into()),
    ] {
        let mut s = base.clone();
        mutate(&mut s);
        assert!(base.restart_required(&s));
    }
}

#[tokio::test]
async fn load_reads_the_settings_of_a_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let store = Store::open(&data.join("dots.db")).await.unwrap();
    let s = Settings {
        port: 40001,
        ..Settings::default()
    };
    store.put_settings(&s).await.unwrap();
    store.close().await;
    assert_eq!(settings::load(&data).await.unwrap().port, 40001);
    // A data dir without a database yet gives the defaults.
    let fresh = dir.path().join("fresh");
    assert_eq!(settings::load(&fresh).await.unwrap(), Settings::default());
}

#[test]
fn config_from_settings_takes_port_concurrency_and_claude_path() {
    let s = Settings {
        port: 40002,
        max_concurrent_runs: 3,
        claude_path: Some("C:/tools/claude.exe".into()),
        ..Settings::default()
    };
    let cfg = Config::from_settings("C:/data", &s);
    assert_eq!(cfg.port, 40002);
    assert_eq!(cfg.max_concurrent_runs, 3);
    assert_eq!(
        cfg.claude_path.as_deref(),
        Some(std::path::Path::new("C:/tools/claude.exe"))
    );
    assert_eq!(cfg.data_dir, std::path::PathBuf::from("C:/data"));
}

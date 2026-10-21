//! A dot's browser switch and the Edge path of Settings (browser spec section 6): migration
//! 0009's column, its round trip, a spec stored before it, and the setting's checks.

mod common;

use common::{spec, temp_store};
use dots_core::model::DotSpec;
use dots_core::policy::{Policy, Preset};
use dots_core::settings::Settings;
use dots_core::store::Store;
use dots_core::Error;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

#[tokio::test]
async fn a_dot_keeps_its_browser_switch() {
    let (_dir, store) = temp_store().await;
    let plain = store.create_dot(&spec("plain")).await.unwrap();
    assert!(!plain.spec.browser, "a new dot has no browser");
    let mut s = spec("shopper");
    s.browser = true;
    let shopper = store.create_dot(&s).await.unwrap();
    assert!(shopper.spec.browser);
    assert!(store.get_dot(&shopper.id).await.unwrap().spec.browser);
    let on: Vec<String> = store
        .list_dots()
        .await
        .unwrap()
        .into_iter()
        .filter(|d| d.spec.browser)
        .map(|d| d.spec.name)
        .collect();
    assert_eq!(on, ["shopper"]);

    let mut off = shopper.spec.clone();
    off.browser = false;
    assert!(
        !store
            .update_dot(&shopper.id, &off)
            .await
            .unwrap()
            .spec
            .browser
    );
    let mut on_again = plain.spec.clone();
    on_again.browser = true;
    assert!(
        store
            .update_dot(&plain.id, &on_again)
            .await
            .unwrap()
            .spec
            .browser
    );
}

#[test]
fn a_spec_written_before_the_switch_reads_as_off() {
    let mut value = serde_json::to_value(DotSpec::new("old", "Do it.", "C:/repo")).unwrap();
    assert_eq!(value["browser"], false);
    value.as_object_mut().unwrap().remove("browser");
    let read: DotSpec = serde_json::from_value(value).unwrap();
    assert!(!read.browser);
}

#[tokio::test]
async fn migration_0009_gives_every_stored_dot_the_switch_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 9)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
         webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
         created_at, updated_at) VALUES ('D1', 'old', 'Do it.', 'claude', 'sonnet', \
         'C:/tmp/repo', 'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(serde_json::to_string(&Policy::preset(Preset::Sandboxed)).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    assert!(!store.get_dot("D1").await.unwrap().spec.browser);
}

#[tokio::test]
async fn the_edge_path_is_unset_by_default_and_kept_when_set() {
    let (_dir, store) = temp_store().await;
    assert_eq!(store.get_settings().await.unwrap().edge_path, None);
    let s = Settings {
        edge_path: Some("D:/Apps/Edge/msedge.exe".into()),
        ..Settings::default()
    };
    store.put_settings(&s).await.unwrap();
    assert_eq!(
        store.get_settings().await.unwrap().edge_path.as_deref(),
        Some("D:/Apps/Edge/msedge.exe")
    );
}

#[test]
fn an_edge_path_must_name_something_and_show_as_it_is() {
    let with = |p: &str| Settings {
        edge_path: Some(p.into()),
        ..Settings::default()
    };
    assert!(with("C:/Edge/msedge.exe").validate().is_ok());
    let blank = with("  ").validate().unwrap_err();
    assert!(
        matches!(&blank, Error::Invalid(m) if m == "edge_path must not be empty (leave it unset to use the installed Microsoft Edge)"),
        "{blank}"
    );
    // In a dialog this reads "C:/Edge/msedge.exe": the override turns the rest around.
    let hidden = with("C:/Edge/\u{202e}exe.egdesm").validate().unwrap_err();
    assert!(
        matches!(&hidden, Error::Invalid(m) if m.starts_with("edge_path must not contain invisible or text-direction characters")),
        "{hidden}"
    );
}

//! The `chat_attachments` rows (attachments spec sections 4.1, 4.2 and 4.7): removing a staged
//! file, the startup sweep, deleting a dot, and the dot check when staging.

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::chat_files::{ChatFiles, DIR};
use dots_core::model::{NewRun, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use tempfile::TempDir;

async fn runtime(dir: &TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    rt.hold_runs(true).await;
    rt
}

async fn folder_dot(rt: &Runtime, dir: &TempDir) -> dots_core::model::Dot {
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut s = spec("courier");
    s.workdir = work.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    rt.create_dot(s, None, None).await.unwrap()
}

fn files_root(dir: &TempDir) -> PathBuf {
    dir.path().join("data").join(DIR)
}

#[tokio::test]
async fn a_staged_file_can_be_removed_until_it_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let dot = folder_dot(&rt, &dir).await;
    let store = rt.store();
    store
        .insert_staged_attachment("A1", &dot.id, "a.txt", 1)
        .await
        .unwrap();
    assert!(store.remove_unsent_attachment("A1").await.unwrap());
    assert!(matches!(
        store.get_attachment("A1").await,
        Err(Error::NotFound(_))
    ));

    store
        .insert_staged_attachment("A2", &dot.id, "b.txt", 1)
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(serde_json::json!({"message": "hi"}));
    store
        .create_chat_run(&new, &[("A2".into(), "b.txt".into())], None, None)
        .await
        .unwrap();
    assert!(!store.remove_unsent_attachment("A2").await.unwrap());
    // Through the runtime: a sent file is a conflict, and the row stays.
    let e = rt.remove_attachment("A2").await.unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");
    assert!(store.get_attachment("A2").await.is_ok());
}

#[tokio::test]
async fn removing_a_staged_file_through_the_runtime_removes_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let dot = folder_dot(&rt, &dir).await;
    let staged = rt
        .add_attachment_bytes(&dot.id, "shot.png", b"png")
        .await
        .unwrap();
    assert_eq!(staged.name, "shot.png");
    assert_eq!(staged.size, 3);
    let folder = files_root(&dir).join(&staged.id);
    assert!(folder.join("shot.png").is_file());
    rt.remove_attachment(&staged.id).await.unwrap();
    assert!(!folder.exists());
}

#[tokio::test]
async fn the_sweep_removes_unsent_files_and_stray_folders() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let dot = folder_dot(&rt, &dir).await;
    let unsent = rt
        .add_attachment_bytes(&dot.id, "a.txt", b"a")
        .await
        .unwrap();
    let sent = rt
        .add_attachment_bytes(&dot.id, "b.txt", b"b")
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(serde_json::json!({"message": "hi"}));
    rt.store()
        .create_chat_run(&new, &[(sent.id.clone(), "b.txt".into())], None, None)
        .await
        .unwrap();
    let stray = files_root(&dir).join("STRAY");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("x"), "x").unwrap();

    ChatFiles::new(files_root(&dir)).sweep(rt.store()).await;

    assert!(matches!(
        rt.store().get_attachment(&unsent.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(!files_root(&dir).join(&unsent.id).exists());
    assert!(!stray.exists());
    assert!(rt.store().get_attachment(&sent.id).await.is_ok());
    assert!(files_root(&dir).join(&sent.id).join("b.txt").is_file());
}

/// Dots stopped between cancelling a scheduled message and releasing its files: the sweep
/// releases them. A message still scheduled keeps its files.
#[tokio::test]
async fn the_sweep_releases_the_files_of_a_cancelled_scheduled_message() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let dot = folder_dot(&rt, &dir).await;
    let held = rt
        .add_attachment_bytes(&dot.id, "a.txt", b"a")
        .await
        .unwrap();
    let kept = rt
        .add_attachment_bytes(&dot.id, "b.txt", b"b")
        .await
        .unwrap();
    let store = rt.store();
    let now = chrono::Utc::now();
    let due = (now + chrono::Duration::hours(1)).to_rfc3339();
    let cancelled = store
        .insert_scheduled_with_files(&dot.id, None, "", &due, now, &[held.id.clone()])
        .await
        .unwrap();
    store
        .insert_scheduled_with_files(&dot.id, None, "", &due, now, &[kept.id.clone()])
        .await
        .unwrap();
    // Cancelled in the store only: the release that follows never ran.
    store
        .cancel_scheduled_message(&cancelled.id, now)
        .await
        .unwrap()
        .unwrap();

    ChatFiles::new(files_root(&dir)).sweep(store).await;

    assert!(matches!(
        store.get_attachment(&held.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(!files_root(&dir).join(&held.id).exists());
    assert!(store.get_attachment(&kept.id).await.is_ok());
    assert!(files_root(&dir).join(&kept.id).join("b.txt").is_file());
}

#[tokio::test]
async fn deleting_a_dot_removes_its_stored_files() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let dot = folder_dot(&rt, &dir).await;
    let staged = rt
        .add_attachment_bytes(&dot.id, "a.txt", b"a")
        .await
        .unwrap();
    assert!(files_root(&dir).join(&staged.id).exists());
    rt.delete_dot(&dot.id).await.unwrap();
    assert!(matches!(
        rt.store().get_attachment(&staged.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(!files_root(&dir).join(&staged.id).exists());
}

#[tokio::test]
async fn staging_checks_the_dot() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let e = rt
        .add_attachment_bytes("NODOT", "a.txt", b"a")
        .await
        .unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
    let from = dir.path().join("a.txt");
    std::fs::write(&from, "a").unwrap();
    let e = rt.add_attachment_path("NODOT", &from).await.unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
    let made = std::fs::read_dir(files_root(&dir))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(made, 0);
}

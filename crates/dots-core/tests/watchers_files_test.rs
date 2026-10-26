//! Attachments of email watcher runs (watchers spec section 4.2): names made safe for Windows,
//! the copy into a run's workspace with the payload's paths, its removal, the staged copy kept
//! while the thread has a run to come, and the sweep at start.

mod common;

use common::{spec, temp_store};
use dots_core::model::{EmailConfig, NewRun, RunStatus, TriggerKind, WatcherConfig};
use dots_core::watchers::files::{safe_name, unique_names, WatcherFiles};
use serde_json::json;

#[test]
fn attachment_names_are_made_safe_for_windows() {
    for (raw, safe) in [
        ("../../evil.pdf", "evil.pdf"),
        ("C:\\x\\y.txt", "y.txt"),
        ("CON", "_CON"),
        ("con.txt", "_con.txt"),
        ("LPT1.log", "_LPT1.log"),
        ("a<b>:c|d?.pdf", "a_b__c_d_.pdf"),
        ("  report. ", "report"),
        ("", "attachment"),
        ("\u{7}bell.txt", "bell.txt"),
    ] {
        assert_eq!(safe_name(raw), safe, "{raw:?}");
    }
    let cut = safe_name(&format!("{}.pdf", "a".repeat(200)));
    assert_eq!(cut.chars().count(), 120);
    assert!(cut.ends_with(".pdf"));
    assert_eq!(
        unique_names(&[
            "a.pdf".into(),
            "a.pdf".into(),
            "A.PDF".into(),
            "b".into(),
            "b".into()
        ]),
        ["a.pdf", "a (2).pdf", "A (3).PDF", "b", "b (2)"]
    );
}

#[tokio::test]
async fn the_copy_fills_the_payload_paths_and_is_removed_whole() {
    let (dir, store) = temp_store().await;
    let files = WatcherFiles::new(dir.path().join("watchers"));
    let dot = store.create_dot(&spec("mail")).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.payload = Some(
        json!({"source": "email", "watcher_id": "W1", "attachments": [
        {"name": "a.pdf", "size": 1, "path": null},
        {"name": "big.bin", "size": 99, "skipped": "too large"}]}),
    );
    let run = store.create_run(&new).await.unwrap();
    files
        .stage("W1", &run.id, &[("a.pdf".into(), b"x".to_vec())])
        .unwrap();
    let workspace = dir.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let copy = workspace.join("attachments").join(&run.id).join("a.pdf");
    let payload = files.copy_into(&run, &workspace).unwrap().unwrap();
    assert_eq!(
        payload["attachments"],
        json!([{"name": "a.pdf", "size": 1, "path": copy.to_string_lossy()},
               {"name": "big.bin", "size": 99, "skipped": "too large"}])
    );
    assert_eq!(std::fs::read(&copy).unwrap(), b"x");
    let stored = store.set_run_payload(&run.id, &payload).await.unwrap();
    assert_eq!(stored.payload, Some(payload));
    WatcherFiles::remove_copy(&workspace, &run.id);
    assert!(!workspace.join("attachments").exists());
    assert!(workspace.is_dir());
    let plain = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(files.copy_into(&plain, &workspace).unwrap(), None);
}

#[tokio::test]
async fn the_staged_copy_stays_while_the_thread_has_a_run_to_come() {
    let (dir, store) = temp_store().await;
    let files = WatcherFiles::new(dir.path().join("watchers"));
    let dot = store.create_dot(&spec("mail")).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.payload = Some(json!({"source": "email", "watcher_id": "W1", "attachments": []}));
    let root = store.create_run(&new).await.unwrap();
    files
        .stage("W1", &root.id, &[("a.pdf".into(), b"x".to_vec())])
        .unwrap();
    let staged = files.stage_dir("W1", &root.id);
    let mut child = NewRun::new(&dot.id, TriggerKind::Resume);
    child.parent_run_id = Some(root.id.clone());
    child.root_run_id = Some(root.id.clone());
    let child = store.create_run(&child).await.unwrap();
    store
        .finish_run(&root.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    files.release(&store, &root.id).await;
    assert!(staged.is_dir(), "the resume is still queued");
    store
        .finish_run(&child.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    files.release(&store, &child.id).await;
    assert!(!staged.exists());
}

#[tokio::test]
async fn the_sweep_removes_what_no_watcher_or_open_thread_needs() {
    let (dir, store) = temp_store().await;
    let files = WatcherFiles::new(dir.path().join("watchers"));
    let dot = store.create_dot(&spec("mail")).await.unwrap();
    let w = store
        .add_watcher(
            &dot.id,
            "Mail",
            &WatcherConfig::Email(EmailConfig::new("me@gmail.com", "GMAIL_APP_PASSWORD")),
        )
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.payload = Some(json!({"watcher_id": w.id}));
    let open = store.create_run(&new).await.unwrap();
    let done = store.create_run(&new).await.unwrap();
    store
        .finish_run(&done.id, RunStatus::Failed, None, Some("x"))
        .await
        .unwrap();
    for (watcher, run) in [
        (w.id.as_str(), open.id.as_str()),
        (w.id.as_str(), done.id.as_str()),
        (w.id.as_str(), "GONE"),
        ("NOWATCHER", "R"),
    ] {
        files
            .stage(watcher, run, &[("a".into(), b"x".to_vec())])
            .unwrap();
    }
    files.sweep(&store).await;
    assert!(files.stage_dir(&w.id, &open.id).is_dir());
    assert!(!files.stage_dir(&w.id, &done.id).exists());
    assert!(!files.stage_dir(&w.id, "GONE").exists());
    assert!(!dir.path().join("watchers").join("NOWATCHER").exists());
}

/// Deferred minor: the device names Windows also reserves, a reserved name before any
/// extension, the characters that reverse how a name reads, and a suffix that keeps 120.
#[test]
fn more_names_windows_or_people_misread_are_made_safe() {
    for (raw, safe) in [
        ("CONIN$", "_CONIN$"),
        ("conout$.txt", "_conout$.txt"),
        ("CON.tar.gz", "_CON.tar.gz"),
        ("invoice\u{202E}fdp.exe", "invoicefdp.exe"),
        ("a\u{2066}b\u{200F}.pdf", "ab.pdf"),
    ] {
        assert_eq!(safe_name(raw), safe, "{raw:?}");
    }
    let long = format!("{}.pdf", "a".repeat(116));
    let names = unique_names(&[long.clone(), long]);
    assert!(names.iter().all(|n| n.chars().count() <= 120), "{names:?}");
    assert_ne!(names[0].to_lowercase(), names[1].to_lowercase());
    assert!(names[1].ends_with(" (2).pdf"), "{}", names[1]);
}

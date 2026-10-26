//! Folder watchers (watchers spec section 4.1) on a real temp folder with the OS's notifications:
//! each file wakes the dot once it has settled, as created, changed or deleted; patterns, events,
//! subfolders and the skip list filter; the dot's acting span drops what is reported in it; a
//! sync engine's touches and a vanished folder are handled (Review Focus 1 and 2).

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::watchers::{after, Harness};
use dots_core::model::{Dot, FolderConfig, FolderEvent, PausedReason, Run, Watcher, WatcherConfig};
use dots_core::watchers::folder::{matches_pattern, skipped};
use dots_core::watchers::folder_gone;
use serde_json::{json, Value};

async fn watch(h: &Harness, dot: &Dot, root: &Path, f: impl FnOnce(&mut FolderConfig)) -> Watcher {
    let mut config = FolderConfig::new(&Harness::path_text(root));
    f(&mut config);
    let w = h
        .watcher(dot, "Invoices folder", WatcherConfig::Folder(config))
        .await;
    h.watching(&w.id).await;
    w
}

/// (event, relative, size) of a run.
fn what(run: &Run) -> (String, String, Value) {
    let p = run.payload.clone().unwrap();
    (
        p["event"].as_str().unwrap().to_string(),
        p["relative"].as_str().unwrap().to_string(),
        p["size"].clone(),
    )
}

async fn started() -> (Harness, Dot, PathBuf) {
    let h = Harness::new().await;
    h.start();
    let dot = h.dot("invoices").await;
    let root = h.folder("in");
    (h, dot, root)
}

#[tokio::test]
async fn new_changed_and_deleted_files_each_wake_the_dot_once_they_settle() {
    let (h, dot, root) = started().await;
    let w = watch(&h, &dot, &root, |_| {}).await;
    std::fs::write(root.join("a.pdf"), b"abc").unwrap();
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(
        runs[0].payload,
        Some(json!({"source": "folder", "event": "created",
            "path": Harness::path_text(&root.join("a.pdf")), "relative": "a.pdf", "size": 3,
            "watcher_id": w.id, "watcher_name": "Invoices folder"}))
    );
    std::fs::write(root.join("a.pdf"), b"abcdef").unwrap();
    let runs = h.wait_runs(&dot.id, 2, 10).await;
    assert_eq!(what(&runs[1]), ("changed".into(), "a.pdf".into(), json!(6)));
    std::fs::remove_file(root.join("a.pdf")).unwrap();
    let runs = h.wait_runs(&dot.id, 3, 10).await;
    assert_eq!(
        what(&runs[2]),
        ("deleted".into(), "a.pdf".into(), Value::Null)
    );
    h.still(&dot.id, 3, 3).await;
}

#[tokio::test]
async fn a_file_written_in_pieces_wakes_it_once_when_complete() {
    let (h, dot, root) = started().await;
    watch(&h, &dot, &root, |_| {}).await;
    let mut file = std::fs::File::create(root.join("big.pdf")).unwrap();
    for _ in 0..5 {
        file.write_all(&[0u8; 1000]).unwrap();
        file.flush().unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    drop(file);
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(
        what(&runs[0]),
        ("created".into(), "big.pdf".into(), json!(5000))
    );
    h.still(&dot.id, 1, 3).await;
}

#[tokio::test]
async fn only_matching_names_and_chosen_events_wake_it() {
    let (h, dot, root) = started().await;
    watch(&h, &dot, &root, |c| {
        c.pattern = "*.pdf".into();
        c.events = vec![FolderEvent::Created];
    })
    .await;
    std::fs::write(root.join("A.PDF"), b"x").unwrap();
    std::fs::write(root.join("b.txt"), b"x").unwrap();
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(what(&runs[0]).1, "A.PDF");
    std::fs::remove_file(root.join("A.PDF")).unwrap();
    h.still(&dot.id, 1, 4).await;
}

#[tokio::test]
async fn subfolders_count_only_when_recursive() {
    let (h, deep, root) = started().await;
    let flat = h.dot("flat").await;
    std::fs::create_dir_all(root.join("sub")).unwrap();
    watch(&h, &deep, &root, |_| {}).await;
    watch(&h, &flat, &root, |c| c.recursive = false).await;
    std::fs::write(root.join("sub").join("c.pdf"), b"x").unwrap();
    let runs = h.wait_runs(&deep.id, 1, 10).await;
    assert_eq!(
        what(&runs[0]).1,
        Harness::path_text(&Path::new("sub").join("c.pdf"))
    );
    h.still(&flat.id, 0, 3).await;
}

#[tokio::test]
async fn partial_downloads_temp_files_hidden_names_and_folders_never_wake_it() {
    let (h, dot, root) = started().await;
    std::fs::create_dir_all(root.join(".git")).unwrap();
    watch(&h, &dot, &root, |_| {}).await;
    for name in [
        "x.crdownload",
        "x.part",
        "x.tmp",
        "x.partial",
        "~$doc.docx",
        ".~lock.doc#",
        ".hidden",
    ] {
        std::fs::write(root.join(name), b"x").unwrap();
    }
    std::fs::write(root.join(".git").join("index"), b"x").unwrap();
    std::fs::create_dir(root.join("newdir")).unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    std::fs::remove_dir(root.join("newdir")).unwrap();
    h.still(&dot.id, 0, 4).await;
    // It is watching all the same.
    std::fs::write(root.join("ok.pdf"), b"x").unwrap();
    assert_eq!(what(&h.wait_runs(&dot.id, 1, 10).await[0]).1, "ok.pdf");
}

#[tokio::test]
async fn a_rename_is_a_deletion_and_a_creation() {
    let (h, dot, root) = started().await;
    watch(&h, &dot, &root, |_| {}).await;
    std::fs::write(root.join("a.pdf"), b"x").unwrap();
    h.wait_runs(&dot.id, 1, 10).await;
    std::fs::rename(root.join("a.pdf"), root.join("b.pdf")).unwrap();
    let runs = h.wait_runs(&dot.id, 3, 10).await;
    let mut seen: Vec<(String, String)> = runs[1..]
        .iter()
        .map(|r| {
            let (event, relative, _) = what(r);
            (event, relative)
        })
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [
            ("created".to_string(), "b.pdf".to_string()),
            ("deleted".into(), "a.pdf".into())
        ]
    );
    h.still(&dot.id, 3, 3).await;
}

#[tokio::test]
async fn nothing_wakes_it_while_the_dot_acts_or_for_five_seconds_after() {
    let (h, dot, root) = started().await;
    watch(&h, &dot, &root, |_| {}).await;
    h.acting.begin(&dot.id);
    std::fs::write(root.join("a.pdf"), b"x").unwrap();
    h.still(&dot.id, 0, 3).await;
    // The clock stands at T0: the run ended at T0, so T0 is within the 5 seconds after.
    h.acting.end(&dot.id);
    std::fs::write(root.join("b.pdf"), b"x").unwrap();
    h.still(&dot.id, 0, 3).await;
    h.clock.set(after(6));
    std::fs::write(root.join("c.pdf"), b"x").unwrap();
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(what(&runs[0]).1, "c.pdf");
    h.still(&dot.id, 1, 3).await;
}

// The touch is the read-only attribute set and cleared, as a sync engine does on Windows.
#[allow(clippy::permissions_set_readonly_false)]
#[tokio::test]
async fn a_file_touched_again_by_a_sync_engine_wakes_the_dot_once() {
    let (h, dot, root) = started().await;
    watch(&h, &dot, &root, |_| {}).await;
    let path = root.join("a.pdf");
    std::fs::write(&path, b"abc").unwrap();
    h.wait_runs(&dot.id, 1, 10).await;
    // What a sync engine does to a settled file: its attributes, then the same bytes put back
    // with the same modified time.
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms.clone()).unwrap();
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
    std::fs::write(&path, b"abc").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    h.still(&dot.id, 1, 4).await;
    // A real change still counts.
    std::fs::write(&path, b"abcdef").unwrap();
    let runs = h.wait_runs(&dot.id, 2, 10).await;
    assert_eq!(what(&runs[1]), ("changed".into(), "a.pdf".into(), json!(6)));
}

#[tokio::test]
async fn a_watched_folder_that_disappears_pauses_its_watcher() {
    let (mut h, dot, root) = started().await;
    let w = watch(&h, &dot, &root, |_| {}).await;
    std::fs::rename(&root, h.dir.path().join("in-old")).unwrap();
    let sentence = folder_gone(&Harness::path_text(&root));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let now = h.store.get_watcher(&w.id).await.unwrap();
        if now.paused_reason == Some(PausedReason::Error) {
            assert_eq!(now.last_error.as_deref(), Some(sentence.as_str()));
            break;
        }
        assert!(std::time::Instant::now() < deadline, "not paused: {now:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The event follows the row's write; give it a moment under load.
    let mut announced = Vec::new();
    for _ in 0..50 {
        announced.extend(h.paused());
        if !announced.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(announced, vec![(w.id.clone(), Some(sentence))]);
    assert!(!h.watchers.running().contains(&w.id));
}

#[test]
fn the_skip_list_and_the_pattern() {
    for name in [
        "a.crdownload",
        "a.PART",
        "a.tmp",
        "a.partial",
        "~$report.docx",
        ".~lock.report.odt#",
        ".hidden",
        "sub/.git/x",
    ] {
        assert!(skipped(Path::new(name)), "{name}");
    }
    for name in [
        "a.pdf",
        "report.docx",
        "sub/a.pdf",
        "tmp.pdf",
        "partial-invoice.pdf",
    ] {
        assert!(!skipped(Path::new(name)), "{name}");
    }
    assert!(matches_pattern("", "x.bin"));
    assert!(matches_pattern("*.pdf", "INVOICE.PDF"));
    assert!(!matches_pattern("*.pdf", "a.pdf.txt"));
    assert!(matches_pattern("invoice-*.*", "invoice-12.pdf"));
    assert!(!matches_pattern("invoice-*.*", "bill-12.pdf"));
}

//! Microsoft Edge for dots (browser spec section 2.1) through `EdgeHost`, with `fake-browser`
//! in Edge's place: the switches and profile of a start, a second open that finds it (also
//! from a Dots that started again), the pages it lists, closing it, a window the person
//! closed, a profile another Edge holds, an Edge that is not there, and moving downloads.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::fake_browser::{exe, FakeEdges};
use common::playwright::JSON_LIST;
use dots_core::browser::host::{
    edge_args, edge_candidates, find_edge, move_downloads, page_urls, BrowserHost, EdgeHost,
    HostError, Opened, EDGE_FLAGS,
};
use serde_json::Value;

/// Dropped in this order: the fakes quit, then the folder goes.
struct Env {
    edges: FakeEdges,
    host: EdgeHost,
    root: PathBuf,
    dir: tempfile::TempDir,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("browser");
    Env {
        edges: FakeEdges::new(&root),
        host: EdgeHost::new(&root),
        root,
        dir,
    }
}

fn edge() -> PathBuf {
    PathBuf::from(exe())
}

#[tokio::test]
async fn edge_starts_on_the_dots_profile_with_its_switches() {
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    assert!(opened.started);
    let starts = e.edges.starts("D1");
    assert_eq!(starts.len(), 1);
    let args: Vec<String> = serde_json::from_value(starts[0]["args"].clone()).unwrap();
    assert_eq!(args, edge_args(&e.host.profile("D1"), opened.port));
    assert_eq!(
        args[0],
        format!(
            "--user-data-dir={}",
            e.root.join("D1").join("profile").display()
        )
    );
    assert_eq!(args[1], "--remote-debugging-address=127.0.0.1");
    assert_eq!(args[2], format!("--remote-debugging-port={}", opened.port));
    assert_eq!(args[3..10], EDGE_FLAGS);
    assert_eq!(args[10], "about:blank");
    // Downloads go to the dot's own folder, without a question.
    let prefs: Value = serde_json::from_str(
        &std::fs::read_to_string(e.host.profile("D1").join("Default").join("Preferences")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        prefs["download"]["default_directory"].as_str(),
        Some(e.host.downloads("D1").to_str().unwrap())
    );
    assert_eq!(prefs["download"]["prompt_for_download"], false);
    assert!(e.host.downloads("D1").is_dir());
    // Its pages: the blank page; the extension's service worker is not one.
    assert_eq!(e.host.pages(opened.port).await.unwrap(), ["about:blank"]);
}

#[tokio::test]
async fn a_second_open_finds_the_running_edge() {
    let e = env();
    let first = e.host.open("D1", &edge()).await.unwrap();
    let again = e.host.open("D1", &edge()).await.unwrap();
    assert_eq!(
        again,
        Opened {
            port: first.port,
            started: false
        }
    );
    assert_eq!(e.edges.starts("D1").len(), 1);
    // A Dots that starts again finds it through the port it kept.
    let restarted = EdgeHost::new(&e.root);
    assert_eq!(
        restarted.open("D1", &edge()).await.unwrap(),
        Opened {
            port: first.port,
            started: false
        }
    );
    // Another dot gets an Edge of its own.
    let other = e.host.open("D2", &edge()).await.unwrap();
    assert_ne!(other.port, first.port);
    assert!(e.edges.running("D1") && e.edges.running("D2"));
}

#[tokio::test]
async fn closing_ends_it_and_a_closed_window_starts_again() {
    let e = env();
    e.host.open("D1", &edge()).await.unwrap();
    e.host.close("D1").await;
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert!(e.host.open("D1", &edge()).await.unwrap().started);
    // The person closes the window: the next open starts Edge again on the same profile.
    e.edges.quit("D1");
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert!(e.host.open("D1", &edge()).await.unwrap().started);
    assert_eq!(e.edges.starts("D1").len(), 3);
    // Closing an Edge that does not run does nothing.
    e.host.close("D9").await;
}

#[tokio::test]
async fn its_pages_are_the_page_targets_of_its_list() {
    let targets: Vec<Value> = serde_json::from_str(JSON_LIST).unwrap();
    assert_eq!(
        page_urls(&targets),
        [
            "edge://sync-confirmation-dialog/",
            "http://127.0.0.1:54428/cart"
        ]
    );
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    e.edges
        .set_pages("D1", &["https://shop.com/other", "https://shop.com/cart"]);
    assert_eq!(
        e.host.pages(opened.port).await.unwrap(),
        ["https://shop.com/other", "https://shop.com/cart"]
    );
}

#[tokio::test]
async fn a_profile_held_by_another_edge_is_refused() {
    let e = env();
    // An Edge started outside Dots on the dot's profile: the fake, by hand.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut outside = tokio::process::Command::new(exe());
    outside
        .arg(format!(
            "--user-data-dir={}",
            e.host.profile("D1").display()
        ))
        .arg(format!("--remote-debugging-port={port}"))
        .kill_on_drop(true);
    dots_core::proc::hide_window(&mut outside);
    let _outside = outside.spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !e.edges.running("D1") {
        assert!(
            std::time::Instant::now() < deadline,
            "the outside fake did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        e.host.open("D1", &edge()).await,
        Err(HostError::ProfileInUse)
    );
    assert_eq!(
        HostError::ProfileInUse.to_string(),
        "This dot's browser profile is in use by another Edge window; close it and try again."
    );
}

#[tokio::test]
async fn an_edge_that_is_not_there_is_not_found() {
    let e = env();
    let missing = e.dir.path().join("no-edge.exe");
    assert_eq!(e.host.open("D1", &missing).await, Err(HostError::NotFound));
    assert_eq!(
        HostError::NotFound.to_string(),
        "Microsoft Edge was not found; browser use needs it (Settings → Edge path)."
    );
    let installed = |key: &str| match key {
        "ProgramFiles(x86)" => Some("C:\\PF86".to_string()),
        "ProgramFiles" => Some("C:\\PF".to_string()),
        _ => None,
    };
    let under = |base: &str| {
        Path::new(base)
            .join("Microsoft")
            .join("Edge")
            .join("Application")
            .join("msedge.exe")
    };
    assert_eq!(
        edge_candidates(None, &installed),
        [under("C:\\PF86"), under("C:\\PF")]
    );
    assert_eq!(edge_candidates(Some("  "), &installed).len(), 2);
    assert_eq!(
        edge_candidates(Some(" D:/Edge/msedge.exe "), &installed),
        [PathBuf::from("D:/Edge/msedge.exe")]
    );
    assert_eq!(find_edge(Some(exe().as_str()), &installed), Some(edge()));
    // A path that is set but missing is not found: no fall back to an installed Edge.
    assert_eq!(find_edge(Some(missing.to_str().unwrap()), &installed), None);
}

#[test]
fn finished_downloads_move_into_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("downloads");
    let to = dir.path().join("ws").join("downloads");
    std::fs::create_dir_all(&from).unwrap();
    std::fs::write(from.join("statement.pdf"), "new").unwrap();
    std::fs::write(from.join("report.csv"), "b").unwrap();
    std::fs::write(from.join("big.zip.crdownload"), "c").unwrap();
    std::fs::create_dir_all(&to).unwrap();
    std::fs::write(to.join("statement.pdf"), "old").unwrap();

    assert_eq!(
        move_downloads(&from, &to),
        ["report.csv", "statement (2).pdf"]
    );
    assert_eq!(
        std::fs::read_to_string(to.join("statement (2).pdf")).unwrap(),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(to.join("statement.pdf")).unwrap(),
        "old"
    );
    assert!(
        from.join("big.zip.crdownload").exists(),
        "still being written"
    );
    assert!(move_downloads(&from, &to).is_empty());
    assert!(move_downloads(&dir.path().join("none"), &to).is_empty());
}

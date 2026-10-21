//! Microsoft Edge for dots (browser spec sections 2.1 and 14) through `EdgeHost`, with
//! `fake-browser` in Edge's place: the switches and profile of a start, with no port; a second
//! open that finds it; a Dots that starts again starting a new one; the pages it lists and
//! closing it, over its pipe; a window the person closed; a profile another Edge holds; an Edge
//! or a Node.js that is not there; the helper `browser-pipe.mjs`, which gives Edge its pipe and
//! closes it when Dots goes; and moving downloads.

mod common;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use common::fake_browser::{exe, host, FakeEdges};
use common::playwright::JSON_LIST;
use dots_core::browser::host::{
    edge_args, edge_candidates, find_edge, move_downloads, page_urls, BrowserHost, EdgeHost,
    HostError, Launcher, EDGE_FLAGS, HELPER, HELPER_FILE, PIPE_FLAGS,
};
use dots_core::browser::NO_NODE;
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
        host: host(&root),
        root,
        dir,
    }
}

fn edge() -> PathBuf {
    PathBuf::from(exe())
}

#[tokio::test]
async fn edge_starts_on_the_dots_profile_with_its_switches_and_no_port() {
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    assert!(opened.started);
    let starts = e.edges.starts("D1");
    assert_eq!(starts.len(), 1);
    let args: Vec<String> = serde_json::from_value(starts[0]["args"].clone()).unwrap();
    assert_eq!(args, edge_args(&e.host.profile("D1")));
    assert_eq!(
        args[0],
        format!(
            "--user-data-dir={}",
            e.root.join("D1").join("profile").display()
        )
    );
    assert_eq!(args[1..8], EDGE_FLAGS);
    assert_eq!(args[8], "about:blank");
    assert_eq!(args.len(), 9);
    assert!(
        args.iter().all(|a| !a.contains("remote-debugging")),
        "{args:?}"
    );
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
    // Its pages, read over its pipe: the blank page; the extension's service worker is not one.
    assert_eq!(opened.edge.pages().await.unwrap(), ["about:blank"]);
    assert_eq!(
        e.edges.methods("D1"),
        ["Browser.getVersion", "Target.getTargets"]
    );
}

#[tokio::test]
async fn a_second_open_finds_it_and_a_dots_that_starts_again_starts_another() {
    let e = env();
    let first = e.host.open("D1", &edge()).await.unwrap();
    let again = e.host.open("D1", &edge()).await.unwrap();
    assert!(!again.started);
    assert!(Arc::ptr_eq(&first.edge, &again.edge));
    assert_eq!(e.edges.starts("D1").len(), 1);
    // Another dot gets an Edge of its own.
    let other = e.host.open("D2", &edge()).await.unwrap();
    assert!(!Arc::ptr_eq(&other.edge, &first.edge));
    assert!(e.edges.running("D1") && e.edges.running("D2"));
    // Dots stops: its Edges close with it. A Dots that starts again starts a new one.
    drop((e.host, first, again, other));
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert!(e.edges.gone_within("D2", Duration::from_secs(10)).await);
    let restarted = host(&e.root);
    assert!(restarted.open("D1", &edge()).await.unwrap().started);
    assert_eq!(e.edges.starts("D1").len(), 2);
}

#[tokio::test]
async fn closing_ends_it_over_its_pipe_and_a_closed_window_starts_again() {
    let e = env();
    let opened = e.host.open("D1", &edge()).await.unwrap();
    e.host.close("D1").await;
    assert!(e.edges.gone_within("D1", Duration::from_secs(10)).await);
    assert_eq!(e.edges.methods("D1").last().unwrap(), "Browser.close");
    assert!(opened.edge.is_closed());
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
        opened.edge.pages().await.unwrap(),
        ["https://shop.com/other", "https://shop.com/cart"]
    );
}

#[tokio::test]
async fn a_profile_held_by_another_edge_is_refused() {
    let e = env();
    // An Edge started outside Dots on the dot's profile: the fake, by hand.
    let mut outside = tokio::process::Command::new(exe());
    outside
        .arg(format!(
            "--user-data-dir={}",
            e.host.profile("D1").display()
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
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
        e.host.open("D1", &edge()).await.err(),
        Some(HostError::ProfileInUse)
    );
    assert_eq!(
        HostError::ProfileInUse.to_string(),
        "This dot's browser profile is in use by another Edge window; close it and try again."
    );
}

#[tokio::test]
async fn an_edge_or_a_node_that_is_not_there_is_said() {
    let e = env();
    let missing = e.dir.path().join("no-edge.exe");
    assert_eq!(
        e.host.open("D1", &missing).await.err(),
        Some(HostError::NotFound)
    );
    assert_eq!(
        HostError::NotFound.to_string(),
        "Microsoft Edge was not found; browser use needs it (Settings → Edge path)."
    );
    let nodeless = EdgeHost::with_launcher(&e.root, Launcher::Node("dots-no-such-node".into()));
    assert_eq!(
        nodeless.open("D1", &edge()).await.err(),
        Some(HostError::NoNode)
    );
    assert_eq!(HostError::NoNode.to_string(), NO_NODE);
    assert!(!e.edges.running("D1"));
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

/// Needs Node.js (`node`) on PATH, as browser use does.
#[tokio::test]
async fn the_helper_gives_edge_its_pipe_and_closes_it_when_dots_goes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("browser");
    let edges = FakeEdges::new(&root);
    let helped = EdgeHost::new(&root);
    let opened = helped.open("D1", &edge()).await.unwrap();
    assert!(opened.started);
    assert_eq!(
        std::fs::read_to_string(root.join(HELPER_FILE)).unwrap(),
        HELPER
    );
    let args: Vec<String> = serde_json::from_value(edges.starts("D1")[0]["args"].clone()).unwrap();
    assert_eq!(args[..2], PIPE_FLAGS);
    assert_eq!(args[2..], edge_args(&helped.profile("D1")));
    // Through the helper, on Edge's descriptors 3 and 4.
    edges.set_pages("D1", &["https://shop.com/cart"]);
    assert_eq!(
        opened.edge.pages().await.unwrap(),
        ["https://shop.com/cart"]
    );
    // Dots goes: the helper's stdin ends, and it closes Edge.
    drop((helped, opened));
    assert!(edges.gone_within("D1", Duration::from_secs(10)).await);
    let last = edges.received("D1").pop().unwrap();
    assert_eq!(last["method"], "Browser.close");
    assert_eq!(last["id"], 2147483647);
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

//! The runtime's browser commands (browser spec sections 6 to 8), with `fake-browser` as Edge,
//! started through the helper and Node.js (`node` on PATH) as Dots starts Edge: "Open the
//! browser" opens the dot's Edge on its pipe and profile, and a second click finds it; the
//! status names Edge; a deleted dot's Edge closes and its profile goes; a run's screenshot is
//! read back as a `data:` URL, only under the name Playwright gives one.

mod common;

use std::time::Duration;

use common::fake_browser::{exe, FakeEdges};
use common::spec;
use dots_core::model::{NewRun, TriggerKind, WorkspaceMode};
use dots_core::settings::Settings;
use dots_core::{Config, Error, Runtime};

/// A runtime on a temporary data folder, with no Claude CLI and `fake-browser` as Edge.
async fn runtime(dir: &tempfile::TempDir) -> Runtime {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(dir.path().join("no-claude.exe"));
    let rt = Runtime::start(cfg).await.unwrap();
    let settings = Settings {
        edge_path: Some(exe()),
        ..rt.settings().await
    };
    rt.update_settings(settings).await.unwrap();
    rt
}

fn browser_spec(dir: &tempfile::TempDir) -> dots_core::model::DotSpec {
    let mut s = spec("shopper");
    s.workdir = dir.path().to_string_lossy().into_owned();
    s.workspace_mode = WorkspaceMode::Folder;
    s.browser = true;
    s
}

#[tokio::test]
async fn open_the_browser_opens_the_dots_edge_and_deleting_the_dot_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let root = dir.path().join("data").join("browser");
    let edges = FakeEdges::new(&root);
    let dot = rt.create_dot(browser_spec(&dir), None).await.unwrap();
    rt.browser_open(&dot.id).await.unwrap();
    assert!(edges.running(&dot.id));
    let args: Vec<String> =
        serde_json::from_value(edges.starts(&dot.id)[0]["args"].clone()).unwrap();
    assert_eq!(args[..2], dots_core::browser::host::PIPE_FLAGS);
    assert_eq!(
        args[2],
        format!("--user-data-dir={}", edges.profile(&dot.id).display())
    );
    rt.browser_open(&dot.id).await.unwrap();
    assert_eq!(edges.starts(&dot.id).len(), 1, "a second click finds it");
    assert!(matches!(
        rt.browser_open("no-such-dot").await,
        Err(Error::NotFound(_))
    ));
    rt.delete_dot(&dot.id).await.unwrap();
    assert!(edges.gone_within(&dot.id, Duration::from_secs(10)).await);
    assert!(
        !root.join(&dot.id).exists(),
        "the profile goes with its dot"
    );
    rt.shutdown();
}

#[tokio::test]
async fn the_status_names_edge_and_a_missing_edge_is_said() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let status = rt.browser_status().await;
    assert_eq!(status.edge.as_deref(), Some(exe().as_str()));
    assert!(status
        .npx
        .is_none_or(|npx| npx.to_ascii_lowercase().contains("npx")));
    let missing = dir
        .path()
        .join("no-edge.exe")
        .to_string_lossy()
        .into_owned();
    let settings = Settings {
        edge_path: Some(missing),
        ..rt.settings().await
    };
    rt.update_settings(settings).await.unwrap();
    assert_eq!(rt.browser_status().await.edge, None);
    let dot = rt.create_dot(browser_spec(&dir), None).await.unwrap();
    let err = rt.browser_open(&dot.id).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "Microsoft Edge was not found; browser use needs it (Settings → Edge path)."
    );
    rt.shutdown();
}

#[tokio::test]
async fn a_screenshot_is_read_only_by_its_playwright_name() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let dot = rt.create_dot(browser_spec(&dir), None).await.unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join("downloads")).unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Manual);
    new.workspace_path = Some(ws.to_string_lossy().into_owned());
    let run = rt.store().create_run(&new).await.unwrap();
    let name = "page-2026-10-21T12-16-35-575Z.png";
    std::fs::write(ws.join("downloads").join(name), [0x89, b'P', b'N', b'G']).unwrap();
    assert_eq!(
        rt.browser_screenshot(&run.id, name).await.unwrap(),
        "data:image/png;base64,iVBORw=="
    );
    for other in [
        "../page-1.png",
        "page-1.png/../../secret",
        "statement.pdf",
        "page-.png",
        "page-1.gif",
        "PAGE-1.png",
        "page-1.png.exe",
    ] {
        assert!(
            matches!(
                rt.browser_screenshot(&run.id, other).await,
                Err(Error::Invalid(_))
            ),
            "{other}"
        );
    }
    assert!(matches!(
        rt.browser_screenshot(&run.id, "page-2026-10-21T12-16-35-575Z.jpeg")
            .await,
        Err(Error::NotFound(_))
    ));
    rt.shutdown();
}

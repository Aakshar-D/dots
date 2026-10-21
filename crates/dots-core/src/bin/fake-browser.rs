//! Test stand-in for Microsoft Edge in dots-core tests: `fake-browser --user-data-dir=<dir>
//! --remote-debugging-port=<port> [other switches] [address]`.
//!
//! - It holds `<dir>/lockfile` as Edge does: while it runs, no other process can open that
//!   file for writing. A start on a profile another process holds exits 0 at once without
//!   serving, as Edge hands over to the Edge that holds it.
//! - It answers enough of Edge's debugging HTTP on 127.0.0.1:<port>: `GET /json/version`;
//!   `GET /json/list` (its pages, most recent first, then a `service_worker` target, as
//!   Edge lists an extension's); `PUT /json/close/<id>`, which closes a page (the process
//!   exits once none is left).
//! - Test controls: `POST /fake/pages` with a JSON array of addresses makes those its pages,
//!   most recent first; `POST /fake/quit` exits at once, as when the person closes the window.
//! - Its first pages are `<dir>/fake-pages.json` (a JSON array) when that file exists, else the
//!   address it was given, else `about:blank`.
//! - Each start appends `{"args": [...], "port": <port>, "pid": <pid>}` to
//!   `<dir>/fake-browser.jsonl`. It exits when `<dir>` is gone, and after 120 s.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

struct Fake {
    port: u16,
    next: AtomicU64,
    /// `(id, address)`, most recent first.
    pages: Mutex<Vec<(String, String)>>,
}

impl Fake {
    fn page(&self, url: String) -> (String, String) {
        (
            format!("P{}", self.next.fetch_add(1, Ordering::Relaxed)),
            url,
        )
    }
}

/// The value of `--name=value`.
fn switch(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .find_map(|a| a.strip_prefix(name)?.strip_prefix('=').map(str::to_string))
}

/// Holds `<dir>/lockfile` as Edge does; `None` when another process holds it.
fn hold_lock(dir: &Path) -> Option<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ: others may read the file, not open it for writing.
        options.share_mode(1);
    }
    let file = options.open(dir.join("lockfile")).ok()?;
    #[cfg(not(windows))]
    file.try_lock().ok()?;
    Some(file)
}

fn exit_soon() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::process::exit(0);
    });
}

async fn version(State(fake): State<Arc<Fake>>) -> Json<Value> {
    Json(json!({
        "Browser": "Edg/0.0.0.0 (fake)",
        "Protocol-Version": "1.3",
        "webSocketDebuggerUrl": format!("ws://127.0.0.1:{}/devtools/browser/fake", fake.port)
    }))
}

async fn list(State(fake): State<Arc<Fake>>) -> Json<Value> {
    let mut targets: Vec<Value> = fake
        .pages
        .lock()
        .unwrap()
        .iter()
        .map(|(id, url)| {
            json!({
                "description": "", "id": id, "title": "", "type": "page", "url": url,
                "webSocketDebuggerUrl": format!("ws://127.0.0.1:{}/devtools/page/{id}", fake.port)
            })
        })
        .collect();
    targets.push(json!({
        "description": "", "id": "SW", "title": "Service Worker", "type": "service_worker",
        "url": "chrome-extension://fake/background.js"
    }));
    Json(Value::Array(targets))
}

async fn close(
    State(fake): State<Arc<Fake>>,
    UrlPath(id): UrlPath<String>,
) -> (StatusCode, String) {
    let mut pages = fake.pages.lock().unwrap();
    let Some(i) = pages.iter().position(|(page, _)| *page == id) else {
        return (StatusCode::NOT_FOUND, format!("No such target id: {id}"));
    };
    pages.remove(i);
    if pages.is_empty() {
        exit_soon();
    }
    (StatusCode::OK, "Target is closing".to_string())
}

async fn set_pages(State(fake): State<Arc<Fake>>, Json(urls): Json<Vec<String>>) -> StatusCode {
    let pages: Vec<(String, String)> = urls.into_iter().map(|url| fake.page(url)).collect();
    *fake.pages.lock().unwrap() = pages;
    StatusCode::OK
}

async fn quit() -> StatusCode {
    exit_soon();
    StatusCode::OK
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(switch(&args, "--user-data-dir").expect("--user-data-dir"));
    let port: u16 = switch(&args, "--remote-debugging-port")
        .expect("--remote-debugging-port")
        .parse()
        .expect("a port");
    std::fs::create_dir_all(&dir).unwrap();
    // Edge hands over to the Edge that holds the profile, and exits.
    let Some(_lock) = hold_lock(&dir) else {
        return;
    };
    let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await else {
        std::process::exit(2);
    };
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("fake-browser.jsonl"))
        .unwrap();
    writeln!(
        log,
        "{}",
        json!({"args": args, "port": port, "pid": std::process::id()})
    )
    .unwrap();
    let first: Vec<String> = std::fs::read_to_string(dir.join("fake-pages.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| {
            vec![args
                .iter()
                .rev()
                .find(|a| !a.starts_with("--"))
                .cloned()
                .unwrap_or_else(|| "about:blank".to_string())]
        });
    let fake = Arc::new(Fake {
        port,
        next: AtomicU64::new(0),
        pages: Mutex::new(Vec::new()),
    });
    let pages: Vec<(String, String)> = first.into_iter().map(|url| fake.page(url)).collect();
    *fake.pages.lock().unwrap() = pages;
    // Gone with its test: when the profile's folder is removed, or after 120 s.
    let watched = dir.clone();
    tokio::spawn(async move {
        let end = Instant::now() + Duration::from_secs(120);
        while Instant::now() < end && watched.exists() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        std::process::exit(0);
    });
    let app = Router::new()
        .route("/json/version", get(version))
        .route("/json/list", get(list))
        .route("/json/close/{id}", put(close))
        .route("/fake/pages", post(set_pages))
        .route("/fake/quit", post(quit))
        .with_state(fake);
    axum::serve(listener, app).await.unwrap();
}

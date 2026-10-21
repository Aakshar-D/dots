//! Test stand-in for Microsoft Edge in dots-core tests: `fake-browser --user-data-dir=<dir>
//! [other switches] [address]`.
//!
//! - It holds `<dir>/lockfile` as Edge does: while it runs, no other process can open that
//!   file for writing. A start on a profile another process holds exits 0 at once without
//!   serving, as Edge hands over to the Edge that holds it.
//! - It speaks Edge's debugging pipe: JSON messages, each ended by a NUL byte, read from file
//!   descriptor 3 and written to 4 when it was started with `--remote-debugging-pipe` (by the
//!   helper `browser-pipe.mjs`), else read from stdin and written to stdout (`EdgeHost` with
//!   `Launcher::Direct`). It exits when its input ends, as Edge does when the helper closes it.
//! - It answers `Browser.getVersion`; `Target.getTargets` (its pages, most recent first, then
//!   a `service_worker` target, as Edge lists an extension's); `Target.setAutoAttach` with
//!   `autoAttach: true` (first an event `Target.attachedToTarget` per page, session
//!   `S<page id>`); `Browser.close` (answers, then exits); `Fake.event` (first an event
//!   `Fake.event` with the same params); `Fake.big` with `size` (a result `{"data": "<size
//!   x's>"}`); any other request with an empty result. An answer carries its request's
//!   `sessionId`. Each message goes out in two pieces, as a pipe may deliver it.
//! - Every message it reads is appended to `<dir>/fake-cdp.jsonl`; one that is not JSON as
//!   `{"invalid": "<text>"}`, unanswered.
//! - Test controls, over HTTP on a port of its own on 127.0.0.1 that Dots never uses: `POST
//!   /fake/pages` with a JSON array of addresses makes those its pages, most recent first;
//!   `POST /fake/quit` exits at once, as when the person closes the window.
//! - Its first pages are `<dir>/fake-pages.json` (a JSON array) when that file exists, else the
//!   address it was given, else `about:blank`.
//! - Each start appends `{"args": [...], "control": <port>, "pid": <pid>}` to
//!   `<dir>/fake-browser.jsonl`. It exits when `<dir>` is gone, and after 120 s.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

type Output = Mutex<Box<dyn Write + Send>>;

struct Fake {
    next: AtomicU64,
    /// `(id, address)`, most recent first.
    pages: Mutex<Vec<(String, String)>>,
    received: Mutex<File>,
}

impl Fake {
    fn page(&self, url: String) -> (String, String) {
        (
            format!("P{}", self.next.fetch_add(1, Ordering::Relaxed)),
            url,
        )
    }

    fn record(&self, message: &Value) {
        let _ = writeln!(self.received.lock().unwrap(), "{message}");
    }

    fn pages(&self) -> Vec<(String, String)> {
        self.pages.lock().unwrap().clone()
    }
}

/// The value of `--name=value`.
fn switch(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .find_map(|a| a.strip_prefix(name)?.strip_prefix('=').map(str::to_string))
}

/// Holds `<dir>/lockfile` as Edge does; `None` when another process holds it.
fn hold_lock(dir: &Path) -> Option<File> {
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

/// The pipe's ends: descriptors 3 and 4 when the helper started it, else stdin and stdout.
fn pipe_ends(args: &[String]) -> (Box<dyn Read + Send>, Box<dyn Write + Send>) {
    if args.iter().any(|a| a == "--remote-debugging-pipe") {
        let (input, output) = inherited();
        (Box::new(input), Box::new(output))
    } else {
        (Box::new(std::io::stdin()), Box::new(std::io::stdout()))
    }
}

#[cfg(windows)]
fn inherited() -> (File, File) {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    extern "C" {
        fn _get_osfhandle(fd: i32) -> isize;
    }
    // SAFETY: the helper gave this process the pipe's ends as its C descriptors 3 and 4, and
    // each handle is owned once, here.
    unsafe {
        (
            File::from_raw_handle(_get_osfhandle(3) as RawHandle),
            File::from_raw_handle(_get_osfhandle(4) as RawHandle),
        )
    }
}

#[cfg(not(windows))]
fn inherited() -> (File, File) {
    use std::os::fd::FromRawFd;
    // SAFETY: the helper gave this process the pipe's ends as its descriptors 3 and 4, and
    // each is owned once, here.
    unsafe { (File::from_raw_fd(3), File::from_raw_fd(4)) }
}

/// One message, ended by its NUL byte, in two pieces.
fn send(output: &Output, message: &Value) {
    let mut bytes = message.to_string().into_bytes();
    bytes.push(0);
    let (first, second) = bytes.split_at(bytes.len() / 2);
    let mut output = output.lock().unwrap();
    let _ = output.write_all(first);
    let _ = output.flush();
    std::thread::sleep(Duration::from_millis(1));
    let _ = output.write_all(second);
    let _ = output.flush();
}

fn answer(fake: &Fake, frame: &[u8], output: &Output) {
    let Ok(message) = serde_json::from_slice::<Value>(frame) else {
        fake.record(&json!({"invalid": String::from_utf8_lossy(frame)}));
        return;
    };
    fake.record(&message);
    let Some(id) = message.get("id") else {
        return;
    };
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let mut result = json!({});
    match method {
        "Browser.getVersion" => {
            result = json!({"product": "Edg/0.0.0.0 (fake)", "protocolVersion": "1.3"});
        }
        "Target.getTargets" => {
            let mut targets: Vec<Value> = fake
                .pages()
                .into_iter()
                .map(|(id, url)| json!({"targetId": id, "type": "page", "title": "", "url": url, "attached": false}))
                .collect();
            targets.push(
                json!({"targetId": "SW", "type": "service_worker", "title": "Service Worker",
                                "url": "chrome-extension://fake/background.js", "attached": false}),
            );
            result = json!({"targetInfos": targets});
        }
        "Target.setAutoAttach" if params["autoAttach"] == true => {
            for (id, url) in fake.pages() {
                send(
                    output,
                    &json!({"method": "Target.attachedToTarget", "params": {
                        "sessionId": format!("S{id}"),
                        "targetInfo": {"targetId": id, "type": "page", "url": url, "attached": true},
                        "waitingForDebugger": false
                    }}),
                );
            }
        }
        "Fake.event" => send(output, &json!({"method": "Fake.event", "params": params})),
        "Fake.big" => {
            let size = params["size"].as_u64().unwrap_or(0) as usize;
            result = json!({"data": "x".repeat(size)});
        }
        _ => {}
    }
    let mut reply = json!({"id": id, "result": result});
    if let Some(session) = message.get("sessionId") {
        reply["sessionId"] = session.clone();
    }
    send(output, &reply);
    if method == "Browser.close" {
        std::process::exit(0);
    }
}

/// Reads the pipe until its input ends, then exits.
fn serve_pipe(fake: Arc<Fake>, mut input: Box<dyn Read + Send>, output: Output) {
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) | Err(_) => std::process::exit(0),
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(end) = pending.iter().position(|b| *b == 0) {
            let frame: Vec<u8> = pending.drain(..=end).collect();
            if end > 0 {
                answer(&fake, &frame[..end], &output);
            }
        }
    }
}

fn exit_soon() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::process::exit(0);
    });
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
    std::fs::create_dir_all(&dir).unwrap();
    // Edge hands over to the Edge that holds the profile, and exits.
    let Some(_lock) = hold_lock(&dir) else {
        return;
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let control = listener.local_addr().unwrap().port();
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("fake-browser.jsonl"))
        .unwrap();
    writeln!(
        log,
        "{}",
        json!({"args": args, "control": control, "pid": std::process::id()})
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
    let received = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("fake-cdp.jsonl"))
        .unwrap();
    let fake = Arc::new(Fake {
        next: AtomicU64::new(0),
        pages: Mutex::new(Vec::new()),
        received: Mutex::new(received),
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
    let (input, output) = pipe_ends(&args);
    let piped = fake.clone();
    std::thread::spawn(move || serve_pipe(piped, input, Mutex::new(output)));
    let app = Router::new()
        .route("/fake/pages", post(set_pages))
        .route("/fake/quit", post(quit))
        .with_state(fake);
    axum::serve(listener, app).await.unwrap();
}

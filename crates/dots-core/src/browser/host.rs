//! Microsoft Edge for dots (browser spec section 2.1): one visible window per dot, on the dot's
//! own profile (`<root>/<dot_id>/profile`), with its debugging port on 127.0.0.1. Dots reads the
//! addresses of its pages from `GET /json/list` and closes it with `PUT /json/close/<id>`, over
//! plain HTTP; it never clicks or types through that connection. Edge's own `msedge.exe` hands
//! over to the browser process and exits at once, so an Edge is followed by its port, not by a
//! process: the port is kept in `<root>/<dot_id>/port`, so a Dots that starts again finds an
//! Edge that is still open.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

/// The switches every dot's Edge starts with after its profile and port: no first-run pages,
/// no sign-in or sync of the person's account into the dot's profile, and pages that keep
/// running while the window is behind others (measured: without them every click timed out).
pub const EDGE_FLAGS: [&str; 7] = [
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-features=CalculateNativeWinOcclusion",
];
/// How long a started Edge has to open its debugging port.
pub const START_WAIT: Duration = Duration::from_secs(20);
/// How long a closed Edge has to go.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// How long one request to an Edge may take.
const ASK_WAIT: Duration = Duration::from_secs(2);

/// Why a dot's Edge cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("Microsoft Edge was not found; browser use needs it (Settings → Edge path).")]
    NotFound,
    #[error(
        "This dot's browser profile is in use by another Edge window; close it and try again."
    )]
    ProfileInUse,
    #[error("Microsoft Edge could not start: {0}")]
    Start(String),
    #[error("Microsoft Edge did not answer: {0}")]
    Unreachable(String),
}

/// A dot's Edge as `BrowserHost::open` found or started it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opened {
    pub port: u16,
    /// It did not run: it was started now.
    pub started: bool,
}

/// The dots' Edges. Tests use `EdgeHost` with `fake-browser` as the program.
#[async_trait]
pub trait BrowserHost: Send + Sync {
    /// The dot's Edge: the one running on its profile, or `exe` started on it.
    async fn open(&self, dot_id: &str, exe: &Path) -> Result<Opened, HostError>;
    /// The address of every page of the Edge on `port`, the one last in front first.
    async fn pages(&self, port: u16) -> Result<Vec<String>, HostError>;
    /// Closes the dot's Edge if it runs; returns once it is gone, or after `CLOSE_WAIT`.
    async fn close(&self, dot_id: &str);
    /// Where the dot's Edge saves downloads.
    fn downloads(&self, dot_id: &str) -> PathBuf;
    /// Closes the dot's Edge and removes its folder: profile, downloads, port (its dot was
    /// deleted).
    async fn remove(&self, dot_id: &str);
}

/// Microsoft Edge, one per dot, each with a folder under `root` (`<data_dir>/browser`).
pub struct EdgeHost {
    root: PathBuf,
    http: reqwest::Client,
    ports: Mutex<HashMap<String, u16>>,
    /// One start or close at a time: two runs of a dot must not start two Edges on its profile.
    starting: tokio::sync::Mutex<()>,
}

impl EdgeHost {
    pub fn new(root: impl Into<PathBuf>) -> EdgeHost {
        EdgeHost {
            root: root.into(),
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(ASK_WAIT)
                .build()
                .expect("an HTTP client"),
            ports: Mutex::new(HashMap::new()),
            starting: tokio::sync::Mutex::new(()),
        }
    }

    /// The dot's folder: its profile, downloads and port.
    pub fn dot_dir(&self, dot_id: &str) -> PathBuf {
        self.root.join(dot_id)
    }

    pub fn profile(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("profile")
    }

    fn port_file(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("port")
    }

    /// The port of the dot's Edge as last seen, by this Dots or an earlier one.
    fn known_port(&self, dot_id: &str) -> Option<u16> {
        if let Some(port) = self.ports.lock().unwrap().get(dot_id) {
            return Some(*port);
        }
        std::fs::read_to_string(self.port_file(dot_id))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Keeps (or forgets) the port of the dot's Edge.
    fn remember(&self, dot_id: &str, port: Option<u16>) {
        let mut ports = self.ports.lock().unwrap();
        match port {
            Some(port) => {
                ports.insert(dot_id.to_string(), port);
                if let Err(e) = std::fs::write(self.port_file(dot_id), port.to_string()) {
                    tracing::warn!("could not keep the port of {dot_id}'s Edge: {e}");
                }
            }
            None => {
                ports.remove(dot_id);
                let _ = std::fs::remove_file(self.port_file(dot_id));
            }
        }
    }

    /// Whether an Edge answers on `port`.
    async fn alive(&self, port: u16) -> bool {
        let url = format!("http://127.0.0.1:{port}/json/version");
        match self.http.get(url).send().await {
            Ok(answer) if answer.status().is_success() => answer
                .json::<Value>()
                .await
                .is_ok_and(|v| v.get("Browser").is_some()),
            _ => false,
        }
    }

    /// Every target of the Edge on `port`: pages, extensions' pages, workers.
    async fn targets(&self, port: u16) -> Result<Vec<Value>, HostError> {
        let url = format!("http://127.0.0.1:{port}/json/list");
        let unreachable = |e: &dyn std::fmt::Display| HostError::Unreachable(e.to_string());
        let answer = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| unreachable(&e))?;
        let text = answer.text().await.map_err(|e| unreachable(&e))?;
        serde_json::from_str(&text).map_err(|e| unreachable(&e))
    }
}

#[async_trait]
impl BrowserHost for EdgeHost {
    async fn open(&self, dot_id: &str, exe: &Path) -> Result<Opened, HostError> {
        let _one = self.starting.lock().await;
        if let Some(port) = self.known_port(dot_id) {
            if self.alive(port).await {
                self.remember(dot_id, Some(port));
                return Ok(Opened {
                    port,
                    started: false,
                });
            }
            self.remember(dot_id, None);
        }
        if !exe.is_file() {
            return Err(HostError::NotFound);
        }
        let profile = self.profile(dot_id);
        if profile_in_use(&profile) {
            return Err(HostError::ProfileInUse);
        }
        prepare_profile(&profile, &self.downloads(dot_id))
            .map_err(|e| HostError::Start(format!("its profile could not be prepared: {e}")))?;
        let port = free_port().map_err(|e| HostError::Start(format!("no free port: {e}")))?;
        let mut command = tokio::process::Command::new(exe);
        command
            .args(edge_args(&profile, port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::proc::hide_window(&mut command);
        // Not waited for: Edge's own program hands over to the browser and exits.
        command
            .spawn()
            .map_err(|e| HostError::Start(e.to_string()))?;
        let deadline = Instant::now() + START_WAIT;
        while Instant::now() < deadline {
            // Ours only once it holds the profile: another Edge may have taken the port.
            if self.alive(port).await && profile_in_use(&profile) {
                self.remember(dot_id, Some(port));
                return Ok(Opened {
                    port,
                    started: true,
                });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(HostError::Start(format!(
            "it did not open its debugging port within {} s",
            START_WAIT.as_secs()
        )))
    }

    async fn pages(&self, port: u16) -> Result<Vec<String>, HostError> {
        Ok(page_urls(&self.targets(port).await?))
    }

    async fn close(&self, dot_id: &str) {
        let _one = self.starting.lock().await;
        let Some(port) = self.known_port(dot_id) else {
            return;
        };
        // Edge ends once its last page is closed.
        if let Ok(targets) = self.targets(port).await {
            for target in targets.iter().filter(|t| is_page(t)) {
                if let Some(id) = target.get("id").and_then(Value::as_str) {
                    let url = format!("http://127.0.0.1:{port}/json/close/{id}");
                    let _ = self.http.put(url).send().await;
                }
            }
        }
        let deadline = Instant::now() + CLOSE_WAIT;
        while Instant::now() < deadline && self.alive(port).await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.remember(dot_id, None);
    }

    fn downloads(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("downloads")
    }

    async fn remove(&self, dot_id: &str) {
        self.close(dot_id).await;
        let dir = self.dot_dir(dot_id);
        // Edge lets go of its profile a moment after its port closes.
        for _ in 0..20 {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
        tracing::warn!("could not remove {}", dir.display());
    }
}

/// The arguments of a dot's Edge: its profile, its port on 127.0.0.1, `EDGE_FLAGS`, and a blank
/// page.
pub fn edge_args(profile: &Path, port: u16) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--remote-debugging-port={port}"),
    ];
    args.extend(EDGE_FLAGS.iter().map(|flag| flag.to_string()));
    args.push("about:blank".to_string());
    args
}

fn is_page(target: &Value) -> bool {
    target.get("type").and_then(Value::as_str) == Some("page")
}

/// The address of every `page` target of a `/json/list` answer, in its order.
pub fn page_urls(targets: &[Value]) -> Vec<String> {
    targets
        .iter()
        .filter(|t| is_page(t))
        .filter_map(|t| t.get("url").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

/// Where Edge is looked for: `configured` (Settings' Edge path) when it is set, else
/// `%ProgramFiles(x86)%` and `%ProgramFiles%` under `Microsoft\Edge\Application\msedge.exe`.
pub fn edge_candidates(
    configured: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<PathBuf> {
    if let Some(path) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        return vec![PathBuf::from(path)];
    }
    ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(|key| env(key))
        .map(|base| {
            Path::new(&base)
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe")
        })
        .collect()
}

/// The first of `edge_candidates` that is a file. A configured path that is not a file is not
/// found: Dots does not fall back to another Edge.
pub fn find_edge(
    configured: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    edge_candidates(configured, env)
        .into_iter()
        .find(|path| path.is_file())
}

/// A program as Dots starts it: a path that is a file, or a bare name found on `PATH` with
/// `PATHEXT` (`npx` is `npx.cmd`). `None` when there is none.
pub fn find_program(program: &str) -> Option<PathBuf> {
    let path = crate::mcp::resolve_program(
        program,
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("PATHEXT").as_deref(),
    );
    path.is_file().then_some(path)
}

/// Whether an Edge holds `profile`: Edge keeps `<profile>/lockfile` open so that no other
/// process can open it for writing.
pub fn profile_in_use(profile: &Path) -> bool {
    let lock = profile.join("lockfile");
    if !lock.exists() {
        return false;
    }
    match std::fs::OpenOptions::new().write(true).open(&lock) {
        Ok(file) => matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        Err(_) => true,
    }
}

/// Makes the profile's folder and the download folder, and writes the download folder into the
/// profile's `Default/Preferences`, without a question per file. A `Preferences` that Dots
/// cannot read as a JSON object is Edge's and is left as it is.
fn prepare_profile(profile: &Path, downloads: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(profile.join("Default"))?;
    std::fs::create_dir_all(downloads)?;
    let prefs = profile.join("Default").join("Preferences");
    let mut value = match std::fs::read_to_string(&prefs) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(object @ Value::Object(_)) => object,
            _ => return Ok(()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    if !value["download"].is_object() {
        value["download"] = json!({});
    }
    value["download"]["default_directory"] = json!(downloads.to_string_lossy());
    value["download"]["prompt_for_download"] = json!(false);
    value["download"]["directory_upgrade"] = json!(true);
    std::fs::write(&prefs, value.to_string())
}

/// A port on 127.0.0.1 that is free now.
fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

/// Moves every finished download in `from` into `to` (made when needed); a name taken there
/// gets ` (2)`, ` (3)`, … before its extension. A file Edge is still writing
/// (`.crdownload`, `.partial`, `.tmp`) stays. Returns the names the files have in `to`, by
/// name.
pub fn move_downloads(from: &Path, to: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !in_progress(name))
        .collect();
    names.sort();
    let mut moved = Vec::new();
    for name in names {
        if let Err(e) = std::fs::create_dir_all(to) {
            tracing::warn!("could not make {}: {e}", to.display());
            break;
        }
        let source = from.join(&name);
        let target = free_name(to, &name);
        let done = std::fs::rename(&source, &target).or_else(|_| {
            // Another drive: copy, then remove.
            std::fs::copy(&source, &target)?;
            std::fs::remove_file(&source)
        });
        match done {
            Ok(()) => moved.push(target.file_name().unwrap().to_string_lossy().into_owned()),
            Err(e) => tracing::warn!("could not move the download {name}: {e}"),
        }
    }
    moved
}

fn in_progress(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".crdownload")
        || name.ends_with(".partial")
        || name.ends_with(".tmp")
        || name.starts_with('.')
}

/// `dir/name`, or `dir/<stem> (<n>)<ext>` with the first `n` from 2 that is free.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .expect("a free name")
}

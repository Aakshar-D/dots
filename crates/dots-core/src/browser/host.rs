//! Microsoft Edge for dots (browser spec sections 2.1 and 14): one visible window per dot, on
//! the dot's own profile (`<root>/<dot_id>/profile`), with no debugging port. Dots starts Edge
//! through the helper `browser-pipe.mjs` (written into `<root>`), which gives Edge its
//! debugging pipe and relays it to the helper's stdin and stdout: Dots holds the only
//! connection (`pipe.rs`). Dots reads Edge's pages over it and closes Edge with
//! `Browser.close`; Playwright reaches Edge through its door (`door.rs`). An Edge lives at
//! most as long as the Dots that started it: when Dots stops, the helper's stdin ends and it
//! closes Edge.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;

use super::pipe::Pipe;

/// The switches every dot's Edge starts with after its profile: no first-run pages, no sign-in
/// or sync of the person's account into the dot's profile, and pages that keep running while
/// the window is behind others (measured: without them every click timed out).
pub const EDGE_FLAGS: [&str; 7] = [
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-features=CalculateNativeWinOcclusion",
];
/// The switches the helper puts first: the debugging pipe, and no hand-over to another
/// process, which would lose the pipe.
pub const PIPE_FLAGS: [&str; 2] = [
    "--remote-debugging-pipe",
    "--edge-skip-compat-layer-relaunch",
];
/// The helper, as Dots writes it into `<root>` before each start.
pub const HELPER: &str = include_str!("browser-pipe.mjs");
/// Its file name.
pub const HELPER_FILE: &str = "browser-pipe.mjs";
/// How long a started Edge has to answer on its pipe.
pub const START_WAIT: Duration = Duration::from_secs(20);
/// How long a closed Edge has to go.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);

/// Why a dot's Edge cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error("Microsoft Edge was not found; browser use needs it (Settings → Edge path).")]
    NotFound,
    #[error(
        "This dot's browser profile is in use by another Edge window; close it and try again."
    )]
    ProfileInUse,
    #[error("Browser use needs Node.js (npx).")]
    NoNode,
    #[error("Microsoft Edge could not start: {0}")]
    Start(String),
    #[error("Microsoft Edge did not answer: {0}")]
    Unreachable(String),
}

/// A dot's Edge as `BrowserHost::open` found or started it.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The connection to it.
    pub edge: Arc<Pipe>,
    /// It did not run: it was started now.
    pub started: bool,
}

/// The dots' Edges. Tests use `EdgeHost` with `fake-browser` as the program.
#[async_trait]
pub trait BrowserHost: Send + Sync {
    /// The dot's Edge: the one running on its profile, or `exe` started on it.
    async fn open(&self, dot_id: &str, exe: &Path) -> Result<Opened, HostError>;
    /// Closes the dot's Edge if it runs; returns once it is gone, or after `CLOSE_WAIT`.
    async fn close(&self, dot_id: &str);
    /// Where the dot's Edge saves downloads.
    fn downloads(&self, dot_id: &str) -> PathBuf;
    /// The folder Playwright servers start in: Dots' own, never a workspace, where a dot could
    /// plant a `node_modules` or an `.npmrc` for npx to follow.
    fn server_dir(&self) -> PathBuf;
    /// Closes the dot's Edge and removes its folder: profile and downloads (its dot was
    /// deleted).
    async fn remove(&self, dot_id: &str);
}

/// How a dot's Edge is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launcher {
    /// `<node> <root>/browser-pipe.mjs <msedge.exe> <switches>`: the helper gives Edge its
    /// pipe. `node` is found on `PATH` when it is a bare name.
    Node(String),
    /// The program speaks the pipe itself on its stdin and stdout (`fake-browser` in tests).
    Direct,
}

/// A dot's running Edge: its pipe, and the helper's process.
struct Running {
    pipe: Arc<Pipe>,
    child: tokio::process::Child,
}

/// Microsoft Edge, one per dot, each with a folder under `root` (`<data_dir>/browser`).
pub struct EdgeHost {
    root: PathBuf,
    launcher: Launcher,
    /// The running Edges. Held through a start or a close: two runs of a dot must not start
    /// two Edges on its profile.
    edges: tokio::sync::Mutex<HashMap<String, Running>>,
}

impl EdgeHost {
    /// Edges started through the helper, with `node`.
    pub fn new(root: impl Into<PathBuf>) -> EdgeHost {
        EdgeHost::with_launcher(root, Launcher::Node("node".to_string()))
    }

    pub fn with_launcher(root: impl Into<PathBuf>, launcher: Launcher) -> EdgeHost {
        EdgeHost {
            root: root.into(),
            launcher,
            edges: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// The dot's folder: its profile and downloads.
    pub fn dot_dir(&self, dot_id: &str) -> PathBuf {
        self.root.join(dot_id)
    }

    pub fn profile(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("profile")
    }

    /// The command that starts `exe` with `args` on a pipe.
    fn command(&self, exe: &Path, args: &[String]) -> Result<tokio::process::Command, HostError> {
        match &self.launcher {
            Launcher::Direct => {
                let mut command = tokio::process::Command::new(exe);
                command.args(args);
                Ok(command)
            }
            Launcher::Node(node) => {
                let node = find_program(node).ok_or(HostError::NoNode)?;
                let helper = self.root.join(HELPER_FILE);
                std::fs::create_dir_all(&self.root)
                    .and_then(|()| std::fs::write(&helper, HELPER))
                    .map_err(|e| HostError::Start(format!("{}: {e}", helper.display())))?;
                let mut command = tokio::process::Command::new(node);
                command.arg(helper).arg(exe).args(args);
                Ok(command)
            }
        }
    }
}

#[async_trait]
impl BrowserHost for EdgeHost {
    fn server_dir(&self) -> PathBuf {
        self.root.join("playwright")
    }

    async fn open(&self, dot_id: &str, exe: &Path) -> Result<Opened, HostError> {
        let mut edges = self.edges.lock().await;
        if let Some(running) = edges.get(dot_id).filter(|r| !r.pipe.is_closed()) {
            // Only while it answers: the person may have closed the window a moment ago, before
            // its pipe's end was read.
            if running
                .pipe
                .ask("Browser.getVersion", json!({}))
                .await
                .is_ok()
            {
                return Ok(Opened {
                    edge: running.pipe.clone(),
                    started: false,
                });
            }
        }
        // The person closed the window, or the helper ended.
        if let Some(gone) = edges.remove(dot_id) {
            reap(gone.child).await;
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
        let mut command = self.command(exe, &edge_args(&profile))?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::proc::hide_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| HostError::Start(e.to_string()))?;
        let (Some(stdout), Some(stdin)) = (child.stdout.take(), child.stdin.take()) else {
            return Err(HostError::Start("its pipe could not be opened".to_string()));
        };
        log_errors(child.stderr.take(), dot_id);
        let pipe = Pipe::new(stdout, stdin);
        if let Err(e) = pipe
            .ask_within("Browser.getVersion", json!({}), START_WAIT)
            .await
        {
            let _ = child.start_kill();
            reap(child).await;
            // Edge hands over to the Edge that holds the profile, and closes its pipe.
            if profile_in_use(&profile) {
                return Err(HostError::ProfileInUse);
            }
            return Err(HostError::Start(format!(
                "it did not answer on its pipe: {e}"
            )));
        }
        edges.insert(
            dot_id.to_string(),
            Running {
                pipe: pipe.clone(),
                child,
            },
        );
        Ok(Opened {
            edge: pipe,
            started: true,
        })
    }

    async fn close(&self, dot_id: &str) {
        let mut edges = self.edges.lock().await;
        let Some(running) = edges.remove(dot_id) else {
            return;
        };
        if !running.pipe.is_closed() {
            let _ = running.pipe.ask("Browser.close", json!({})).await;
        }
        reap(running.child).await;
    }

    fn downloads(&self, dot_id: &str) -> PathBuf {
        self.dot_dir(dot_id).join("downloads")
    }

    async fn remove(&self, dot_id: &str) {
        self.close(dot_id).await;
        let dir = self.dot_dir(dot_id);
        // Edge lets go of its profile a moment after it ends.
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

/// Waits up to `CLOSE_WAIT` for the helper (and its Edge) to end, then ends it.
async fn reap(mut child: tokio::process::Child) {
    if tokio::time::timeout(CLOSE_WAIT, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

/// The helper's own errors go to the app log.
fn log_errors(stderr: Option<tokio::process::ChildStderr>, dot_id: &str) {
    let Some(stderr) = stderr else {
        return;
    };
    let dot_id = dot_id.to_string();
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(dot = %dot_id, "{line}");
        }
    });
}

/// The arguments of a dot's Edge: its profile, `EDGE_FLAGS`, and a blank page. The helper puts
/// `PIPE_FLAGS` before them.
pub fn edge_args(profile: &Path) -> Vec<String> {
    let mut args = vec![format!("--user-data-dir={}", profile.display())];
    args.extend(EDGE_FLAGS.iter().map(|flag| flag.to_string()));
    args.push("about:blank".to_string());
    args
}

fn is_page(target: &Value) -> bool {
    target.get("type").and_then(Value::as_str) == Some("page")
}

/// The address of every `page` target of a list of targets (`Target.getTargets`' `targetInfos`,
/// or a `/json/list` answer), in its order.
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

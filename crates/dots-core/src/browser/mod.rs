//! Browser use for dots (browser spec `docs/superpowers/specs/2026-10-21-browser-design.md`):
//! Microsoft Edge in a visible window with the dot's own profile, driven by Microsoft's
//! Playwright MCP server, behind Dots' gate, which judges each call by the address of the page
//! it acts on (`sites`) and the labels in Playwright's own snapshots.

pub mod gate;
pub mod host;
pub mod sites;
pub mod snapshot;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use self::gate::{SiteRules, Verdict, ASK_KEY};
use self::host::{find_edge, find_program, move_downloads, BrowserHost, HostError};
use self::snapshot::PageMemory;
use crate::engine::{PermissionGate, PermissionOutcome};
use crate::library::McpServerConfig;
use crate::library_service::process_env;
use crate::mcp::{McpSession, SessionOptions};
use crate::model::Dot;
use crate::policy::McpTrust;
use crate::run_tools::RunTools;
use crate::store::Store;

/// The built-in server's name (browser spec section 2.2): its tools are `mcp__browser__<tool>`
/// on both engines.
pub const SERVER: &str = "browser";
/// The prefix of its tools' names.
pub const PREFIX: &str = "mcp__browser__";

/// Whether `qualified` names a tool of the built-in browser.
pub fn is_browser_tool(qualified: &str) -> bool {
    qualified.len() > PREFIX.len() && qualified.starts_with(PREFIX)
}

/// The pinned Playwright MCP server (browser spec section 2.2). A new version comes with
/// captures of its output and the gate's tests on them (section 11).
pub const PLAYWRIGHT_MCP: &str = "@playwright/mcp@0.0.83";
/// How long the Playwright server has to start and answer `initialize`: `npx` fetches the
/// package the first time.
pub const START_TIMEOUT: Duration = Duration::from_secs(120);

/// What the dot is told about a browser call approved after its run parked (section 3.6).
pub const LATE_APPROVAL: &str = "Your browser action was approved after the page may have changed. Take a snapshot and do it again if it is still right.";
/// What the dot is told about a call approved while it waited, on a page that moved meanwhile.
pub const PAGE_MOVED: &str = "The page changed while your browser action waited for approval, so it was not performed. Take a snapshot and do it again if it is still right.";
/// What a call answers when `npx` is not there (section 5).
pub const NO_NODE: &str = "Browser use needs Node.js (npx).";

const TOOLS_JSON: &str = include_str!("tools.json");

/// The tools of the pinned server, as its `tools/list` answered (`tools.json`, captured from
/// 0.0.83 on 2026-10-21). The run serves this list without starting the server.
pub fn tools() -> &'static Value {
    static TOOLS: OnceLock<Value> = OnceLock::new();
    TOOLS.get_or_init(|| serde_json::from_str(TOOLS_JSON).expect("browser/tools.json is JSON"))
}

/// The tools as the local engine offers them: functions named `mcp__browser__<tool>`, which
/// fit a function name as they are.
pub fn function_schemas() -> Vec<Value> {
    tools()
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": format!("{PREFIX}{}", tool["name"].as_str().unwrap_or_default()),
                    "description": tool["description"],
                    "parameters": tool["inputSchema"]
                }
            })
        })
        .collect()
}

/// The line of a resume message about a browser call approved after its run parked.
pub fn late_line(approval_id: &str) -> String {
    format!("- Approval #{approval_id} for a browser action: {LATE_APPROVAL}")
}

/// A `tools/call` result that says why a call was not performed.
pub fn not_performed(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// How the Playwright server is started: `npx -y @playwright/mcp@0.0.83` unless a test says
/// otherwise (`fake-mcp` and its script).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaywrightCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl Default for PlaywrightCommand {
    fn default() -> Self {
        Self {
            program: "npx".to_string(),
            args: vec!["-y".to_string(), PLAYWRIGHT_MCP.to_string()],
        }
    }
}

impl PlaywrightCommand {
    /// Its arguments for a run: drive the Edge on `port`, write files into `output`, and put
    /// no page snapshot into an act's result (section 2.2; no `--caps`).
    pub fn args_for(&self, port: u16, output: &Path) -> Vec<String> {
        let mut args = self.args.clone();
        args.extend([
            "--cdp-endpoint".to_string(),
            format!("http://127.0.0.1:{port}"),
            "--output-dir".to_string(),
            output.to_string_lossy().into_owned(),
            "--snapshot-mode".to_string(),
            "none".to_string(),
        ]);
        args
    }
}

/// Whether Microsoft Edge and Node.js (`npx`) were found, and where (Settings, section 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserStatus {
    pub edge: Option<String>,
    pub npx: Option<String>,
}

/// Who uses a dot's Edge: its runs that called the browser, and the person.
#[derive(Debug, Default)]
struct Users {
    runs: usize,
    person: bool,
}

/// The dots' browsers (section 2.1): one Edge per dot, shared by the dot's runs, closed when the
/// last run that used it ends, unless the person opened it ("Open the browser").
pub struct Browsers {
    store: Store,
    host: Arc<dyn BrowserHost>,
    playwright: PlaywrightCommand,
    users: tokio::sync::Mutex<HashMap<String, Users>>,
}

impl Browsers {
    pub fn new(
        store: Store,
        host: Arc<dyn BrowserHost>,
        playwright: PlaywrightCommand,
    ) -> Arc<Browsers> {
        Arc::new(Browsers {
            store,
            host,
            playwright,
            users: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Edge as Settings name it, or as it is installed.
    async fn edge(&self) -> Result<PathBuf, HostError> {
        let configured = self
            .store
            .get_settings()
            .await
            .map_err(|e| HostError::Start(e.to_string()))?
            .edge_path;
        find_edge(configured.as_deref(), &process_env).ok_or(HostError::NotFound)
    }

    /// The dot's Edge for a run: found, or started. `hold` counts the run in (its first call);
    /// a later call only finds it again, or starts it after the person closed it.
    async fn open(&self, dot_id: &str, hold: bool) -> Result<u16, HostError> {
        let exe = self.edge().await?;
        let mut users = self.users.lock().await;
        let opened = self.host.open(dot_id, &exe).await?;
        let user = users.entry(dot_id.to_string()).or_default();
        if opened.started {
            user.person = false;
        }
        if hold {
            user.runs += 1;
        }
        Ok(opened.port)
    }

    /// A run is done with the dot's Edge: the last one closes it, unless the person opened it.
    async fn release(&self, dot_id: &str) {
        let mut users = self.users.lock().await;
        let Some(user) = users.get_mut(dot_id) else {
            return;
        };
        user.runs = user.runs.saturating_sub(1);
        if user.runs == 0 && !user.person {
            users.remove(dot_id);
            self.host.close(dot_id).await;
        }
    }

    /// "Open the browser" (section 7): the dot's Edge, for the person to log in ahead of time.
    /// It stays open until they close it; the dot's runs use it meanwhile.
    pub async fn open_for_person(&self, dot_id: &str) -> Result<(), HostError> {
        let exe = self.edge().await?;
        let mut users = self.users.lock().await;
        self.host.open(dot_id, &exe).await?;
        users.entry(dot_id.to_string()).or_default().person = true;
        Ok(())
    }

    /// The dot was deleted: its Edge closes and its profile goes (section 6).
    pub async fn forget(&self, dot_id: &str) {
        let mut users = self.users.lock().await;
        users.remove(dot_id);
        self.host.remove(dot_id).await;
    }

    /// Where Edge and `npx` are, as a run would start them.
    pub async fn status(&self) -> BrowserStatus {
        BrowserStatus {
            edge: self.edge().await.ok().map(|p| p.display().to_string()),
            npx: find_program(&self.playwright.program).map(|p| p.display().to_string()),
        }
    }

    /// The browser of a run of `dot` working in `workspace`. Nothing starts before its first
    /// call.
    pub fn for_run(self: &Arc<Self>, dot: &Dot, run_id: &str, workspace: &Path) -> RunBrowser {
        RunBrowser {
            browsers: self.clone(),
            dot_id: dot.id.clone(),
            run_id: run_id.to_string(),
            workspace: workspace.to_path_buf(),
            live: tokio::sync::Mutex::new(Live::default()),
            memory: Mutex::new(PageMemory::default()),
        }
    }
}

/// What a run's browser has open.
#[derive(Default)]
struct Live {
    /// The run counts as a user of the dot's Edge.
    held: bool,
    closed: bool,
    /// The port of the Edge the session drives.
    port: Option<u16>,
    session: Option<Arc<McpSession>>,
}

/// One run's browser (section 2.2): the dot's Edge and a Playwright server of its own, started
/// at its first call; the gate before every call; what its results told of the pages.
pub struct RunBrowser {
    browsers: Arc<Browsers>,
    dot_id: String,
    run_id: String,
    workspace: PathBuf,
    live: tokio::sync::Mutex<Live>,
    memory: Mutex<PageMemory>,
}

impl RunBrowser {
    /// The Playwright session of the next call: Edge found or started, and the server started at
    /// the first call, again after the person closed Edge (a new port), and again after it
    /// stopped.
    async fn ready(&self) -> Result<Arc<McpSession>, String> {
        let mut live = self.live.lock().await;
        if live.closed {
            return Err("The run has ended.".to_string());
        }
        let port = self
            .browsers
            .open(&self.dot_id, !live.held)
            .await
            .map_err(|e| e.to_string())?;
        live.held = true;
        if live.port != Some(port) {
            // Another Edge: what the run knew of its pages is gone.
            *self.memory.lock().unwrap() = PageMemory::default();
            if let Some(old) = live.session.take() {
                old.close().await;
            }
            live.port = Some(port);
        }
        if let Some(session) = live.session.clone().filter(|s| !s.is_stopped()) {
            return Ok(session);
        }
        if let Some(old) = live.session.take() {
            old.close().await;
        }
        let session = Arc::new(self.start_server(port).await?);
        live.session = Some(session.clone());
        Ok(session)
    }

    /// Starts the Playwright server on the Edge on `port`, in the run's workspace.
    async fn start_server(&self, port: u16) -> Result<McpSession, String> {
        let command = &self.browsers.playwright;
        let Some(program) = find_program(&command.program) else {
            return Err(NO_NODE.to_string());
        };
        let config = McpServerConfig::Stdio {
            command: program.to_string_lossy().into_owned(),
            args: command.args_for(port, &self.workspace.join("downloads")),
            cwd: Some(self.workspace.to_string_lossy().into_owned()),
            env: BTreeMap::new(),
        };
        let options = SessionOptions {
            cwd: Some(self.workspace.clone()),
            answer_timeout: START_TIMEOUT,
            ..SessionOptions::default()
        };
        McpSession::open(SERVER, &config, &no_variables, options)
            .await
            .map_err(|e| format!("The browser could not start: {e}"))
    }

    /// The address of the page the next act lands on: the one Playwright reported as current
    /// last, while Edge still has a page at exactly that address; before any report, Edge's
    /// only page. `None` otherwise: the act is then on no site.
    async fn page_now(&self) -> Option<String> {
        let port = self.live.lock().await.port?;
        let pages = self.browsers.host.pages(port).await.ok()?;
        let reported = self.memory.lock().unwrap().current().map(str::to_string);
        match reported {
            Some(url) => pages.contains(&url).then_some(url),
            None => match pages.as_slice() {
                [only] => Some(only.clone()),
                _ => None,
            },
        }
    }

    /// Moves the dot's finished downloads into the workspace's `downloads/`.
    fn collect_downloads(&self) -> Vec<String> {
        move_downloads(
            &self.browsers.host.downloads(&self.dot_id),
            &self.workspace.join("downloads"),
        )
    }

    /// Decides, performs and reports one call of the browser tool `tool` (`browser_click`), as
    /// the gateway and the local engine hand it over. Returns a `tools/call` result; a call that
    /// is not performed is a result with `isError` that says why. A performed call's result
    /// starts with its run-view lines, `Dots: <line>`.
    pub async fn call(
        &self,
        gate: &dyn PermissionGate,
        tool: &str,
        input: Value,
        limit: Duration,
        cancel: &CancellationToken,
    ) -> Value {
        let mut input = match input {
            Value::Null => json!({}),
            object @ Value::Object(_) => object,
            _ => return not_performed("the arguments must be a JSON object"),
        };
        // The card is Dots' own: one in the model's input is dropped.
        if let Some(object) = input.as_object_mut() {
            object.remove(ASK_KEY);
        }
        let qualified = format!("{PREFIX}{tool}");
        let dot = match self.browsers.store.get_dot(&self.dot_id).await {
            Ok(dot) => dot,
            Err(e) => return not_performed(&format!("the dot could not be read: {e}")),
        };
        // A deny rule on the tool itself (`mcp__browser__*`) refuses it as well.
        if dot.spec.policy.denied_by_rule(&qualified, &input, None) {
            return not_performed(&format!(
                "Denied by the policy of dot '{}': {qualified} is not permitted.",
                dot.spec.name
            ));
        }
        let session = match self.ready().await {
            Ok(session) => session,
            Err(text) => return not_performed(&text),
        };
        let kind = gate::classify(tool, &input);
        let page = self.page_now().await;
        let (verdict, line) = {
            let memory = self.memory.lock().unwrap();
            let refs = page.as_deref().and_then(|url| memory.refs(url));
            let rules = SiteRules::of(&dot.spec.policy);
            (
                gate::judge(tool, &kind, page.as_deref(), refs, &rules, &dot.spec.name),
                gate::line(tool, &kind, refs),
            )
        };
        match verdict {
            Verdict::Allow => {}
            Verdict::Refuse(text) => return not_performed(&text),
            Verdict::Ask(ask) => {
                let mut asked = input.clone();
                asked[ASK_KEY] = serde_json::to_value(&ask).unwrap_or(Value::Null);
                // Held: the hub asks unless a deny rule names the tool.
                let held = McpTrust {
                    held: true,
                    read_only: false,
                };
                match gate
                    .check_mcp(&self.run_id, &qualified, asked, held, None)
                    .await
                {
                    Ok(PermissionOutcome::Allow { .. }) => {
                        // The page may have moved on while the person decided.
                        if ask.action != "open" && self.page_now().await != page {
                            return not_performed(PAGE_MOVED);
                        }
                    }
                    Ok(PermissionOutcome::Deny { message }) => return not_performed(&message),
                    Err(e) => return not_performed(&format!("permission check failed: {e}")),
                }
            }
        }
        let result = match session.call_tool(tool, input.clone(), limit, cancel).await {
            Ok(result) => result,
            Err(e) => return not_performed(&e.to_string()),
        };
        self.memory
            .lock()
            .unwrap()
            .learn(tool, &input, &result_text(&result));
        let mut lines: Vec<String> = Vec::new();
        if result.get("isError") != Some(&Value::Bool(true)) {
            lines.extend(line);
        }
        lines.extend(
            self.collect_downloads()
                .into_iter()
                .map(|name| format!("Downloaded {name}")),
        );
        with_lines(result, &lines)
    }

    /// Ends the run's browser: the Playwright server stops, finished downloads move into the
    /// workspace, and the dot's Edge closes unless another run holds it or the person opened
    /// it. Calling it twice does nothing more.
    pub async fn close(&self) {
        let mut live = self.live.lock().await;
        if live.closed {
            return;
        }
        live.closed = true;
        if let Some(session) = live.session.take() {
            session.close().await;
        }
        if live.held {
            let _ = self.collect_downloads();
            live.held = false;
            self.browsers.release(&self.dot_id).await;
        }
    }
}

/// No `${VAR}` in the server's command line is expanded.
fn no_variables(_: &str) -> Option<String> {
    None
}

/// The text parts of a `tools/call` result, joined by new lines.
fn result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// `result` with one text part in front: each of `lines` as `Dots: <line>`.
fn with_lines(mut result: Value, lines: &[String]) -> Value {
    if lines.is_empty() {
        return result;
    }
    let text = lines
        .iter()
        .map(|line| format!("Dots: {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let part = json!({ "type": "text", "text": text });
    match result.get_mut("content").and_then(Value::as_array_mut) {
        Some(parts) => parts.insert(0, part),
        None => result["content"] = json!([part]),
    }
    result
}

/// The run's tools with the built-in browser, for a dot with `browser` on (section 2.4), when
/// the runtime gave the runner its browsers; the tools as they are otherwise. When the browser
/// is left out, the reason goes to `notices`.
pub fn attach(
    browsers: Option<&Arc<Browsers>>,
    dot: &Dot,
    run_id: &str,
    workspace: &Path,
    tools: RunTools,
    notices: &mut Vec<String>,
) -> RunTools {
    let Some(browsers) = browsers.filter(|_| dot.spec.browser) else {
        return tools;
    };
    let (tools, notice) = tools.with_browser(browsers.for_run(dot, run_id, workspace));
    notices.extend(notice);
    tools
}

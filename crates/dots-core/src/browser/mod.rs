//! Browser use for dots (browser spec `docs/superpowers/specs/2026-10-21-browser-design.md`):
//! Microsoft Edge in a visible window with the dot's own profile, driven by Microsoft's
//! Playwright MCP server, behind Dots' gate, which judges each call by the address of the page
//! it acts on (`sites`) and the labels in Playwright's own snapshots.

pub mod door;
pub mod gate;
pub mod host;
pub mod pipe;
pub mod sites;
pub mod snapshot;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use self::door::Doors;
use self::gate::{SiteRules, Verdict, ASK_KEY};
use self::host::{find_edge, find_program, move_downloads, BrowserHost, HostError};
use self::pipe::Pipe;
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

/// Whether an approval of `tool` with `input` is one the built-in browser asked for: it carries
/// Dots' own card (`gate::ASK_KEY`). A Library server named `browser` shares the tool names, and
/// its inputs never carry the card (`drop_card`).
pub fn is_browser_approval(tool: &str, input: &Value) -> bool {
    is_browser_tool(tool) && input.get(gate::ASK_KEY).is_some()
}

/// Drops a card of Dots' own from the input of a call to a Library server, where only the model
/// could have put it.
pub fn drop_card(input: &mut Value) {
    if let Some(object) = input.as_object_mut() {
        object.remove(gate::ASK_KEY);
    }
}

/// How long a call may take. The local engine gives the call itself a fixed time; the Claude
/// engine's CLI waits for the whole call (Edge and the server starting, an approval) only until
/// a moment, after which the call is not performed.
#[derive(Debug, Clone, Copy)]
pub enum CallLimit {
    PerformWithin(Duration),
    FinishBy(std::time::Instant),
}

/// The answer to a call whose time ran out before it could be performed.
pub const TOO_LATE: &str = "Not performed: the time for this call ran out before it could be performed (the browser was starting or an approval took long). Take a snapshot, then try again if it is still right.";

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

/// The variable that tells a Playwright server where its Edge is: its door,
/// `ws://127.0.0.1:<dots port>/cdp/<secret>` (section 14). A variable rather than
/// `--cdp-endpoint`, so that the secret is on no command line that any program can list.
pub const CDP_ENDPOINT_VAR: &str = "PLAYWRIGHT_MCP_CDP_ENDPOINT";

impl PlaywrightCommand {
    /// Its arguments for a run: write files into `output`, and put no page snapshot into an
    /// act's result (section 2.2; no `--caps`). Its Edge is named by `CDP_ENDPOINT_VAR`.
    pub fn args_for(&self, output: &Path) -> Vec<String> {
        let mut args = self.args.clone();
        args.extend([
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

/// Who uses a dot's Edge: the run that holds it, and the person.
#[derive(Debug, Default)]
struct Users {
    /// The run that holds the browser, from its first browser call until it ends.
    run: Option<String>,
    person: bool,
}

/// The dots' browsers (sections 2.1 and 14): one Edge per dot, serving one run of the dot at a
/// time, closed when that run ends, unless the person opened it ("Open the browser").
pub struct Browsers {
    store: Store,
    host: Arc<dyn BrowserHost>,
    playwright: PlaywrightCommand,
    /// The port of Dots' own server, where Playwright servers find their doors.
    port: u16,
    doors: Arc<Doors>,
    users: tokio::sync::Mutex<HashMap<String, Users>>,
}

impl Browsers {
    pub fn new(
        store: Store,
        host: Arc<dyn BrowserHost>,
        playwright: PlaywrightCommand,
        port: u16,
    ) -> Arc<Browsers> {
        Arc::new(Browsers {
            store,
            host,
            playwright,
            port,
            doors: Arc::new(Doors::default()),
            users: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// The doors of the runs' Playwright servers to their dots' Edges (section 14), which
    /// Dots' server serves at `/cdp/<secret>`.
    pub fn doors(&self) -> &Arc<Doors> {
        &self.doors
    }

    /// Where the Playwright server behind the door of `secret` connects.
    fn endpoint(&self, secret: &str) -> String {
        format!("ws://127.0.0.1:{}/cdp/{secret}", self.port)
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

    /// The dot's Edge for the run `run_id`: found, or started, again after the person closed
    /// it. The run holds the browser from then on; while another run holds it, `InUse`.
    async fn open(&self, dot_id: &str, run_id: &str) -> Result<Arc<Pipe>, HostError> {
        let exe = self.edge().await?;
        let mut users = self.users.lock().await;
        let user = users.entry(dot_id.to_string()).or_default();
        if user.run.as_deref().is_some_and(|holder| holder != run_id) {
            return Err(HostError::InUse);
        }
        let opened = self.host.open(dot_id, &exe).await?;
        let user = users.entry(dot_id.to_string()).or_default();
        if opened.started {
            user.person = false;
        }
        user.run = Some(run_id.to_string());
        Ok(opened.edge)
    }

    /// The run `run_id` is done with the dot's Edge: it closes, unless the person opened it.
    async fn release(&self, dot_id: &str, run_id: &str) {
        let mut users = self.users.lock().await;
        let Some(user) = users.get_mut(dot_id) else {
            return;
        };
        if user.run.as_deref() != Some(run_id) {
            return;
        }
        user.run = None;
        if !user.person {
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
    /// The Edge the session drives.
    edge: Option<Arc<Pipe>>,
    /// The secret of the session's door to it.
    secret: Option<String>,
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
    /// the first call, again after the person closed Edge (another Edge), and again after it
    /// stopped. Each server gets a door of its own to the Edge.
    async fn ready(&self) -> Result<Arc<McpSession>, String> {
        let mut live = self.live.lock().await;
        if live.closed {
            return Err("The run has ended.".to_string());
        }
        let edge = self
            .browsers
            .open(&self.dot_id, &self.run_id)
            .await
            .map_err(|e| e.to_string())?;
        live.held = true;
        if !live
            .edge
            .as_ref()
            .is_some_and(|known| Arc::ptr_eq(known, &edge))
        {
            // Another Edge: what the run knew of its pages is gone.
            *self.memory.lock().unwrap() = PageMemory::default();
            self.stop_server(&mut live).await;
            live.edge = Some(edge.clone());
        }
        // A Playwright whose connection ended cannot come in again: its secret is spent.
        let spent = live
            .secret
            .as_deref()
            .is_some_and(|secret| self.browsers.doors.spent(secret));
        if let Some(session) = live.session.clone().filter(|s| !s.is_stopped() && !spent) {
            return Ok(session);
        }
        self.stop_server(&mut live).await;
        let secret = self.browsers.doors.open(edge);
        live.secret = Some(secret.clone());
        let session = Arc::new(self.start_server(&secret).await?);
        live.session = Some(session.clone());
        Ok(session)
    }

    /// Stops the run's Playwright server, if one runs, and shuts its door.
    async fn stop_server(&self, live: &mut Live) {
        if let Some(old) = live.session.take() {
            old.close().await;
        }
        if let Some(secret) = live.secret.take() {
            self.browsers.doors.shut(&secret);
        }
    }

    /// Starts the Playwright server on the Edge behind the door of `secret`, in Dots' own
    /// folder for it.
    async fn start_server(&self, secret: &str) -> Result<McpSession, String> {
        let command = &self.browsers.playwright;
        let Some(program) = find_program(&command.program) else {
            return Err(NO_NODE.to_string());
        };
        let dir = self.browsers.host.server_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("The browser could not start: {}: {e}", dir.display()))?;
        let config = McpServerConfig::Stdio {
            command: program.to_string_lossy().into_owned(),
            args: command.args_for(&self.workspace.join("downloads")),
            cwd: Some(dir.to_string_lossy().into_owned()),
            env: BTreeMap::from([(CDP_ENDPOINT_VAR.to_string(), self.browsers.endpoint(secret))]),
        };
        let options = SessionOptions {
            cwd: Some(dir),
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
        let edge = self.live.lock().await.edge.clone()?;
        let pages = edge.pages().await.ok()?;
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
        limit: CallLimit,
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
        if tool == "browser_press_key" {
            let key = input.get("key").and_then(Value::as_str).unwrap_or_default();
            if gate::press_refused(key, self.memory.lock().unwrap().focus()) {
                return not_performed(gate::PASSWORD_REFUSAL);
            }
        }
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
        let limit = match limit {
            CallLimit::PerformWithin(limit) => limit,
            CallLimit::FinishBy(deadline) => {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left < Duration::from_secs(1) {
                    return not_performed(TOO_LATE);
                }
                left
            }
        };
        let result = match session.call_tool(tool, input.clone(), limit, cancel).await {
            Ok(result) => result,
            Err(e) => return not_performed(&e.to_string()),
        };
        self.memory
            .lock()
            .unwrap()
            .learn_result(tool, &input, &result);
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
        self.stop_server(&mut live).await;
        if live.held {
            let _ = self.collect_downloads();
            live.held = false;
            self.browsers.release(&self.dot_id, &self.run_id).await;
        }
    }
}

/// No `${VAR}` in the server's command line is expanded.
fn no_variables(_: &str) -> Option<String> {
    None
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

/// The section a thread's first prompt gains when the dot's `browser` is on (browser spec
/// section 4).
pub const PROMPT: &str = "## Browser\n\n\
You have a browser (tools named browser_*). It is a visible Microsoft Edge window with your\n\
own profile; logins you were given stay there. Take a snapshot before you act, and act on the\n\
references in it. Opening a site you have not used before, and actions such as paying,\n\
sending, deleting or submitting, wait for the person's approval. Never type a password or a\n\
code: when a page needs a login, 2FA or a CAPTCHA, call ask with \"Please <do it> in the browser\n\
window, then answer done.\" Files you download land in downloads/ in your workspace.\n";

/// The largest screenshot the run view shows.
pub const MAX_SCREENSHOT_BYTES: u64 = 16 * 1024 * 1024;

/// A screenshot the run's browser saved into `workspace/downloads` under the name Playwright
/// gives one (`page-2026-10-21T12-16-35-575Z.png`), as a `data:` URL (browser spec section 7).
/// Any other name is `Invalid`: no other file can be read through it.
pub fn screenshot(workspace: &Path, name: &str) -> crate::Result<String> {
    use base64::Engine as _;
    let kind = screenshot_kind(name).ok_or_else(|| {
        crate::Error::Invalid(format!("{name:?} is not a screenshot of the browser"))
    })?;
    let path = workspace.join("downloads").join(name);
    let size = std::fs::metadata(&path)
        .map_err(|_| crate::Error::NotFound(format!("screenshot {name}")))?
        .len();
    if size > MAX_SCREENSHOT_BYTES {
        return Err(crate::Error::Invalid(format!(
            "screenshot {name} is too large to show"
        )));
    }
    let bytes = std::fs::read(&path)?;
    Ok(format!(
        "data:image/{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// The image type of a name Playwright gives a screenshot: `page-<digits, '-', 'T', 'Z'>`
/// and `.png`, `.jpeg` or `.webp`.
fn screenshot_kind(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.rsplit_once('.')?;
    let kind = match ext {
        "png" => "png",
        "jpeg" => "jpeg",
        "webp" => "webp",
        _ => return None,
    };
    let time = stem.strip_prefix("page-")?;
    let plain = !time.is_empty()
        && time
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == 'T' || c == 'Z');
    plain.then_some(kind)
}

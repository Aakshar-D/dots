//! A run's tools (MCP/skills spec sections 5 to 7), opened by the runner before the engine
//! starts and closed when the run ends:
//!
//! - an MCP session with every server the dot uses, its plugins' servers included, each
//!   checked against its catalog at the start: a list that is not the accepted one holds the
//!   server for the run, and so does a `notifications/tools/list_changed` that brings one;
//! - the skills it reads, standalone (`pdf`) and from its plugins (`forms:fill`);
//! - for a Claude-engine run, the plugin folders it passes with `--plugin-dir`;
//! - the context collections it reads (context library spec section 5), ticked or for every
//!   dot, each with the count of its documents. Nothing is held open for them.
//!
//! What cannot be used is left out, and `open` says so in run events: a server that cannot
//! start, a folder that is missing.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use crate::context::{self, Collection};
use crate::events::Bus;
use crate::kit::{self, KitPlugin, KitSkill, PluginInfo, RunKit, SkillInfo};
use crate::library::{CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, Uses};
use crate::library_service::{record_check, refresh_plugin, skill_problem};
use crate::mcp::{McpError, McpSession, McpTool, SessionOptions};
use crate::policy::McpTrust;
use crate::store::Store;
use crate::vault::{Consumer, Grants, Resolved, Vault};
use crate::Result;

/// What `RunTools::open` needs.
pub struct OpenRequest<'a> {
    pub store: &'a Store,
    pub bus: &'a Bus,
    pub run_id: &'a str,
    pub uses: &'a Uses,
    /// The run's folder: a stdio server without a `cwd` of its own starts there.
    pub workspace: &'a Path,
    /// Where a Claude-engine run's plugin folders go (`<data_dir>/plugins`); `None` makes none.
    pub plugins_dir: Option<&'a Path>,
    /// The run's cancel token: opening stops when it fires.
    pub cancel: &'a CancellationToken,
    /// Where the dot's variables resolve, its grants, and its name for the run events
    /// (env vault spec §8).
    pub vault: &'a Vault,
    pub grants: &'a Grants,
    pub dot_name: &'a str,
}

/// A skill of the run, with the name the model calls it by.
#[derive(Debug, Clone)]
pub struct RunSkill {
    /// The Library name for a standalone skill (`pdf`); `<plugin>:<skill>` for a plugin's.
    pub name: String,
    pub info: SkillInfo,
    /// The plugin's Library name, for a plugin's skill.
    pub plugin: Option<String>,
}

#[derive(Debug)]
struct Live {
    /// The list the server gave last.
    tools: Vec<McpTool>,
    /// Every call asks, for the rest of the run (spec section 4, step 2).
    held: bool,
}

/// An open MCP session of the run.
pub struct RunServer {
    item_id: String,
    name: String,
    session: Arc<McpSession>,
    /// The accepted list's tools, each with its `readOnlyHint`.
    accepted: HashMap<String, bool>,
    accepted_fingerprint: Option<String>,
    live: Arc<Mutex<Live>>,
    /// The count of list changes the watcher has listed and checked (`McpSession::list_changed`
    /// counts the ones announced).
    handled: watch::Receiver<u64>,
}

impl RunServer {
    /// The server's Library item.
    pub fn item_id(&self) -> &str {
        &self.item_id
    }

    /// The tool prefix: tools are `mcp__<name>__<tool>`.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn session(&self) -> &Arc<McpSession> {
        &self.session
    }

    /// The tools the server lists now (the list of the run's start, or the one a list change
    /// brought), in its order.
    pub fn tools(&self) -> Vec<McpTool> {
        self.live.lock().unwrap().tools.clone()
    }

    /// Whether every call of this server asks: its list was not the accepted one at the start
    /// of the run, or changed during it.
    pub fn held(&self) -> bool {
        self.live.lock().unwrap().held
    }

    /// The accepted list's fingerprint when the run started; an approval stores it
    /// (`approvals.mcp_fingerprint`).
    pub fn accepted_fingerprint(&self) -> Option<String> {
        self.accepted_fingerprint.clone()
    }

    /// What the check of a call of `tool` knows of the server (`Policy::resolve_mcp`): held
    /// when the server is held or the accepted list does not have the tool; read-only as the
    /// accepted list says, never as the server says now.
    pub fn trust(&self, tool: &str) -> McpTrust {
        let read_only = self.accepted.get(tool).copied();
        McpTrust {
            held: self.held() || read_only.is_none(),
            read_only: read_only.unwrap_or(false),
        }
    }

    /// Waits until every list change the server announced so far has been listed and checked,
    /// so `tools` and `trust` are current: a change announced with a call's result holds the
    /// server before the next call is checked. Waits at most `mcp::ANSWER_TIMEOUT`, and not at
    /// all once the run's tools are closed. False when the wait timed out: a change is still
    /// being listed, so `tools` and `trust` may be those of the old list.
    pub async fn settled(&self) -> bool {
        let announced = *self.session.list_changed().borrow();
        let mut handled = self.handled.clone();
        let waited = tokio::time::timeout(
            crate::mcp::ANSWER_TIMEOUT,
            handled.wait_for(|n| *n >= announced),
        )
        .await;
        waited.is_ok()
    }

    /// `trust` of `tool` for the check of a call, once the server has `settled`. When it did not
    /// settle in time, the server is treated as held for this call (it asks): a list change it
    /// announced may not be on the accepted list, and the old list's trust must not decide.
    pub async fn settled_trust(&self, tool: &str) -> McpTrust {
        let settled = self.settled().await;
        let mut trust = self.trust(tool);
        if !settled {
            tracing::warn!(
                "{}: its changed tools were not listed in time; the call of {tool} asks",
                self.name
            );
            trust.held = true;
        }
        trust
    }
}

/// The run's MCP sessions, skills and plugin folders. `RunTools::default()` has none.
#[derive(Default)]
pub struct RunTools {
    servers: Vec<RunServer>,
    skills: Vec<RunSkill>,
    contexts: Vec<Collection>,
    kit: Option<Arc<RunKit>>,
    stop: CancellationToken,
    watchers: Mutex<Vec<JoinHandle<()>>>,
    /// The values the dot grants its shell (env vault spec §7), for the engine to set.
    shell_env: Vec<(String, Zeroizing<String>)>,
}

impl fmt::Debug for RunTools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunTools")
            .field(
                "servers",
                &self.servers.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
            .field(
                "skills",
                &self.skills.iter().map(|s| &s.name).collect::<Vec<_>>(),
            )
            .field(
                "contexts",
                &self.contexts.iter().map(|c| &c.name).collect::<Vec<_>>(),
            )
            .field("plugin_dirs", &self.plugin_dirs())
            .finish()
    }
}

impl RunTools {
    /// The open sessions, by server name.
    pub fn servers(&self) -> &[RunServer] {
        &self.servers
    }

    pub fn server(&self, name: &str) -> Option<&RunServer> {
        self.servers.iter().find(|s| s.name == name)
    }

    /// The server and tool of `mcp__<server>__<tool>`, when the run has that server open. A
    /// server name never holds `__`, so the name ends at the first one.
    pub fn resolve<'a>(&self, qualified: &'a str) -> Option<(&RunServer, &'a str)> {
        let (server, tool) = qualified.strip_prefix("mcp__")?.split_once("__")?;
        Some((self.server(server)?, tool))
    }

    /// `RunServer::trust` of `tool` of the open server `server`; `None` without that server.
    pub fn trust(&self, server: &str, tool: &str) -> Option<McpTrust> {
        self.server(server).map(|s| s.trust(tool))
    }

    /// The skills: the standalone ones in the dot's order, then each plugin's.
    pub fn skills(&self) -> &[RunSkill] {
        &self.skills
    }

    pub fn skill(&self, name: &str) -> Option<&RunSkill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// The context collections, by name.
    pub fn contexts(&self) -> &[Collection] {
        &self.contexts
    }

    /// The variables granted to the dot's shell, with their values: the `claude` CLI's
    /// environment, or the local engine's shell tool's.
    pub fn shell_env(&self) -> &[(String, Zeroizing<String>)] {
        &self.shell_env
    }

    /// A Claude-engine run's folders for `--plugin-dir`; empty for a local-engine run.
    pub fn plugin_dirs(&self) -> &[PathBuf] {
        match &self.kit {
            Some(kit) => &kit.plugin_dirs,
            None => &[],
        }
    }

    /// Opens the run's tools for a dot with `uses`. Returns them with the run events to record:
    /// items left out and servers held. A cancel while opening closes what was opened and
    /// returns no tools. Only a database failure is an error.
    pub async fn open(req: OpenRequest<'_>) -> Result<(RunTools, Vec<String>)> {
        let OpenRequest {
            store,
            bus,
            run_id,
            uses,
            workspace,
            plugins_dir,
            cancel,
            vault,
            grants,
            dot_name,
        } = req;
        let mut notices = Vec::new();

        // Plugins first: reading one brings its MCP server items in line.
        // Each with whether its hooks run, as stored after the reading.
        let mut plugins: Vec<(LibraryItem, PluginInfo, bool)> = Vec::new();
        let mut left_out = HashSet::new();
        for name in &uses.plugins {
            let Some(item) = store.find_library_item(LibraryKind::Plugin, name).await? else {
                continue;
            };
            match refresh_plugin(store, bus, &item).await? {
                Ok(read) => {
                    notices.extend(read.notice);
                    notices.extend(read.warnings.iter().map(|w| format!("plugin {name}: {w}")));
                    plugins.push((item, read.info, read.run_hooks));
                }
                Err(reason) => {
                    notices.push(format!("plugin {name}: {reason}"));
                    left_out.insert(item.id);
                }
            }
        }

        let used: Vec<LibraryItem> = store
            .used_mcp_servers(uses)
            .await?
            .into_iter()
            .filter(|s| s.owner_id.as_ref().is_none_or(|o| !left_out.contains(o)))
            .collect();
        // A server whose variables do not all resolve for it is left out, with the reason; its
        // catalog is the Library's and does not record a dot's grants.
        let mut items = Vec::new();
        for item in used {
            let LibraryConfig::McpServer(config) = &item.config else {
                continue;
            };
            let consumer = Consumer::Mcp(item.name.clone());
            match vault
                .resolve(grants, &consumer, &config.variables())
                .await?
            {
                Ok(env) => items.push((item, env)),
                Err(missing) => notices.push(missing.text(dot_name)),
            }
        }
        let shell_env = shell_env(vault, grants, dot_name, &mut notices).await?;
        let Some(opened) = open_sessions(&items, workspace, cancel).await else {
            return Ok((RunTools::default(), Vec::new()));
        };
        let mut tools = RunTools::default();
        tools.shell_env = shell_env;
        for ((item, _), outcome) in items.into_iter().zip(opened) {
            match outcome {
                Ok((session, listed, changes)) => {
                    let catalog = record_check(store, bus, &item.id, Ok(&listed)).await?;
                    if let Some(notice) = held_notice(&item.name, &catalog) {
                        notices.push(notice);
                    }
                    let (handled_tx, handled) = watch::channel(*changes.borrow());
                    let server = RunServer {
                        accepted: accepted_tools(&catalog),
                        accepted_fingerprint: catalog.accepted_fingerprint.clone(),
                        live: Arc::new(Mutex::new(Live {
                            tools: listed,
                            held: catalog.status != CatalogStatus::Accepted,
                        })),
                        session: Arc::new(session),
                        item_id: item.id,
                        name: item.name,
                        handled,
                    };
                    tools.watch(&server, changes, handled_tx, store, bus);
                    tools.servers.push(server);
                }
                Err(e) => {
                    record_check(store, bus, &item.id, Err(&e)).await?;
                    notices.push(format!("{}: {e}", item.name));
                }
            }
        }

        let mut kit_skills = Vec::new();
        for name in &uses.skills {
            let Some(item) = store.find_library_item(LibraryKind::Skill, name).await? else {
                continue;
            };
            let LibraryConfig::Skill(config) = &item.config else {
                continue;
            };
            let dir = PathBuf::from(&config.path);
            let read_dir = dir.clone();
            let read = tokio::task::spawn_blocking(move || kit::read_skill(&read_dir))
                .await
                .map_err(|e| crate::Error::Other(format!("reading a skill failed: {e}")))?;
            match read {
                Ok(info) => {
                    kit_skills.push(KitSkill {
                        name: item.name.clone(),
                        dir,
                    });
                    tools.skills.push(RunSkill {
                        name: item.name,
                        info,
                        plugin: None,
                    });
                }
                Err(e) => notices.push(format!("skill {name}: {}", skill_problem(&dir, &e))),
            }
        }
        for (item, info, _) in &plugins {
            for (name, skill) in info.qualified_skills(&item.name) {
                tools.skills.push(RunSkill {
                    name,
                    info: skill.clone(),
                    plugin: Some(item.name.clone()),
                });
            }
        }

        if let Some(dir) = plugins_dir {
            let kit_plugins: Vec<KitPlugin> = plugins
                .iter()
                .map(|(item, info, run_hooks)| KitPlugin {
                    name: item.name.clone(),
                    dir: info.root.clone(),
                    run_hooks: *run_hooks,
                })
                .collect();
            let (dir, run_id) = (dir.to_path_buf(), run_id.to_string());
            let made = tokio::task::spawn_blocking(move || {
                kit::prepare_run_kit(&dir, &run_id, &kit_skills, &kit_plugins)
            })
            .await
            .map_err(|e| crate::Error::Other(format!("making plugin folders failed: {e}")))?;
            match made {
                Ok(kit) => {
                    notices.extend(
                        kit.left_out
                            .iter()
                            .map(|l| format!("{} {}: {}", l.kind.label(), l.name, l.reason)),
                    );
                    tools.kit = Some(Arc::new(kit));
                }
                Err(e) => notices.push(format!("the plugin folders could not be made: {e}")),
            }
        }

        for item in store.used_collections(uses).await? {
            let LibraryConfig::Context(config) = &item.config else {
                continue;
            };
            let dir = PathBuf::from(&config.path);
            let walk_dir = dir.clone();
            let (walked, is_dir) = tokio::task::spawn_blocking(move || {
                let walked = context::walk(&walk_dir);
                let is_dir = walked.is_err() && walk_dir.is_dir();
                (walked, is_dir)
            })
            .await
            .map_err(|e| crate::Error::Other(format!("reading a collection failed: {e}")))?;
            match walked {
                Ok(walk) => tools.contexts.push(Collection {
                    name: item.name,
                    dir,
                    description: config.description.clone(),
                    documents: walk.total,
                    complete: walk.complete,
                }),
                Err(_) if !is_dir => {
                    notices.push(format!("context {}: folder not found", item.name))
                }
                Err(e) => notices.push(format!("context {}: {e}", item.name)),
            }
        }
        Ok((tools, notices))
    }

    /// Follows the server's `notifications/tools/list_changed` for the run: each one lists the
    /// tools again and records them in the catalog; a list that is not the accepted one holds
    /// the server at once (spec section 6).
    fn watch(
        &self,
        server: &RunServer,
        mut changes: watch::Receiver<u64>,
        handled: watch::Sender<u64>,
        store: &Store,
        bus: &Bus,
    ) {
        let session = server.session.clone();
        let live = server.live.clone();
        let (store, bus) = (store.clone(), bus.clone());
        let (id, name) = (server.item_id.clone(), server.name.clone());
        let stop = self.stop.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    changed = changes.changed() => if changed.is_err() { return },
                }
                let announced = *changes.borrow_and_update();
                let listed = match session.list_tools().await {
                    Ok(listed) => listed,
                    Err(e) => {
                        tracing::warn!("{name}: listing its changed tools failed: {e}");
                        if session.is_stopped() {
                            return;
                        }
                        // A server that announced a change and cannot be listed again is not
                        // trusted on the old list: it is held for the rest of the run.
                        live.lock().unwrap().held = true;
                        handled.send_replace(announced);
                        continue;
                    }
                };
                let accepted = match record_check(&store, &bus, &id, Ok(&listed)).await {
                    Ok(catalog) => catalog.status == CatalogStatus::Accepted,
                    Err(e) => {
                        tracing::warn!("{name}: recording its changed tools failed: {e}");
                        false
                    }
                };
                {
                    let mut live = live.lock().unwrap();
                    live.tools = listed;
                    live.held |= !accepted;
                }
                handled.send_replace(announced);
            }
        });
        self.watchers.lock().unwrap().push(handle);
    }

    /// Ends the run's tools: stops following list changes, closes every session (stdio servers
    /// are stopped) and deletes the plugin folders. Calling it twice does nothing more.
    pub async fn close(&self) {
        self.stop.cancel();
        for handle in std::mem::take(&mut *self.watchers.lock().unwrap()) {
            handle.abort();
        }
        let mut closing = JoinSet::new();
        for server in &self.servers {
            let session = server.session.clone();
            closing.spawn(async move { session.close().await });
        }
        while closing.join_next().await.is_some() {}
        if let Some(kit) = self.kit.clone() {
            let _ = tokio::task::spawn_blocking(move || kit.cleanup()).await;
        }
    }
}

impl Drop for RunTools {
    /// A run that ends without `close` (an error, a panic): the sessions stop as they are
    /// dropped; the watchers and the plugin folders go here.
    fn drop(&mut self) {
        self.stop.cancel();
        for handle in self.watchers.get_mut().unwrap().drain(..) {
            handle.abort();
        }
        if let Some(kit) = &self.kit {
            kit.cleanup();
        }
    }
}

/// Closes the run's tools when it is dropped, for a run that ends early through an error: the
/// close runs on its own task. The runner still closes them itself at the end of a run.
pub struct CloseOnDrop(pub Arc<RunTools>);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let tools = self.0.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { tools.close().await });
        }
    }
}

type Opened = std::result::Result<(McpSession, Vec<McpTool>, watch::Receiver<u64>), McpError>;

/// The values granted to the dot's shell; one that does not resolve is left out, with a run
/// event saying why.
async fn shell_env(
    vault: &Vault,
    grants: &Grants,
    dot_name: &str,
    notices: &mut Vec<String>,
) -> Result<Vec<(String, Zeroizing<String>)>> {
    let mut out = Vec::new();
    for name in grants.names(&Consumer::Shell) {
        let names = std::slice::from_ref(name);
        match vault.resolve(grants, &Consumer::Shell, names).await? {
            Ok(env) => {
                if let Some(value) = env.get(name) {
                    out.push((name.clone(), Zeroizing::new(value)));
                }
            }
            Err(missing) => notices.push(missing.text(dot_name)),
        }
    }
    Ok(out)
}

/// Opens a session with each server at once, its `${VAR}` read from its resolved values, and
/// lists its tools; one outcome per server, in order. `None` when `cancel` fires first: what
/// was opened is closed.
async fn open_sessions(
    items: &[(LibraryItem, Resolved)],
    workspace: &Path,
    cancel: &CancellationToken,
) -> Option<Vec<Opened>> {
    let mut set = JoinSet::new();
    for (i, (item, env)) in items.iter().enumerate() {
        let LibraryConfig::McpServer(config) = item.config.clone() else {
            continue;
        };
        let env = env.clone();
        let name = item.name.clone();
        let options = SessionOptions {
            cwd: Some(workspace.to_path_buf()),
            ..Default::default()
        };
        let cancel = cancel.clone();
        set.spawn(async move {
            let open = async {
                let session = McpSession::open(&name, &config, &|k| env.get(k), options).await?;
                let changes = session.list_changed();
                match session.list_tools().await {
                    Ok(listed) => Ok((session, listed, changes)),
                    Err(e) => {
                        session.close().await;
                        Err(e)
                    }
                }
            };
            tokio::select! {
                opened = open => (i, Some(opened)),
                _ = cancel.cancelled() => (i, None),
            }
        });
    }
    let mut outcomes: Vec<Option<Opened>> = (0..items.len()).map(|_| None).collect();
    let mut cancelled = false;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((i, Some(opened))) => outcomes[i] = Some(opened),
            Ok((_, None)) => cancelled = true,
            Err(e) => tracing::error!("opening an MCP server failed: {e}"),
        }
    }
    if cancelled || cancel.is_cancelled() {
        for (session, _, _) in outcomes.into_iter().flatten().flatten() {
            session.close().await;
        }
        return None;
    }
    Some(
        outcomes
            .into_iter()
            .map(|o| {
                o.unwrap_or_else(|| Err(McpError::Start("the server could not be opened".into())))
            })
            .collect(),
    )
}

/// The accepted list's tools with their read-only hint.
fn accepted_tools(catalog: &McpCatalog) -> HashMap<String, bool> {
    catalog
        .accepted_tools
        .clone()
        .and_then(|v| serde_json::from_value::<Vec<McpTool>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.name.clone(), t.read_only()))
        .collect()
}

fn held_notice(name: &str, catalog: &McpCatalog) -> Option<String> {
    if catalog.status == CatalogStatus::Accepted {
        return None;
    }
    Some(match catalog.accepted_fingerprint {
        Some(_) => format!(
            "{name}: its tool list changed since it was accepted; its tools ask until the \
             change is accepted in the Library"
        ),
        None => format!(
            "{name}: its tool list has not been accepted yet; its tools ask until it is \
             accepted in the Library"
        ),
    })
}

/// Deletes the plugin folders of runs (`run-*` in `plugins_dir`) that a crash left behind.
/// Call it when no run is running.
pub fn sweep_plugin_folders(plugins_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_run = entry.file_name().to_string_lossy().starts_with("run-")
            && entry.file_type().is_ok_and(|t| t.is_dir());
        if is_run {
            if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                tracing::warn!("could not delete {}: {e}", entry.path().display());
            }
        }
    }
}

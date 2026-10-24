//! The Library at runtime (MCP/skills spec sections 3 and 9): what the app's `library_*`
//! commands do. It adds, changes and removes items (asking first when dots use one), tests an
//! MCP server config, checks a server and accepts its list with the fingerprint guard, keeps a
//! plugin's own MCP server items in line with its folder, scans a folder for skills, lists the
//! installed Claude Code plugins, and says which variables are set. Every change of an item or
//! a catalog is published as `RuntimeEvent::LibraryUpdated`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::context::{self, Document};
use crate::events::{Bus, RuntimeEvent};
use crate::kit::{self, InstalledPlugin, PluginInfo};
use crate::library::{
    check_name, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
    SkillConfig, PERSONAL_SCOPE,
};
use crate::mcp::{fingerprint, McpError, McpSession, McpTool, SessionOptions};
use crate::model::{DotRef, ToolsChanged};
use crate::skill_creator::{self, check_draft};
use crate::store::Store;
use crate::vault::literals::Prior;
use crate::vault::{Consumer, DotScope, Resolved, Vault};
use crate::{Error, Result};

/// Dots' own environment, where Claude Code's plugin list is looked up (`kit::installed`).
/// `${VAR}` is never read here: it resolves from the vault (env vault spec §8).
pub(crate) fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// One Library item as the Library screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryEntry {
    #[serde(flatten)]
    pub item: LibraryItem,
    /// The dots that use it; for a plugin's MCP server, the dots that use the plugin.
    pub used_by: Vec<DotRef>,
    /// An MCP server's Personal catalog.
    pub catalog: Option<McpCatalog>,
    /// The `${VAR}` of an MCP server's config that Dots' environment does not set.
    pub missing_variables: Vec<String>,
    /// A skill read from its folder.
    pub skill: Option<SkillDetail>,
    /// A plugin read from its folder.
    pub plugin: Option<PluginDetail>,
    /// A context collection read from its folder.
    pub context: Option<ContextDetail>,
    /// Why a skill, plugin or context folder could not be read ("folder not found").
    pub problem: Option<String>,
    /// A skill whose folder is in Dots' skills folder: removing it deletes the folder too
    /// (skill creator spec §7).
    pub dots_folder: bool,
}

/// What a context collection's folder holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextDetail {
    pub documents: usize,
    /// False when the walk stopped at its bound: `documents` is a lower bound.
    pub complete: bool,
}

/// The first documents of a collection (`context_documents`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextDocuments {
    pub documents: Vec<Document>,
    /// All documents found, up to `context::MAX_DOCUMENTS` and past it.
    pub total: usize,
    /// More documents than are listed here, or the walk stopped at its bound.
    pub truncated: bool,
    /// False when the walk stopped at its bound: `total` is a lower bound.
    pub complete: bool,
}

impl ContextDocuments {
    fn new(documents: Vec<Document>, total: usize, complete: bool) -> Self {
        Self {
            truncated: total > documents.len() || !complete,
            documents,
            total,
            complete,
        }
    }
}

/// How many documents `context_documents` describes.
pub const SHOWN_DOCUMENTS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDetail {
    pub description: String,
}

/// A plugin's parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginDetail {
    pub version: Option<String>,
    pub description: Option<String>,
    /// Qualified: `<plugin>:<skill>`.
    pub skills: Vec<String>,
    pub commands: Vec<String>,
    pub agents: Vec<String>,
    /// The names of its MCP server items.
    pub servers: Vec<String>,
    pub has_hooks: bool,
    /// What could not be read in it.
    pub warnings: Vec<String>,
}

/// What a server config lists (`library_probe`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct McpProbe {
    pub tools: Vec<McpTool>,
    pub fingerprint: String,
}

/// A skill found by `library_scan_skills`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScannedSkill {
    pub name: String,
    pub description: String,
    pub path: String,
    /// Why `name` cannot be a Library name; the person picks another.
    pub name_error: Option<String>,
}

/// Whether Personal has a variable. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvVariable {
    pub name: String,
    pub set: bool,
}

pub(crate) fn publish(bus: &Bus, item_id: &str) {
    let _ = bus.send(RuntimeEvent::LibraryUpdated {
        item_id: item_id.to_string(),
    });
}

/// Opens a session with the server and lists its tools. The session is closed again when the
/// listing fails.
pub(crate) async fn open_and_list(
    name: &str,
    config: &McpServerConfig,
    env: &Resolved,
    options: SessionOptions,
) -> std::result::Result<(McpSession, Vec<McpTool>), McpError> {
    let session = McpSession::open(name, config, &|k| env.get(k), options).await?;
    match session.list_tools().await {
        Ok(tools) => Ok((session, tools)),
        Err(e) => {
            session.close().await;
            Err(e)
        }
    }
}

/// Writes the outcome of a check to the server's catalog in `scope` (`personal`, or the org id
/// of the dot or the Check) and publishes the change.
pub(crate) async fn record_check(
    store: &Store,
    bus: &Bus,
    server_id: &str,
    scope: &str,
    outcome: std::result::Result<&[McpTool], &McpError>,
) -> Result<McpCatalog> {
    let catalog = match outcome {
        Ok(tools) => {
            let listed = serde_json::to_value(tools)?;
            store
                .record_catalog_seen(server_id, scope, &fingerprint(tools), &listed)
                .await?
        }
        Err(e) => {
            store
                .record_catalog_failure(server_id, scope, &e.to_string())
                .await?
        }
    };
    publish(bus, server_id);
    Ok(catalog)
}

/// Why the skill folder `dir` cannot be used, in the words of a run event.
pub(crate) fn skill_problem(dir: &Path, e: &Error) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else if !dir.join("SKILL.md").is_file() {
        "SKILL.md not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

fn context_problem(dir: &Path, e: &str) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else {
        e.to_string()
    }
}

fn plugin_problem(dir: &Path, e: &Error) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("reading the folder failed: {e}")))
}

fn plugin_config(item: &LibraryItem) -> Result<&PluginConfig> {
    match &item.config {
        LibraryConfig::Plugin(c) => Ok(c),
        _ => Err(Error::Invalid(format!("{:?} is not a plugin", item.name))),
    }
}

/// Serialises `sync_plugin_servers` (one lock for all plugins, which are few and synced rarely).
static PLUGIN_SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The reason given when a plugin leaves the Library while a run is opening it.
const GONE: &str = "removed from the Library";

/// Brings the plugin's MCP server items in line with what its folder lists (spec section 2):
/// new servers are added, changed ones updated (their catalog goes back to `unchecked`),
/// servers no longer listed removed. A server whose item name the Library already has for a
/// server of its own is left out, with a warning.
pub(crate) async fn sync_plugin_servers(
    store: &Store,
    bus: &Bus,
    vault: &Vault,
    plugin: &LibraryItem,
    info: &PluginInfo,
) -> Result<Vec<String>> {
    // One sync at a time for all plugins: two runs opening the same plugin must not both act on
    // the same stale list of owned items.
    let _turn = PLUGIN_SYNC.lock().await;
    // The plugin may have been removed since it was read: then there is nothing to keep in line.
    store.get_library_item(&plugin.id).await?;
    let mut warnings = Vec::new();
    let wanted = info.server_items(&plugin.name);
    let existing = store.owned_library_items(&plugin.id).await?;
    for old in &existing {
        if !wanted.iter().any(|(name, _)| *name == old.name) {
            match store.remove_library_item(&old.id).await {
                Ok(()) | Err(Error::NotFound(_)) => publish(bus, &old.id),
                Err(e) => return Err(e),
            }
        }
    }
    for (name, config) in wanted {
        let old = existing.iter().find(|i| i.name == name);
        // A secret written into the plugin's `.mcp.json` moves into Personal; the Library keeps
        // `${NAME}` (env vault spec §11). It stays in the plugin's own folder. A key the stored
        // server already reads as `${NAME}` keeps it, and Personal keeps its value.
        let prior = match old.map(|i| &i.config) {
            Some(LibraryConfig::McpServer(stored)) => Prior::Plugin(stored),
            _ => Prior::New,
        };
        let moved = vault.move_literals_over(&name, &config, prior).await?.0;
        let config = LibraryConfig::McpServer(moved);
        match old {
            Some(old) if old.config == config => {}
            Some(old) => match store.update_library_item(&old.id, &config).await {
                Ok(_) | Err(Error::NotFound(_)) => publish(bus, &old.id),
                Err(Error::Invalid(m)) => {
                    warnings.push(format!("its MCP server {name} is left out: {m}"))
                }
                Err(e) => return Err(e),
            },
            None => match store
                .add_library_item(&name, &config, Some(&plugin.id))
                .await
            {
                Ok(item) => publish(bus, &item.id),
                Err(Error::Conflict(_)) => {
                    let ours = store
                        .find_library_item(LibraryKind::McpServer, &name)
                        .await?
                        .is_some_and(|i| i.owner_id.as_deref() == Some(plugin.id.as_str()));
                    if !ours {
                        warnings.push(format!(
                            "its MCP server {name} is left out: the Library already has an MCP \
                             server of that name"
                        ));
                    }
                }
                // A server the Library refuses (a URL the parser would read differently) leaves
                // that server out, not the plugin.
                Err(Error::Invalid(m)) => {
                    warnings.push(format!("its MCP server {name} is left out: {m}"))
                }
                Err(e) => return Err(e),
            },
        }
    }
    Ok(warnings)
}

/// A plugin read again at a run's start (`refresh_plugin`).
#[derive(Debug)]
pub(crate) struct RefreshedPlugin {
    pub info: PluginInfo,
    /// What the run reports about the plugin, each to be prefixed with its name.
    pub warnings: Vec<String>,
    /// A run event of its own: the plugin moved and its hooks were switched off.
    pub notice: Option<String>,
    /// Whether the plugin's hooks run, as stored now.
    pub run_hooks: bool,
}

/// Reads the plugin's folder again, as a run does at its start (spec section 6). An installed
/// plugin whose folder is gone is looked up in Claude Code's list again, and its new folder is
/// stored; its hooks are switched off, since the folder they would run from is a new one, with
/// a notice when they were on. Its MCP server items are brought in line. `Ok(Err(reason))`
/// when the folder cannot be used: "folder not found" or "could not be read: ...".
pub(crate) async fn refresh_plugin(
    store: &Store,
    bus: &Bus,
    vault: &Vault,
    item: &LibraryItem,
) -> Result<std::result::Result<RefreshedPlugin, String>> {
    let mut config = plugin_config(item)?.clone();
    let mut notice = None;
    if !Path::new(&config.path).is_dir() {
        let moved = config.installed_id.clone().and_then(|id| {
            kit::find_installed(&process_env, &id)
                .filter(|p| p.exists)
                .map(|p| p.install_path)
        });
        if let Some(path) = moved {
            config.path = path.to_string_lossy().into_owned();
            if config.run_hooks {
                config.run_hooks = false;
                notice = Some(format!(
                    "the plugin {} moved, so its hooks were switched off; switch them on again \
                     in the Library",
                    item.name
                ));
            }
            match store
                .update_library_item(&item.id, &LibraryConfig::Plugin(config.clone()))
                .await
            {
                Ok(_) => publish(bus, &item.id),
                Err(Error::NotFound(_)) => return Ok(Err(GONE.into())),
                Err(e) => return Err(e),
            }
        }
    }
    let root = PathBuf::from(&config.path);
    let read = blocking(move || {
        let info = kit::read_plugin(&root);
        info.map_err(|e| plugin_problem(&root, &e))
    })
    .await?;
    let info = match read {
        Ok(info) => info,
        Err(reason) => return Ok(Err(reason)),
    };
    let mut warnings = info.warnings.clone();
    match sync_plugin_servers(store, bus, vault, item, &info).await {
        Ok(more) => warnings.extend(more),
        Err(Error::NotFound(_)) => return Ok(Err(GONE.into())),
        Err(e) => return Err(e),
    }
    Ok(Ok(RefreshedPlugin {
        info,
        warnings,
        notice,
        run_hooks: config.run_hooks,
    }))
}

pub struct LibraryService {
    store: Store,
    bus: Bus,
    skills_dir: Option<PathBuf>,
    /// Where a probe, a check and the missing variables read `${VAR}`: Personal.
    vault: Vault,
}

impl LibraryService {
    pub fn new(store: Store, bus: Bus) -> LibraryService {
        LibraryService {
            vault: Vault::in_memory(store.clone()),
            store,
            bus,
            skills_dir: None,
        }
    }

    /// The vault whose Personal variables `${VAR}` reads (env vault spec §8).
    pub fn with_vault(mut self, vault: Vault) -> LibraryService {
        self.vault = vault;
        self
    }

    /// Where the skill creator saves skills (`<data>/skills`, skill creator spec §6). Without
    /// it, `create_skill` fails and no skill is Dots' own.
    pub fn with_skills_dir(mut self, dir: PathBuf) -> LibraryService {
        self.skills_dir = Some(dir);
        self
    }

    /// Whether the skill folder `path` is in Dots' skills folder.
    fn is_own_folder(skills_dir: Option<&Path>, path: &Path) -> bool {
        skills_dir.is_some_and(|root| skill_creator::is_inside(path, root))
    }

    /// Every item, with the dots that use it, a server's Personal catalog and missing
    /// variables, and what a skill's or plugin's folder holds (`library_list`).
    pub async fn list(&self) -> Result<Vec<LibraryEntry>> {
        let items = self.store.list_library_items().await?;
        let mut catalogs: HashMap<String, McpCatalog> = self
            .store
            .list_catalogs()
            .await?
            .into_iter()
            .filter(|(scope, _)| scope == PERSONAL_SCOPE)
            .map(|(_, c)| (c.server_id.clone(), c))
            .collect();
        let folders: Vec<(LibraryKind, String)> = items
            .iter()
            .map(|i| match &i.config {
                LibraryConfig::Skill(c) => (LibraryKind::Skill, c.path.clone()),
                LibraryConfig::Plugin(c) => (LibraryKind::Plugin, c.path.clone()),
                LibraryConfig::McpServer(_) => (LibraryKind::McpServer, String::new()),
                LibraryConfig::Context(c) => (LibraryKind::Context, c.path.clone()),
            })
            .collect();
        let skills_dir = self.skills_dir.clone();
        let read = blocking(move || {
            folders
                .into_iter()
                .map(|(kind, path)| {
                    let own = kind == LibraryKind::Skill
                        && Self::is_own_folder(skills_dir.as_deref(), Path::new(&path));
                    let dir = PathBuf::from(&path);
                    let read = match kind {
                        LibraryKind::Skill => Some(
                            kit::read_skill(&dir)
                                .map(|s| (Some(s), None, None))
                                .map_err(|e| skill_problem(&dir, &e)),
                        ),
                        LibraryKind::Plugin => Some(
                            kit::read_plugin(&dir)
                                .map(|p| (None, Some(p), None))
                                .map_err(|e| plugin_problem(&dir, &e)),
                        ),
                        LibraryKind::Context => Some(
                            context::walk(&dir)
                                .map(|w| (None, None, Some((w.total, w.complete))))
                                .map_err(|e| context_problem(&dir, &e)),
                        ),
                        LibraryKind::McpServer => None,
                    };
                    (read, own)
                })
                .collect::<Vec<_>>()
        })
        .await?;
        let mut entries = Vec::new();
        for (item, (read, dots_folder)) in items.iter().zip(read) {
            let used_by = self
                .store
                .dots_using(item)
                .await?
                .into_iter()
                .map(|d| DotRef {
                    id: d.id,
                    name: d.spec.name,
                })
                .collect();
            let mut entry = LibraryEntry {
                item: item.clone(),
                used_by,
                catalog: catalogs.remove(&item.id),
                missing_variables: Vec::new(),
                skill: None,
                plugin: None,
                context: None,
                problem: None,
                dots_folder,
            };
            if let LibraryConfig::McpServer(config) = &item.config {
                let variables = config.variables();
                let personal = self.vault.personal_values(&variables).await?;
                entry.missing_variables = variables
                    .into_iter()
                    .filter(|v| !personal.contains(v))
                    .collect();
            }
            match read {
                Some(Ok((skill, plugin, documents))) => {
                    entry.context = documents.map(|(documents, complete)| ContextDetail {
                        documents,
                        complete,
                    });
                    entry.skill = skill.map(|s| SkillDetail {
                        description: s.description,
                    });
                    entry.plugin = plugin.map(|p| PluginDetail {
                        skills: p
                            .qualified_skills(&item.name)
                            .into_iter()
                            .map(|(name, _)| name)
                            .collect(),
                        servers: items
                            .iter()
                            .filter(|i| i.owner_id.as_deref() == Some(item.id.as_str()))
                            .map(|i| i.name.clone())
                            .collect(),
                        version: p.version,
                        description: p.description,
                        commands: p.commands,
                        agents: p.agents,
                        has_hooks: p.has_hooks,
                        warnings: p.warnings,
                    });
                }
                Some(Err(problem)) => entry.problem = Some(problem),
                None => {}
            }
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Adds an item (`library_add`). A skill or plugin folder must be readable. A plugin's MCP
    /// servers become items of their own. An MCP server is not started unless `accept` is
    /// given: then it is checked, and the list it gives is accepted if its fingerprint is
    /// `accept`, the one the person was shown by `probe`. Otherwise the list waits for review
    /// (status `changed`), or the server stays `unchecked`.
    pub async fn add(
        &self,
        name: &str,
        config: LibraryConfig,
        accept: Option<&str>,
    ) -> Result<LibraryItem> {
        if accept.is_some() && config.kind() != LibraryKind::McpServer {
            return Err(Error::Invalid(
                "only an MCP server has a tool list to accept".into(),
            ));
        }
        let plugin = self.read_folder(&config).await?;
        let config = self.without_literals(name, config, None).await?;
        let item = self.store.add_library_item(name, &config, None).await?;
        publish(&self.bus, &item.id);
        if let Some(info) = plugin {
            let synced = sync_plugin_servers(&self.store, &self.bus, &self.vault, &item, &info);
            for warning in synced.await? {
                tracing::warn!("plugin {name}: {warning}");
            }
        }
        if let Some(fp) = accept {
            let catalog = self.check_item(&item, &DotScope::Personal).await?;
            if catalog.seen_fingerprint.as_deref() == Some(fp) {
                self.store
                    .accept_catalog(&item.id, PERSONAL_SCOPE, fp)
                    .await?;
                publish(&self.bus, &item.id);
            }
        }
        Ok(item)
    }

    /// Changes an item's config (`library_update`). A changed MCP server goes back to
    /// `unchecked`. A plugin's folder is read again and its MCP servers brought in line. A
    /// plugin's own MCP server is changed through its plugin only.
    pub async fn update(&self, id: &str, config: LibraryConfig) -> Result<LibraryItem> {
        let old = self.store.get_library_item(id).await?;
        self.refuse_owned(&old, "change")?;
        let plugin = self.read_folder(&config).await?;
        let config = self.without_literals(&old.name, config, Some(&old)).await?;
        let item = self.store.update_library_item(id, &config).await?;
        if item.config != old.config {
            publish(&self.bus, id);
        }
        if let Some(info) = plugin {
            let synced = sync_plugin_servers(&self.store, &self.bus, &self.vault, &item, &info);
            for warning in synced.await? {
                tracing::warn!("plugin {}: {warning}", item.name);
            }
        }
        Ok(item)
    }

    /// Removes an item (`library_remove`). When dots use it, it is refused (`Conflict`, naming
    /// them) unless `confirm`; then its name is taken out of their `uses`. A plugin's MCP
    /// servers go with it; a plugin's server cannot be removed on its own.
    pub async fn remove(&self, id: &str, confirm: bool) -> Result<()> {
        let item = self.store.get_library_item(id).await?;
        self.refuse_owned(&item, "remove")?;
        let users = self.store.dots_using(&item).await?;
        if !users.is_empty() && !confirm {
            let names: Vec<String> = users.into_iter().map(|d| d.spec.name).collect();
            return Err(Error::Conflict(format!(
                "the {} {:?} is used by {}; removing it takes it out of them",
                item.kind().label(),
                item.name,
                names.join(", ")
            )));
        }
        // Dots' own skill takes its folder with it, first: a folder that cannot be deleted
        // keeps the row, so its name is never left taken by a folder (skill creator spec §7).
        if let LibraryConfig::Skill(c) = &item.config {
            let skills_dir = self.skills_dir.clone();
            let dir = PathBuf::from(&c.path);
            // Another row on the same folder keeps it: only this row goes.
            let others: Vec<PathBuf> = self
                .store
                .list_library_items()
                .await?
                .into_iter()
                .filter(|i| i.id != item.id)
                .filter_map(|i| match i.config {
                    LibraryConfig::Skill(c) => Some(PathBuf::from(c.path)),
                    _ => None,
                })
                .collect();
            blocking(move || {
                let shared = dir.canonicalize().is_ok_and(|mine| {
                    others
                        .iter()
                        .any(|o| o.canonicalize().is_ok_and(|o| o == mine))
                });
                if !shared && Self::is_own_folder(skills_dir.as_deref(), &dir) {
                    skill_creator::delete_skill_folder(&dir)
                } else {
                    Ok(())
                }
            })
            .await??;
        }
        let owned = self.store.owned_library_items(id).await?;
        self.store.remove_library_item(id).await?;
        publish(&self.bus, id);
        for server in owned {
            publish(&self.bus, &server.id);
        }
        Ok(())
    }

    /// Starts or reaches the server of `config` and lists its tools, without saving anything
    /// (`library_probe`). The error is the text the person sees ("could not start: SF_TOKEN is
    /// not set").
    pub async fn probe(&self, config: &McpServerConfig) -> Result<McpProbe> {
        let env = self.vault.personal_values(&config.variables()).await?;
        let (session, tools) =
            open_and_list("probe", config, &env, SessionOptions::default()).await?;
        session.close().await;
        Ok(McpProbe {
            fingerprint: fingerprint(&tools),
            tools,
        })
    }

    /// Checks the MCP server `id` now in `scope` (`library_check`, "Check now", "Check as"):
    /// its list is recorded as seen in that scope's catalog, and the status is `accepted`,
    /// `changed` or `unreachable`. Personal starts the server with Personal's values. An org
    /// resolves each of its variables as a run in that org would, as if all were granted
    /// (env vault spec §8); one that does not resolve fails the check, and the catalog's error
    /// names the variable and the org. An org not joined on this PC is `NotFound`.
    pub async fn check(&self, id: &str, scope: &DotScope) -> Result<McpCatalog> {
        let item = self.server(id).await?;
        self.check_item(&item, scope).await
    }

    /// Accepts the list last seen in `scope` of the MCP server `id`, if its fingerprint is
    /// still `fingerprint` (`library_accept`); a newer list is a `Conflict`.
    pub async fn accept(
        &self,
        id: &str,
        scope: &DotScope,
        fingerprint: &str,
    ) -> Result<McpCatalog> {
        self.server(id).await?;
        let catalog = self
            .store
            .accept_catalog(id, scope.key(), fingerprint)
            .await?;
        publish(&self.bus, id);
        Ok(catalog)
    }

    /// Saves a drafted `SKILL.md` as a new skill (`skill_create`, skill creator spec §6): to
    /// `<skills folder>/<name>/SKILL.md`, then into the Library. A name the Library has for a
    /// skill, or a folder of that name, is refused, so nothing is overwritten; a failed add
    /// removes the folder again.
    pub async fn create_skill(&self, text: &str) -> Result<LibraryItem> {
        let summary = check_draft(text)?;
        let Some(root) = self.skills_dir.clone() else {
            return Err(Error::Other("Dots has no skills folder".into()));
        };
        let name = summary.name;
        if self
            .store
            .find_library_item(LibraryKind::Skill, &name)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "A skill named {name} is already in the Library."
            )));
        }
        let (folder_name, body) = (name.clone(), text.to_string());
        let dir = blocking(move || skill_creator::write_skill_folder(&root, &folder_name, &body))
            .await??;
        let config = LibraryConfig::Skill(SkillConfig {
            path: dir.to_string_lossy().into_owned(),
        });
        match self.add(&name, config, None).await {
            Ok(item) => Ok(item),
            Err(e) => {
                let _ = blocking(move || std::fs::remove_dir_all(&dir)).await;
                Err(e)
            }
        }
    }

    /// The skills under `path` (`library_scan_skills`).
    pub async fn scan_skills(&self, path: &str) -> Result<Vec<ScannedSkill>> {
        let root = PathBuf::from(path);
        let found = blocking(move || kit::scan_skills(&root)).await??;
        Ok(found
            .into_iter()
            .map(|s| ScannedSkill {
                name_error: check_name(LibraryKind::Skill, &s.name)
                    .err()
                    .map(|e| match e {
                        Error::Invalid(text) => text,
                        other => other.to_string(),
                    }),
                path: s.dir.to_string_lossy().into_owned(),
                name: s.name,
                description: s.description,
            })
            .collect())
    }

    /// The plugins Claude Code has installed (`installed_claude_plugins`).
    pub fn installed_plugins(&self) -> Vec<InstalledPlugin> {
        kit::installed_plugins(&process_env)
    }

    /// Which of `names` Personal has (`environment_variables`).
    pub async fn environment_variables(&self, names: &[String]) -> Result<Vec<EnvVariable>> {
        let personal = self.vault.personal_values(names).await?;
        Ok(names
            .iter()
            .map(|name| EnvVariable {
                name: name.clone(),
                set: personal.contains(name),
            })
            .collect())
    }

    /// The MCP servers whose tool list waits for review, with the dots that use them (the
    /// Inbox's "Tools changed").
    pub async fn tools_changed(&self) -> Result<Vec<ToolsChanged>> {
        let mut out = Vec::new();
        for item in self.store.changed_mcp_servers(PERSONAL_SCOPE).await? {
            let dots = self
                .store
                .dots_using(&item)
                .await?
                .into_iter()
                .map(|d| DotRef {
                    id: d.id,
                    name: d.spec.name,
                })
                .collect();
            out.push(ToolsChanged {
                server_id: item.id,
                name: item.name,
                dots,
            });
        }
        Ok(out)
    }

    async fn server(&self, id: &str) -> Result<LibraryItem> {
        let item = self.store.get_library_item(id).await?;
        if item.kind() != LibraryKind::McpServer {
            return Err(Error::Invalid(format!(
                "{:?} is a {}, not an MCP server",
                item.name,
                item.kind().label()
            )));
        }
        Ok(item)
    }

    async fn check_item(&self, item: &LibraryItem, scope: &DotScope) -> Result<McpCatalog> {
        let LibraryConfig::McpServer(config) = &item.config else {
            return Err(Error::Invalid(format!(
                "{:?} is not an MCP server",
                item.name
            )));
        };
        let consumer = Consumer::Mcp(item.name.clone());
        let resolved = self
            .vault
            .check_values(scope, &consumer, &config.variables())
            .await?;
        let env = match resolved {
            Ok(env) => env,
            Err(missing) => {
                let catalog = self
                    .store
                    .record_catalog_failure(&item.id, scope.key(), &missing.check_text())
                    .await?;
                publish(&self.bus, &item.id);
                return Ok(catalog);
            }
        };
        let key = scope.key();
        match open_and_list(&item.name, config, &env, SessionOptions::default()).await {
            Ok((session, tools)) => {
                session.close().await;
                record_check(&self.store, &self.bus, &item.id, key, Ok(&tools)).await
            }
            Err(e) => record_check(&self.store, &self.bus, &item.id, key, Err(&e)).await,
        }
    }

    /// An MCP server's config with its secret-looking values moved into Personal (env vault
    /// spec §11): the Library never stores them. `old` is the item being changed: a key it
    /// read as `${NAME}` keeps the name when nothing else reads it, and the typed value
    /// replaces NAME's in Personal.
    async fn without_literals(
        &self,
        name: &str,
        config: LibraryConfig,
        old: Option<&LibraryItem>,
    ) -> Result<LibraryConfig> {
        let LibraryConfig::McpServer(server) = config else {
            return Ok(config);
        };
        let prior = match old {
            Some(LibraryItem {
                id,
                config: LibraryConfig::McpServer(stored),
                ..
            }) => Prior::Edit { id, config: stored },
            _ => Prior::New,
        };
        let moved = self.vault.move_literals_over(name, &server, prior).await?.0;
        Ok(LibraryConfig::McpServer(moved))
    }

    fn refuse_owned(&self, item: &LibraryItem, verb: &str) -> Result<()> {
        if item.owner_id.is_some() {
            return Err(Error::Invalid(format!(
                "the MCP server {:?} comes with a plugin; {verb} the plugin instead",
                item.name
            )));
        }
        Ok(())
    }

    /// Reads a skill's, plugin's or context collection's folder before it is stored: `NotFound`
    /// when it is missing.
    /// A plugin's reading is returned, for its MCP servers.
    async fn read_folder(&self, config: &LibraryConfig) -> Result<Option<PluginInfo>> {
        match config {
            LibraryConfig::Skill(c) => {
                let dir = PathBuf::from(&c.path);
                blocking(move || kit::read_skill(&dir)).await??;
                Ok(None)
            }
            LibraryConfig::Plugin(c) => {
                let dir = PathBuf::from(&c.path);
                Ok(Some(blocking(move || kit::read_plugin(&dir)).await??))
            }
            LibraryConfig::Context(c) => {
                let dir = PathBuf::from(&c.path);
                if !blocking(move || dir.is_dir()).await? {
                    return Err(Error::NotFound(format!(
                        "the folder {} was not found",
                        c.path
                    )));
                }
                Ok(None)
            }
            LibraryConfig::McpServer(_) => Ok(None),
        }
    }

    /// The first `SHOWN_DOCUMENTS` documents of the context collection `id`, each with its
    /// title and summary (`context_documents`).
    pub async fn context_documents(&self, id: &str) -> Result<ContextDocuments> {
        let item = self.store.get_library_item(id).await?;
        let LibraryConfig::Context(config) = &item.config else {
            return Err(Error::Invalid(format!(
                "the {} {:?} is not a context collection",
                item.kind().label(),
                item.name
            )));
        };
        let dir = PathBuf::from(&config.path);
        let (documents, total, complete) =
            blocking(move || context::documents(&dir, SHOWN_DOCUMENTS))
                .await?
                .map_err(Error::NotFound)?;
        Ok(ContextDocuments::new(documents, total, complete))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::new_bus;

    #[test]
    fn a_count_that_is_only_a_lower_bound_is_marked_truncated() {
        let doc = || Document {
            path: "a.md".into(),
            title: "A".into(),
            summary: String::new(),
        };
        let all = ContextDocuments::new(vec![doc()], 1, true);
        assert!(!all.truncated && all.complete);
        let more = ContextDocuments::new(vec![doc()], 2, true);
        assert!(more.truncated);
        let bound = ContextDocuments::new(vec![doc()], 1, false);
        assert!(bound.truncated && !bound.complete);
    }

    #[tokio::test]
    async fn a_plugin_removed_while_a_run_reads_it_is_left_out_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
        let bus = new_bus();
        let root = dir.path().join("forms");
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"forms"}"#,
        )
        .unwrap();
        let config = LibraryConfig::Plugin(PluginConfig {
            path: root.to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        });
        let item = store
            .add_library_item("forms", &config, None)
            .await
            .unwrap();
        // The person removes it after the run read the item.
        store.remove_library_item(&item.id).await.unwrap();
        let vault = Vault::in_memory(store.clone());
        let out = refresh_plugin(&store, &bus, &vault, &item).await.unwrap();
        assert_eq!(out.unwrap_err(), GONE);
    }
}

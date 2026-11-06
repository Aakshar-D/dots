//! Connectors (connectors spec): the apps of `catalog` connected with a pasted key or a sign-in.
//! A connected app is an ordinary Library MCP server named after the connector whose config is
//! the catalog's template; its values are Personal variables named `CONNECTOR_<ID>_<FIELD>`,
//! and a sign-in's grant is kept in the secret store under `connector/<id>/oauth`. Everything
//! goes through the Library, the vault and `Runtime::update_dot`, so the rules for any server,
//! variable and dot hold here too.

pub mod catalog;

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub use catalog::{
    catalog, find, AuthKind, Category, Connector, FieldFormat, Instructions, KeyField, RuntimeNeed,
    ServerTemplate, Trust,
};

use crate::library::{CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpServerConfig};
use crate::library_service::LibraryEntry;
use crate::mcp::oauth::{self, OAuthGrant, RefreshFailure};
use crate::model::DotRef;
use crate::vault::service::Parts;
use crate::vault::{DotScope, EntryKind, Vault, MAX_SECRET_BYTES};
use crate::{Error, Result, Runtime};

str_enum!(ConnectorState {
    NotConnected => "not_connected",
    Connected => "connected",
    NeedsSignIn => "needs_sign_in",
    Error => "error",
    NameTaken => "name_taken",
    ComingSoon => "coming_soon",
});

/// A token is refreshed when it has less than this left.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// One connector as the Connectors screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorEntry {
    #[serde(flatten)]
    pub connector: Connector,
    pub state: ConnectorState,
    /// The Library item of a connected connector.
    pub server_id: Option<String>,
    /// The dots that use it.
    pub used_by: Vec<DotRef>,
    /// Why it is in `error` or `needs_sign_in`.
    pub error: Option<String>,
    /// How many tools its accepted list has.
    pub tools: Option<usize>,
    /// Its Personal catalog's status.
    pub catalog_status: Option<CatalogStatus>,
}

/// Whether this PC has the program a stdio connector needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCheck {
    pub needed: Option<RuntimeNeed>,
    /// The version found (`v24.1.0`), if any.
    pub found: Option<String>,
    /// Nothing is needed, or what was found is new enough.
    pub ok: bool,
}

/// The secret store prefix of a connector's sign-in.
fn grant_key(id: &str) -> String {
    format!("connector/{id}/oauth")
}

/// What is kept of a connector's sign-in.
pub(crate) enum StoredGrant {
    Missing,
    /// It cannot be read back (a write cut off, a part deleted outside Dots): only signing in
    /// again helps.
    Corrupt,
    Present(Box<OAuthGrant>),
}

/// The sign-in grant kept for the connector `id`.
pub(crate) fn load_grant(vault: &Vault, id: &str) -> Result<StoredGrant> {
    Ok(match vault.get_parts(&grant_key(id))? {
        Parts::Missing => StoredGrant::Missing,
        Parts::Corrupt => StoredGrant::Corrupt,
        Parts::Value(text) => match serde_json::from_str(&text) {
            Ok(grant) => StoredGrant::Present(Box::new(grant)),
            Err(_) => StoredGrant::Corrupt,
        },
    })
}

/// Forgets what Dots keeps for the connector `id`: its Personal variables `variables` and its
/// sign-in grant. What is already gone is not an error.
pub(crate) async fn forget(vault: &Vault, id: &str, variables: &[String]) -> Result<()> {
    for name in variables {
        match vault.remove(name).await {
            Ok(()) | Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    vault.delete_parts(&grant_key(id))
}

/// The Personal variables a connector's server item reads that Dots made for it
/// (`CONNECTOR_<NAME>_…`), for `forget` when the item leaves the Library.
pub(crate) fn owned_variables(item: &LibraryItem) -> Vec<String> {
    let LibraryConfig::McpServer(config) = &item.config else {
        return Vec::new();
    };
    let own = catalog::variable_for(&item.name, "");
    config
        .variables()
        .into_iter()
        .filter(|v| v.starts_with(&own))
        .collect()
}

fn save_grant(vault: &Vault, id: &str, grant: &OAuthGrant) -> Result<()> {
    let redact = crate::vault::redact::global();
    if let Some(r) = &grant.refresh_token {
        redact.add(&format!("{id} refresh token"), r);
    }
    if let Some(s) = &grant.client.client_secret {
        redact.add(&format!("{id} client secret"), s);
    }
    let text = Zeroizing::new(serde_json::to_string(grant)?);
    vault.put_parts(&grant_key(id), &text)
}

/// The connector whose server `item` is: an item named after a catalog connector whose config
/// is its template.
pub fn connector_of(item: &LibraryItem) -> Option<Connector> {
    let connector = find(&item.name)?;
    let LibraryConfig::McpServer(config) = &item.config else {
        return None;
    };
    (connector.server.as_ref()?.config() == *config).then_some(connector)
}

/// The value of `field` as it is stored, or why it cannot be.
pub fn check_field(field: &KeyField, raw: &str) -> Result<String> {
    let value = if field.strip_spaces {
        raw.chars().filter(|c| !c.is_whitespace()).collect()
    } else {
        raw.trim().to_string()
    };
    let bad = |why: &str| Err(Error::Invalid(format!("{}: {why}", field.label)));
    if value.is_empty() {
        return bad("enter a value");
    }
    if value.chars().any(char::is_control) {
        return bad("remove the control characters");
    }
    if field.secret && value.len() > MAX_SECRET_BYTES {
        return bad(&format!("it is longer than {MAX_SECRET_BYTES} bytes"));
    }
    match &field.format {
        FieldFormat::Any => {}
        FieldFormat::Prefix { prefixes } => {
            if !prefixes.iter().any(|p| value.starts_with(p.as_str())) {
                return bad(&format!("it should start with {}", prefixes.join(" or ")));
            }
        }
        FieldFormat::Email => {
            let ok = value
                .split_once('@')
                .is_some_and(|(a, d)| !a.is_empty() && d.contains('.') && !value.contains(' '));
            if !ok {
                return bad("enter an email address");
            }
        }
        FieldFormat::Letters { length } => {
            if value.chars().count() != *length || !value.chars().all(|c| c.is_ascii_alphabetic()) {
                return bad(&format!("it should be {length} letters"));
            }
        }
    }
    Ok(value)
}

/// The values of a key connector's fields, checked, in field order.
pub fn check_values(
    connector: &Connector,
    values: &BTreeMap<String, String>,
) -> Result<Vec<(KeyField, String)>> {
    connector
        .fields
        .iter()
        .map(|field| {
            let raw = values.get(&field.key).map(String::as_str).unwrap_or("");
            Ok((field.clone(), check_field(field, raw)?))
        })
        .collect()
}

/// The connector `id` and its server's config, for a connect or sign-in: `NotFound` for an id
/// the catalog lacks, `Invalid` for one that cannot be connected yet or is not of `auth`.
pub fn plan(id: &str, auth: AuthKind) -> Result<(Connector, McpServerConfig)> {
    let connector = find(id).ok_or_else(|| Error::NotFound(format!("no connector named {id}")))?;
    let Some(server) = &connector.server else {
        return Err(Error::Invalid(format!(
            "{} cannot be connected yet",
            connector.name
        )));
    };
    if connector.auth != auth {
        return Err(Error::Invalid(match connector.auth {
            AuthKind::OAuth => format!("{} is connected by signing in", connector.name),
            _ => format!("{} is connected with a key", connector.name),
        }));
    }
    let config = server.config();
    Ok((connector, config))
}

/// The first line of the confirmation before a connector is connected: who makes it.
pub fn maker_line(connector: &Connector) -> String {
    match connector.trust {
        Trust::Official => format!(
            "{} connector, made by {} (official).",
            connector.name, connector.maker
        ),
        Trust::Community => format!(
            "{} connector, made by {} (community), not by {}. Its program gets the values you \
             entered.",
            connector.name, connector.maker, connector.name
        ),
    }
}

/// The major version in `node --version`'s output (`v24.1.0`).
fn major(version: &str) -> Option<u32> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Whether this PC has the program `connector` needs (`connector_runtime`).
pub async fn runtime_check(connector: &Connector) -> RuntimeCheck {
    let Some(need) = connector.runtime.clone() else {
        return RuntimeCheck {
            needed: None,
            found: None,
            ok: true,
        };
    };
    let program = crate::mcp::resolve_program(
        &need.command,
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("PATHEXT").as_deref(),
    );
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::proc::hide_window(&mut cmd);
    let found = match tokio::time::timeout(Duration::from_secs(10), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        _ => None,
    };
    let ok = found
        .as_deref()
        .and_then(major)
        .is_some_and(|m| m >= need.min_major);
    RuntimeCheck {
        needed: Some(need),
        found,
        ok,
    }
}

/// What a run or check does before it opens a connector's server: go on, or leave it out with
/// the notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    NeedsSignIn(String),
}

/// One refresh at a time per connector: a server that rotates refresh tokens refuses the old
/// one.
fn refresh_lock(id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .entry(id.to_string())
        .or_default()
        .clone()
}

fn sign_in_again(name: &str) -> String {
    format!("{name}: sign in again on the Connectors screen")
}

/// Refreshes the access token of the sign-in connector whose server `item` is, when it expires
/// within `REFRESH_MARGIN`, or when `refused` names the token the server just answered 401 to.
/// A refused token that is no longer the stored one was refreshed meanwhile by another run or
/// check, so it is not refreshed again. Any other item is `Fresh`. A refresh the server refuses,
/// or a grant that cannot be read back, marks the connector "needs sign-in again"; a refresh
/// that fails for another reason keeps the old token and is logged.
pub async fn refresh_if_due(
    vault: &Vault,
    item: &LibraryItem,
    refused: Option<&str>,
) -> Result<Freshness> {
    if !signs_in(item) {
        return Ok(Freshness::Fresh);
    }
    let lock = refresh_lock(&item.name);
    let _turn = lock.lock().await;
    let mut grant = match load_grant(vault, &item.name)? {
        StoredGrant::Missing => return Ok(Freshness::Fresh),
        StoredGrant::Corrupt => return Ok(Freshness::NeedsSignIn(sign_in_again(&item.name))),
        StoredGrant::Present(grant) => *grant,
    };
    if grant.needs_sign_in {
        return Ok(Freshness::NeedsSignIn(sign_in_again(&item.name)));
    }
    match refused {
        None if !grant.expires_within(Utc::now(), REFRESH_MARGIN) => {
            return Ok(Freshness::Fresh);
        }
        Some(sent) => {
            let token = catalog::variable_for(&item.name, "ACCESS_TOKEN");
            let stored = vault.personal_value(&token).await?;
            if stored.as_deref().is_some_and(|now| now.as_str() != sent) {
                return Ok(Freshness::Fresh);
            }
        }
        None => {}
    }
    match oauth::refresh(&oauth::client(), &grant).await {
        Ok(signed) => {
            // The grant first: it holds the new refresh token, which a rotating server will not
            // give again. A failure to keep the access token after it only costs a refresh.
            save_grant(vault, &item.name, &signed.grant)?;
            if let Err(e) = store_token(vault, &item.name, &item.name, &signed.access_token).await {
                tracing::warn!("{}: keeping the refreshed token failed: {e}", item.name);
            }
            Ok(Freshness::Fresh)
        }
        Err(RefreshFailure::Rejected) => {
            grant.needs_sign_in = true;
            save_grant(vault, &item.name, &grant)?;
            Ok(Freshness::NeedsSignIn(sign_in_again(&item.name)))
        }
        Err(RefreshFailure::Failed(why)) => {
            tracing::warn!("{}: refreshing the sign-in failed: {why}", item.name);
            Ok(Freshness::Fresh)
        }
    }
}

/// The variable a sign-in connector's server `item` sends its access token from.
pub fn token_variable(item: &LibraryItem) -> String {
    catalog::variable_for(&item.name, "ACCESS_TOKEN")
}

/// Whether `item` is a sign-in connector's server: an http server that sends its connector's
/// access token, `Authorization: Bearer ${CONNECTOR_<NAME>_ACCESS_TOKEN}`.
pub fn signs_in(item: &LibraryItem) -> bool {
    let LibraryConfig::McpServer(McpServerConfig::Http { headers, .. }) = &item.config else {
        return false;
    };
    let wanted = format!(
        "Bearer ${{{}}}",
        catalog::variable_for(&item.name, "ACCESS_TOKEN")
    );
    headers.get("Authorization") == Some(&wanted)
}

/// Stores a sign-in's access token as the Personal secret of the connector `id` (`app` names
/// it in an error).
async fn store_token(vault: &Vault, id: &str, app: &str, token: &str) -> Result<()> {
    if token.len() > MAX_SECRET_BYTES {
        return Err(Error::Invalid(format!(
            "{app} gave a sign-in token longer than Windows Credential Manager keeps ({MAX_SECRET_BYTES} bytes)"
        )));
    }
    let name = catalog::variable_for(id, "ACCESS_TOKEN");
    put_value(vault, &name, EntryKind::Secret, token)
        .await
        .map(|_| ())
}

/// Sets the Personal variable `name` to `value` as `kind`: added when it is not there, its
/// value replaced when it is (a variable of the other kind is replaced by one of this kind).
/// `true` when it was added.
async fn put_value(vault: &Vault, name: &str, kind: EntryKind, value: &str) -> Result<bool> {
    match vault.store().find_env_entry(name).await? {
        Some(entry) if entry.kind == kind => {
            vault.set_value(&entry.name, value).await?;
            Ok(false)
        }
        Some(entry) => {
            vault.remove(&entry.name).await?;
            vault.add(name, kind, value).await?;
            Ok(true)
        }
        None => {
            vault.add(name, kind, value).await?;
            Ok(true)
        }
    }
}

/// The sign-ins in progress, by connector id, for Cancel.
fn pending() -> &'static Mutex<HashMap<String, CancellationToken>> {
    static PENDING: std::sync::OnceLock<Mutex<HashMap<String, CancellationToken>>> =
        std::sync::OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

struct PendingSignIn(String);
impl Drop for PendingSignIn {
    fn drop(&mut self) {
        pending().lock().unwrap().remove(&self.0);
    }
}

/// Cancels the sign-in to `id` in progress, if there is one.
pub fn cancel_sign_in(id: &str) {
    if let Some(token) = pending().lock().unwrap().get(id) {
        token.cancel();
    }
}

impl Runtime {
    /// Every connector with its state (`connector_list`).
    pub async fn connectors(&self) -> Result<Vec<ConnectorEntry>> {
        let entries = self.library().list().await?;
        let mut out = Vec::new();
        for connector in catalog() {
            let entry = entries
                .iter()
                .find(|e| e.item.kind() == LibraryKind::McpServer && e.item.name == connector.id);
            out.push(self.connector_entry(connector, entry)?);
        }
        Ok(out)
    }

    /// One connector's entry (`connector_list` gives them all).
    pub async fn connector(&self, id: &str) -> Result<ConnectorEntry> {
        self.connectors()
            .await?
            .into_iter()
            .find(|c| c.connector.id == id)
            .ok_or_else(|| Error::NotFound(format!("no connector named {id}")))
    }

    fn connector_entry(
        &self,
        connector: Connector,
        entry: Option<&LibraryEntry>,
    ) -> Result<ConnectorEntry> {
        let mut out = ConnectorEntry {
            state: ConnectorState::NotConnected,
            server_id: None,
            used_by: Vec::new(),
            error: None,
            tools: None,
            catalog_status: None,
            connector,
        };
        if out.connector.auth == AuthKind::ComingSoon {
            out.state = ConnectorState::ComingSoon;
            return Ok(out);
        }
        let Some(entry) = entry else {
            return Ok(out);
        };
        let ours = out
            .connector
            .server
            .as_ref()
            .is_some_and(|s| entry.item.config == LibraryConfig::McpServer(s.config()));
        if !ours {
            out.state = ConnectorState::NameTaken;
            out.error = Some(format!(
                "The Library already has an MCP server named {} of your own. Rename or remove it \
                 in the Library to connect {}.",
                entry.item.name, out.connector.name
            ));
            return Ok(out);
        }
        out.server_id = Some(entry.item.id.clone());
        out.used_by = entry.used_by.clone();
        out.catalog_status = entry.catalog.as_ref().map(|c| c.status);
        out.tools = entry
            .catalog
            .as_ref()
            .and_then(|c| c.accepted_tools.as_ref())
            .and_then(|t| t.as_array().map(Vec::len));
        out.state = ConnectorState::Connected;
        if out.connector.auth == AuthKind::OAuth {
            let signed_in = matches!(
                load_grant(self.vault(), &out.connector.id)?,
                StoredGrant::Present(g) if !g.needs_sign_in
            );
            if !signed_in || !entry.missing_variables.is_empty() {
                out.state = ConnectorState::NeedsSignIn;
                out.error = Some(format!(
                    "{} needs you to sign in again.",
                    out.connector.name
                ));
                return Ok(out);
            }
        } else if !entry.missing_variables.is_empty() {
            out.state = ConnectorState::Error;
            out.error = Some(format!(
                "Missing {}: enter the key again.",
                entry.missing_variables.join(", ")
            ));
            return Ok(out);
        }
        if let Some(c) = entry.catalog.as_ref() {
            if c.status == CatalogStatus::Unreachable {
                out.state = ConnectorState::Error;
                out.error = c.error.clone();
            }
        }
        Ok(out)
    }

    /// Connects the key connector `id` with `values` (field key to text) and gives it to the
    /// Personal dots `dot_ids` (`connector_connect`, connectors spec section 4). The caller has
    /// asked the person first (`confirm::connect_confirmed`). Connecting again replaces the
    /// values and the dots.
    pub async fn connector_connect(
        &self,
        id: &str,
        values: &BTreeMap<String, String>,
        dot_ids: &[String],
    ) -> Result<ConnectorEntry> {
        let (connector, _) = plan(id, AuthKind::Key)?;
        self.connect_with(&connector, values, dot_ids).await
    }

    /// `connector_connect` for `connector`, which need not be in the catalog (tests).
    pub async fn connect_with(
        &self,
        connector: &Connector,
        values: &BTreeMap<String, String>,
        dot_ids: &[String],
    ) -> Result<ConnectorEntry> {
        let checked = check_values(connector, values)?;
        self.check_dots(dot_ids).await?;
        let existing = self.connector_item_with(connector).await?;
        let mut added = Vec::new();
        for (field, value) in &checked {
            let kind = if field.secret {
                EntryKind::Secret
            } else {
                EntryKind::Plain
            };
            match put_value(self.vault(), &field.variable, kind, value).await {
                Ok(true) => added.push(field.variable.clone()),
                Ok(false) => {}
                Err(e) => {
                    self.forget(&added).await;
                    return Err(e);
                }
            }
        }
        self.finish_connect(connector, existing, &added, dot_ids)
            .await
    }

    /// `connector`'s entry as `connectors` gives it; `connector` need not be in the catalog
    /// (tests).
    pub async fn connector_entry_for(&self, connector: &Connector) -> Result<ConnectorEntry> {
        let entries = self.library().list().await?;
        let entry = entries
            .iter()
            .find(|e| e.item.kind() == LibraryKind::McpServer && e.item.name == connector.id);
        self.connector_entry(connector.clone(), entry)
    }

    /// The Library item of `connector`, matched against its own template (it may not be in the
    /// catalog).
    async fn connector_item_with(&self, connector: &Connector) -> Result<Option<LibraryItem>> {
        let Some(item) = self
            .store()
            .find_library_item(LibraryKind::McpServer, &connector.id)
            .await?
        else {
            return Ok(None);
        };
        let ours = connector
            .server
            .as_ref()
            .is_some_and(|s| item.config == LibraryConfig::McpServer(s.config()));
        if !ours {
            return Err(Error::Conflict(format!(
                "The Library already has an MCP server named {} of your own. Rename or remove it \
                 in the Library to connect {}.",
                item.name, connector.name
            )));
        }
        Ok(Some(item))
    }

    /// Removes the Personal variables `names` a failed connect added.
    async fn forget(&self, names: &[String]) {
        for name in names {
            if let Err(e) = self.vault().remove(name).await {
                tracing::warn!("could not remove {name} after a failed connect: {e}");
            }
        }
    }

    /// The rest of a connect, once the values are stored: the server item (added when new),
    /// its check, the first list accepted, the dots.
    async fn finish_connect(
        &self,
        connector: &Connector,
        existing: Option<LibraryItem>,
        added: &[String],
        dot_ids: &[String],
    ) -> Result<ConnectorEntry> {
        let server = connector
            .server
            .as_ref()
            .ok_or_else(|| Error::Invalid(format!("{} cannot be connected yet", connector.name)))?;
        let item = match existing {
            Some(item) => item,
            None => {
                let config = LibraryConfig::McpServer(server.config());
                match self.library().add(&connector.id, config, None).await {
                    Ok(item) => item,
                    Err(e) => {
                        self.forget(added).await;
                        return Err(e);
                    }
                }
            }
        };
        // The person chose this connector and was told what it does: its first list is
        // accepted (connectors spec §2.1.5). A later change waits for review.
        let catalog = self.library().check(&item.id, &DotScope::Personal).await?;
        if catalog.accepted_fingerprint.is_none() {
            if let Some(fp) = catalog.seen_fingerprint.clone() {
                self.library()
                    .accept(&item.id, &DotScope::Personal, &fp)
                    .await?;
            }
        }
        self.apply_dots(&connector.id, dot_ids).await?;
        let entries = self.library().list().await?;
        let entry = entries.iter().find(|e| e.item.id == item.id);
        self.connector_entry(connector.clone(), entry)
    }

    /// Refuses a dot that is not a Personal one: a connector's values are Personal.
    async fn check_dots(&self, dot_ids: &[String]) -> Result<()> {
        for id in dot_ids {
            let dot = self.store().get_dot(id).await?;
            if DotScope::of(&dot) != DotScope::Personal {
                return Err(Error::Invalid(format!(
                    "{} is an organisation's dot; connectors are for Personal dots",
                    dot.spec.name
                )));
            }
        }
        Ok(())
    }

    /// Makes the Personal dots `dot_ids` use the server `name`, and the other Personal dots not
    /// use it, through `update_dot` (which grants or drops its variables and checks the role).
    async fn apply_dots(&self, name: &str, dot_ids: &[String]) -> Result<()> {
        for dot in self.store().list_dots().await? {
            if DotScope::of(&dot) != DotScope::Personal {
                continue;
            }
            let want = dot_ids.contains(&dot.id);
            let has = dot.spec.uses.mcp_servers.iter().any(|n| n == name);
            if want == has {
                continue;
            }
            let mut spec = dot.spec.clone();
            if want {
                spec.uses.mcp_servers.push(name.to_string());
            } else {
                spec.uses.mcp_servers.retain(|n| n != name);
            }
            self.update_dot(&dot.id, spec).await?;
        }
        Ok(())
    }

    /// Gives the connected connector `id` to exactly the Personal dots `dot_ids`
    /// (`connector_set_dots`).
    pub async fn connector_set_dots(&self, id: &str, dot_ids: &[String]) -> Result<ConnectorEntry> {
        let connector =
            find(id).ok_or_else(|| Error::NotFound(format!("no connector named {id}")))?;
        if self.connector_item_with(&connector).await?.is_none() {
            return Err(Error::NotFound(format!(
                "{} is not connected",
                connector.name
            )));
        }
        self.check_dots(dot_ids).await?;
        self.apply_dots(id, dot_ids).await?;
        self.connector(id).await
    }

    /// Disconnects `id` (`connector_disconnect`): its dots stop using it (their grants go), its
    /// Library item is removed, and its values and sign-in are forgotten.
    pub async fn connector_disconnect(&self, id: &str) -> Result<ConnectorEntry> {
        let connector =
            find(id).ok_or_else(|| Error::NotFound(format!("no connector named {id}")))?;
        self.disconnect_with(&connector).await?;
        self.connector(id).await
    }

    /// `connector_disconnect` for `connector`, which need not be in the catalog (tests). It can
    /// be run again: a server already gone is no error, and the values and sign-in are always
    /// forgotten, so what a removal cut off part way left behind goes too. A server of the
    /// person's own with the connector's name is left alone.
    pub async fn disconnect_with(&self, connector: &Connector) -> Result<()> {
        match self.connector_item_with(connector).await {
            Ok(Some(item)) => {
                self.apply_dots(&connector.id, &[]).await?;
                match self.library().remove(&item.id, true).await {
                    Ok(()) | Err(Error::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(None) | Err(Error::Conflict(_)) => {}
            Err(e) => return Err(e),
        }
        forget(self.vault(), &connector.id, &connector.variables()).await
    }

    /// Signs in to the sign-in connector `id` and gives it to the Personal dots `dot_ids`
    /// (`connector_sign_in`, connectors spec section 5). The caller has asked the person first.
    /// `open_browser` opens the sign-in page. Signing in again replaces the tokens.
    pub async fn connector_sign_in<F>(
        &self,
        id: &str,
        dot_ids: &[String],
        open_browser: F,
    ) -> Result<ConnectorEntry>
    where
        F: FnOnce(&str) -> Result<()> + Send,
    {
        let (connector, _) = plan(id, AuthKind::OAuth)?;
        self.sign_in_with(&connector, dot_ids, open_browser, oauth::SIGN_IN_LIMIT)
            .await
    }

    /// `connector_sign_in` for `connector`, which need not be in the catalog (tests).
    pub async fn sign_in_with<F>(
        &self,
        connector: &Connector,
        dot_ids: &[String],
        open_browser: F,
        limit: Duration,
    ) -> Result<ConnectorEntry>
    where
        F: FnOnce(&str) -> Result<()> + Send,
    {
        let Some(ServerTemplate::Http { url, .. }) = &connector.server else {
            return Err(Error::Invalid(format!(
                "{} has no hosted server to sign in to",
                connector.name
            )));
        };
        self.check_dots(dot_ids).await?;
        let existing = self.connector_item_with(connector).await?;
        let cancel = CancellationToken::new();
        {
            let mut pending = pending().lock().unwrap();
            if pending.contains_key(&connector.id) {
                return Err(Error::Conflict(format!(
                    "A sign-in to {} is already in progress.",
                    connector.name
                )));
            }
            pending.insert(connector.id.clone(), cancel.clone());
        }
        let _pending = PendingSignIn(connector.id.clone());
        let signed = oauth::sign_in(url, open_browser, limit, cancel).await?;
        let variable = connector.token_variable();
        let added = self
            .vault()
            .store()
            .find_env_entry(&variable)
            .await?
            .is_none();
        store_token(
            self.vault(),
            &connector.id,
            &connector.name,
            &signed.access_token,
        )
        .await?;
        save_grant(self.vault(), &connector.id, &signed.grant)?;
        let added = if added { vec![variable] } else { Vec::new() };
        self.finish_connect(connector, existing, &added, dot_ids)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_connector_is_complete_and_its_template_is_a_valid_server() {
        let all = catalog();
        let mut ids = std::collections::HashSet::new();
        for c in &all {
            assert!(ids.insert(c.id.clone()), "{} twice", c.id);
            crate::library::check_name(LibraryKind::McpServer, &c.id).unwrap();
            assert!(!c.instructions.steps.is_empty(), "{} has no steps", c.id);
            assert!(c.instructions.open_url.starts_with("https://"), "{}", c.id);
            assert!(c.source_url.starts_with("https://"), "{}", c.id);
            match c.auth {
                AuthKind::ComingSoon => assert!(c.server.is_none()),
                AuthKind::OAuth => {
                    let Some(ServerTemplate::Http { url, bearer }) = &c.server else {
                        panic!("{} signs in to a hosted server", c.id)
                    };
                    assert!(url.starts_with("https://"));
                    assert_eq!(bearer.as_deref(), Some(c.token_variable().as_str()));
                    assert!(c.fields.is_empty());
                }
                AuthKind::Key => {
                    assert!(!c.fields.is_empty(), "{} has no fields", c.id);
                    let config = LibraryConfig::McpServer(c.server.as_ref().unwrap().config());
                    config.validate().unwrap();
                    // Every variable the server reads is one of its fields, and the other way.
                    let LibraryConfig::McpServer(server) = &config else {
                        unreachable!()
                    };
                    let mut read = server.variables();
                    read.sort();
                    let mut fields = c.variables();
                    fields.sort();
                    assert_eq!(read, fields, "{}", c.id);
                    for f in &c.fields {
                        crate::vault::entries::check_variable_name(&f.variable).unwrap();
                        assert!(f.variable.starts_with("CONNECTOR_"));
                    }
                }
            }
            // A community server is a stdio program pinned to a version.
            if c.trust == Trust::Community {
                let Some(ServerTemplate::Stdio { args, .. }) = &c.server else {
                    panic!("{} is a community program", c.id)
                };
                let package = &args[1];
                assert!(package[1..].contains('@'), "{} is pinned", c.id);
                assert!(c.note.is_some(), "{} says who makes it", c.id);
            }
        }
    }

    #[test]
    fn fields_are_checked_and_cleaned_before_they_are_stored() {
        let github = find("github").unwrap();
        let token = &github.fields[0];
        assert_eq!(check_field(token, "  ghp_abc ").unwrap(), "ghp_abc");
        assert_eq!(check_field(token, "github_pat_x").unwrap(), "github_pat_x");
        let e = check_field(token, "abc").unwrap_err().to_string();
        assert!(e.contains("github_pat_ or ghp_"), "{e}");
        assert!(check_field(token, "   ").is_err());

        let gmail = find("gmail").unwrap();
        let address = &gmail.fields[0];
        let password = &gmail.fields[1];
        assert!(check_field(address, "me@gmail.com").is_ok());
        assert!(check_field(address, "me").is_err());
        assert_eq!(
            check_field(password, "abcd efgh ijkl mnop").unwrap(),
            "abcdefghijklmnop"
        );
        assert!(check_field(password, "abcd efgh").is_err());
        assert!(check_field(password, "abcd efgh ijkl mno1").is_err());
    }

    #[test]
    fn a_connector_is_matched_by_its_name_and_template() {
        let github = find("github").unwrap();
        let config = LibraryConfig::McpServer(github.server.as_ref().unwrap().config());
        let item = |name: &str, config: LibraryConfig| LibraryItem {
            id: "1".into(),
            name: name.into(),
            config,
            owner_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(connector_of(&item("github", config.clone())).is_some());
        assert!(connector_of(&item("gh", config)).is_none());
        let other = LibraryConfig::McpServer(McpServerConfig::Http {
            url: "https://example.com/mcp".into(),
            headers: BTreeMap::new(),
        });
        assert!(connector_of(&item("github", other)).is_none());
    }

    #[test]
    fn the_node_major_version_is_read() {
        assert_eq!(major("v24.18.0"), Some(24));
        assert_eq!(major("18.2.1\n"), Some(18));
        assert_eq!(major("node"), None);
    }

    #[test]
    fn the_maker_line_names_a_community_maker() {
        let slack = find("slack").unwrap();
        let line = maker_line(&slack);
        assert!(
            line.contains("korotovsky (community), not by Slack"),
            "{line}"
        );
        assert!(maker_line(&find("github").unwrap()).contains("GitHub (official)"));
    }
}

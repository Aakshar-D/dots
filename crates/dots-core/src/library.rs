//! The Library: MCP servers, skills and plugins that dots use (MCP/skills spec sections 2 and 3).
//! Each item has a kind, a name unique within its kind, and a config. A dot ticks items by
//! name in its `uses`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

str_enum!(LibraryKind {
    McpServer => "mcp_server",
    Skill => "skill",
    Plugin => "plugin",
    Context => "context",
    App => "app",
});
// Where a server's tool list stands (spec section 3): the person accepted the list last seen;
// the list changed since; the last check failed; or no list was accepted yet.
str_enum!(CatalogStatus {
    Accepted => "accepted",
    Changed => "changed",
    Unreachable => "unreachable",
    Unchecked => "unchecked",
});

impl LibraryKind {
    /// The kind in a message: "MCP server", "skill", "plugin".
    pub fn label(&self) -> &'static str {
        match self {
            Self::McpServer => "MCP server",
            Self::Skill => "skill",
            Self::Plugin => "plugin",
            Self::Context => "context collection",
            Self::App => "app",
        }
    }

    /// The list of `Uses` that names items of this kind.
    pub fn uses_key(&self) -> &'static str {
        match self {
            Self::McpServer => "mcp_servers",
            Self::Skill => "skills",
            Self::Plugin => "plugins",
            Self::Context => "context",
            Self::App => "apps",
        }
    }
}

/// The longest MCP server name: the tool prefix `mcp__<name>__` must leave room for the tool.
pub const MAX_SERVER_NAME: usize = 40;
/// The longest skill or plugin name.
pub const MAX_ITEM_NAME: usize = 64;
/// The longest context collection name: the model passes it to the `context` tool.
pub const MAX_COLLECTION_NAME: usize = 40;
/// The longest description of a context collection, in characters.
pub const MAX_COLLECTION_DESCRIPTION: usize = 300;

/// How Dots starts or reaches an MCP server. `${VAR}` in `env` values, `args`, `cwd`, `url` and
/// `headers` reads Dots' environment when the server starts (`expanded`, in `mcp::vars`); the
/// stored config keeps the reference, never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// A folder that holds a `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillConfig {
    pub path: String,
}

/// A Claude Code plugin folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginConfig {
    pub path: String,
    /// `<plugin>@<marketplace>` when picked from the installed Claude Code plugins: its folder
    /// is looked up again when it moves on an update.
    #[serde(default)]
    pub installed_id: Option<String>,
    /// The plugin's hooks run on Claude-engine dots. Off until the person allows it.
    #[serde(default)]
    pub run_hooks: bool,
}

/// A folder of documents that dots read with the `context` tool (context library spec
/// section 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextConfig {
    pub path: String,
    #[serde(default)]
    pub description: String,
    /// Every dot uses the collection, ticked or not.
    #[serde(default)]
    pub every_dot: bool,
}

/// An app in the Library (app library spec sections 4 and 6): Dots keeps its code and data in
/// the data folder (`apps::Apps`); the row says which version is active and how dots may use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The active version: a counter Dots raises on each accepted version, from 1.
    pub version: u32,
    /// The version before it, kept for "Roll back" while the update that replaced it ran no
    /// migrations.
    #[serde(default)]
    pub previous_version: Option<u32>,
    /// The highest version Dots has given the app: a roll back keeps it, so a number is never
    /// given twice and a change checked against a version cannot match another (0: `version`).
    #[serde(default)]
    pub last_version: u32,
    /// Every dot uses the app, ticked or not.
    #[serde(default)]
    pub every_dot: bool,
    /// "Can start dots": off until the person switches it on (spec section 7).
    #[serde(default)]
    pub can_start_dots: bool,
    pub origin: AppOrigin,
    /// A bundled app the person changed through a dot's proposal: Dots no longer updates it.
    #[serde(default)]
    pub changed: bool,
}

/// Where an app came from (spec section 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppOrigin {
    /// Shipped with Dots: the release it came from.
    Bundled { release: String },
    /// Proposed by a dot and accepted by the person.
    Proposed,
}

/// An item's config, with its kind: `{"kind": "mcp_server", "config": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum LibraryConfig {
    McpServer(McpServerConfig),
    Skill(SkillConfig),
    Plugin(PluginConfig),
    Context(ContextConfig),
    App(AppConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub config: LibraryConfig,
    /// The plugin an MCP server comes from; it goes with the plugin.
    pub owner_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl LibraryItem {
    pub fn kind(&self) -> LibraryKind {
        self.config.kind()
    }
}

/// The Library items a dot uses, by name. A plugin's MCP servers come with the plugin and are
/// not named here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Uses {
    pub mcp_servers: Vec<String>,
    pub skills: Vec<String>,
    pub plugins: Vec<String>,
    /// Context collections; one whose `every_dot` is on is used without being named.
    pub context: Vec<String>,
    /// Apps (app library spec section 6); one whose `every_dot` is on is used without being
    /// named.
    pub apps: Vec<String>,
}

impl Uses {
    pub fn is_empty(&self) -> bool {
        self.mcp_servers.is_empty()
            && self.skills.is_empty()
            && self.plugins.is_empty()
            && self.context.is_empty()
            && self.apps.is_empty()
    }

    /// The names of the items of `kind`.
    pub fn names(&self, kind: LibraryKind) -> &[String] {
        match kind {
            LibraryKind::McpServer => &self.mcp_servers,
            LibraryKind::Skill => &self.skills,
            LibraryKind::Plugin => &self.plugins,
            LibraryKind::Context => &self.context,
            LibraryKind::App => &self.apps,
        }
    }

    /// Each name is a valid one for its kind, and named once. Whether the Library has it is
    /// the store's check.
    pub fn validate(&self) -> Result<()> {
        for kind in LibraryKind::ALL {
            let mut seen = BTreeSet::new();
            for name in self.names(*kind) {
                check_name(*kind, name)?;
                if !seen.insert(name) {
                    return Err(Error::Invalid(format!(
                        "uses names the {} {name:?} twice",
                        kind.label()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// The scope of a catalog checked with Personal's values. Any other scope is an org's id (env
/// vault spec §8): a server's tools can differ by credential, so each scope has its own.
pub const PERSONAL_SCOPE: &str = "personal";

/// What Dots knows of an MCP server's tools in one scope (spec section 3). The tool lists are
/// `tools/list` results as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpCatalog {
    pub server_id: String,
    pub status: CatalogStatus,
    pub accepted_fingerprint: Option<String>,
    pub accepted_tools: Option<Value>,
    pub seen_fingerprint: Option<String>,
    pub seen_tools: Option<Value>,
    pub checked_at: Option<String>,
    pub error: Option<String>,
}

impl McpCatalog {
    /// The catalog of a server that was never checked: nothing accepted, nothing seen.
    pub fn unchecked(server_id: &str) -> McpCatalog {
        McpCatalog {
            server_id: server_id.to_string(),
            status: CatalogStatus::Unchecked,
            accepted_fingerprint: None,
            accepted_tools: None,
            seen_fingerprint: None,
            seen_tools: None,
            checked_at: None,
            error: None,
        }
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

/// An item name: an MCP server's is its tool prefix (`^[a-z0-9_-]{1,40}$`, no `__`, not starting or
/// ending with `_`, not `dots`); a skill's or plugin's becomes a folder name (1-64 of `A-Za-z0-9._-`, starting with
/// a letter or digit).
pub fn check_name(kind: LibraryKind, name: &str) -> Result<()> {
    match kind {
        LibraryKind::McpServer => {
            let chars_ok = name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if name.is_empty() || name.len() > MAX_SERVER_NAME || !chars_ok {
                return invalid(format!(
                    "MCP server name {name:?}: use 1 to {MAX_SERVER_NAME} characters of a-z, \
                     0-9, '-' or '_'"
                ));
            }
            // `mcp__<server>__<tool>` is split at the first `__` after `mcp__`.
            if name.contains("__") {
                return invalid(format!("MCP server name {name:?}: '__' is not allowed"));
            }
            // `mcp__files_` + `__x` would read as server `files`, tool `_x`.
            if name.starts_with('_') || name.ends_with('_') {
                return invalid(format!(
                    "MCP server name {name:?}: it must not start or end with '_'"
                ));
            }
            if name == "dots" {
                return invalid("the MCP server name 'dots' is reserved");
            }
        }
        LibraryKind::Skill | LibraryKind::Plugin => {
            let mut chars = name.chars();
            let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
            let rest_ok =
                chars.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
            if !first_ok || !rest_ok || name.len() > MAX_ITEM_NAME {
                return invalid(format!(
                    "{} name {name:?}: use 1 to {MAX_ITEM_NAME} characters of A-Z, a-z, 0-9, \
                     '.', '-' or '_', starting with a letter or digit",
                    kind.label()
                ));
            }
        }
        LibraryKind::Context => {
            let mut chars = name.chars();
            let first_ok = chars
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
            let rest_ok =
                chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if !first_ok || !rest_ok || name.len() > MAX_COLLECTION_NAME {
                return invalid(format!(
                    "context collection name {name:?}: use 1 to {MAX_COLLECTION_NAME} characters \
                     of a-z, 0-9, '-' or '_', starting with a letter or digit"
                ));
            }
        }
        LibraryKind::App => crate::apps::manifest::check_app_name(name)?,
    }
    Ok(())
}

impl LibraryConfig {
    pub fn kind(&self) -> LibraryKind {
        match self {
            Self::McpServer(_) => LibraryKind::McpServer,
            Self::Skill(_) => LibraryKind::Skill,
            Self::Plugin(_) => LibraryKind::Plugin,
            Self::Context(_) => LibraryKind::Context,
            Self::App(_) => LibraryKind::App,
        }
    }

    /// The config is complete: a command or an http(s) URL, a folder. Text Dots shows in a
    /// confirmation (a stdio command line, folder and env, an http URL and header names) holds no
    /// control or bidirectional format character, so what the person reads is what runs.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::McpServer(McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
            }) => {
                if command.trim().is_empty() {
                    return invalid("an MCP server needs the command that starts it");
                }
                let parts = std::iter::once(command)
                    .chain(args)
                    .chain(cwd.iter())
                    .chain(env.keys());
                for part in parts {
                    check_plain(part)?;
                }
                Ok(())
            }
            Self::McpServer(McpServerConfig::Http { url, headers }) => {
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return invalid(format!(
                        "MCP server URL must start with http:// or https://: {url}"
                    ));
                }
                // The URL parser takes `\` for `/` and skips every `/` after `scheme:`, so the host
                // it reaches is not the one that reads first. No http(s) URL needs either.
                let after = url.split_once("://").map_or("", |(_, rest)| rest);
                if url.contains('\\') || after.starts_with('/') {
                    return invalid(
                        "MCP server URL: no backslash, and no '/' right after 'http://' or \
                         'https://'",
                    );
                }
                for part in std::iter::once(url).chain(headers.keys()) {
                    check_plain(part)?;
                }
                Ok(())
            }
            Self::Skill(c) if c.path.trim().is_empty() => invalid("a skill needs its folder"),
            Self::Plugin(c) if c.path.trim().is_empty() => invalid("a plugin needs its folder"),
            Self::Context(c) if c.path.trim().is_empty() => {
                invalid("a context collection needs its folder")
            }
            Self::Context(c) if c.description.chars().count() > MAX_COLLECTION_DESCRIPTION => {
                invalid(format!(
                    "a context collection's description is at most {MAX_COLLECTION_DESCRIPTION} \
                     characters"
                ))
            }
            _ => Ok(()),
        }
    }
}

/// Refuses a control character (C0, DEL, C1) and a Unicode bidirectional format character
/// (U+061C, U+200E, U+200F, U+202A-U+202E, U+2066-U+2069): they hide or reorder text in a dialog.
fn check_plain(text: &str) -> Result<()> {
    let bad = text.chars().find(|c| {
        c.is_control()
            || matches!(*c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
    });
    match bad {
        Some(c) => invalid(format!(
            "an MCP server's command, arguments, folder, environment names, URL and header names \
             hold no control or direction character (found U+{:04X})",
            c as u32
        )),
        None => Ok(()),
    }
}

/// Where an http MCP server's requests go: its origin as the HTTP client sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOrigin {
    /// `scheme://host[:port]` as `reqwest::Url` (the WHATWG parser) serializes it: lower case,
    /// the host in ASCII, the default port left out.
    pub ascii: String,
    /// The host in Unicode, when that differs from the ASCII one.
    pub unicode: Option<String>,
}

/// The origin the HTTP client will use for `url` once `${VAR}` is read from `env`, which is the
/// text it is started with: the URL is expanded first and then parsed, so a variable that holds
/// a `/` or `\` moves the host as it does at start. `Err` says why there is none: a variable
/// that is not set ("X is not set"), or text that is not an http(s) URL. A change of origin
/// sends the requests elsewhere.
pub fn http_origin(
    url: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> std::result::Result<HttpOrigin, String> {
    let probe = McpServerConfig::Http {
        url: url.to_string(),
        headers: BTreeMap::new(),
    };
    let McpServerConfig::Http { url: expanded, .. } = probe.expanded(env).map_err(|e| match e {
        Error::Invalid(m) => m,
        e => e.to_string(),
    })?
    else {
        unreachable!("expanding keeps the transport")
    };
    let parsed = reqwest::Url::parse(&expanded).map_err(|_| "not a valid URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("not an http(s) URL".into());
    }
    let ascii = parsed.origin().ascii_serialization();
    let host = parsed.host_str().unwrap_or_default();
    let unicode = punycode_host(host).filter(|u| u != host);
    Ok(HttpOrigin { ascii, unicode })
}

impl McpServerConfig {
    /// Whether the server's origin is pinned (env vault spec §8): an http server whose `url`
    /// reads a `${VAR}`, so a value can move the host its requests go to. A run starts it only
    /// when the origin it reaches with the run's values was confirmed for the dot's scope. A
    /// fixed URL has one origin, which the add and change dialog already names with the
    /// variables its headers send, so it starts as before.
    pub fn origin_pinned(&self) -> bool {
        matches!(self, Self::Http { url, .. } if !url_variables(url).is_empty())
    }
}

/// The variables an http URL reads, sorted, each once: the ones its origin depends on.
pub(crate) fn url_variables(url: &str) -> Vec<String> {
    McpServerConfig::Http {
        url: url.to_string(),
        headers: BTreeMap::new(),
    }
    .variables()
}

/// Whether the origin differs between two URLs under `env`. An origin that cannot be computed
/// for one of them counts as a move unless the two texts are equal.
pub fn origin_changed(old: &str, new: &str, env: &dyn Fn(&str) -> Option<String>) -> bool {
    match (http_origin(old, env), http_origin(new, env)) {
        (Ok(a), Ok(b)) => a.ascii != b.ascii,
        _ => old != new,
    }
}

/// A host with its `xn--` labels decoded (RFC 3492), or `None` when it has none or one is bad.
fn punycode_host(host: &str) -> Option<String> {
    if !host.split('.').any(|l| l.starts_with("xn--")) {
        return None;
    }
    host.split('.')
        .map(|label| match label.strip_prefix("xn--") {
            Some(rest) => punycode_decode(rest),
            None => Some(label.to_string()),
        })
        .collect::<Option<Vec<_>>>()
        .map(|labels| labels.join("."))
}

fn punycode_decode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    let (basic, extended) = match input.rfind('-') {
        Some(i) => (&input[..i], &input[i + 1..]),
        None => ("", input),
    };
    let mut out: Vec<char> = basic.chars().collect();
    let (mut n, mut i, mut bias) = (128u32, 0u32, 72u32);
    let mut digits = extended.chars().peekable();
    while digits.peek().is_some() {
        let old_i = i;
        let (mut w, mut k) = (1u32, BASE);
        loop {
            let d = match digits.next()? {
                c @ 'a'..='z' => c as u32 - 'a' as u32,
                c @ '0'..='9' => c as u32 - '0' as u32 + 26,
                _ => return None,
            };
            i = i.checked_add(d.checked_mul(w)?)?;
            let t = if k <= bias {
                TMIN
            } else if k >= bias + TMAX {
                TMAX
            } else {
                k - bias
            };
            if d < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k += BASE;
        }
        let len = out.len() as u32 + 1;
        let mut delta = (i - old_i) / if old_i == 0 { 700 } else { 2 };
        delta += delta / len;
        let mut k2 = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k2 += BASE;
        }
        bias = k2 + (BASE - TMIN + 1) * delta / (delta + 38);
        n = n.checked_add(i / len)?;
        i %= len;
        out.insert(i as usize, char::from_u32(n)?);
        i += 1;
    }
    Some(out.into_iter().collect())
}

/// The words that make a key name a secret, as whole words of the key.
const SECRET_WORDS: &[&str] = &[
    "TOKEN",
    "KEY",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PWD",
    "AUTH",
    "AUTHORIZATION",
    "CREDENTIAL",
    "CREDENTIALS",
    "APIKEY",
];

/// The last words of a key that names where or what a secret is, not the secret itself.
const NOT_SECRET_LAST_WORDS: &[&str] = &[
    "URL", "URI", "HOST", "ENDPOINT", "ID", "NAME", "TYPE", "PATH", "FILE", "DIR", "HEADER",
    "REGION", "PORT", "ISSUER", "REALM", "SCOPE", "MODE",
];

/// The upper-case words of a key: split on `_`, `-` and `.`, and at a lower-case to upper-case
/// boundary (`accessToken` is `ACCESS`, `TOKEN`). A run of capitals stays one word, so `APIKey`
/// is `APIKEY` and `Authorization` is one word.
pub(crate) fn key_words(key: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in key.chars() {
        let split = matches!(c, '_' | '-' | '.') || (prev_lower && c.is_ascii_uppercase());
        if split && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if !matches!(c, '_' | '-' | '.') {
            word.push(c.to_ascii_uppercase());
        }
        prev_lower = c.is_ascii_lowercase();
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Whether a key names a secret: one of its words (see `key_words`: split on `_`, `-` and `.`
/// and at a lower-case to upper-case boundary, so `accessToken` counts) is `TOKEN`, `KEY`,
/// `SECRET`, `PASSWORD`, `PASSWD`, `PWD`, `AUTH`, `AUTHORIZATION`, `CREDENTIAL(S)` or `APIKEY`,
/// in any case (so also the header `Authorization`), and its last word is not one such as
/// `URL`, `ID` or `HEADER` (`TOKEN_URL`, `KEY_ID`). Such a value is better given as `${VAR}`;
/// the vault moves it there.
pub fn looks_secret(key: &str) -> bool {
    let words = key_words(key);
    match words.last() {
        Some(last) if !NOT_SECRET_LAST_WORDS.contains(&last.as_str()) => {
            words.iter().any(|w| SECRET_WORDS.contains(&w.as_str()))
        }
        _ => false,
    }
}

/// A valid MCP server name made from `raw`: lower case; any other character than a-z, 0-9 and
/// `-` becomes `_`, runs of `_` become one, and `_` is trimmed from the ends; `dots` becomes
/// `dots-server`, nothing becomes `server`; past 40 characters it is cut to 31 plus `-` and the
/// first 8 hex characters of the SHA-256 of `raw`.
pub fn mcp_server_name(raw: &str) -> String {
    let mut name = String::new();
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            c
        } else {
            '_'
        };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    let mut name = match name.trim_matches('_') {
        "" => "server".to_string(),
        "dots" => "dots-server".to_string(),
        trimmed => trimmed.to_string(),
    };
    if name.len() > MAX_SERVER_NAME {
        let hash: String = Sha256::digest(raw.as_bytes())
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect();
        // ASCII only, so any byte index is a character boundary.
        name = format!("{}-{hash}", &name[..MAX_SERVER_NAME - 9]);
    }
    name
}

/// The name of the MCP server `server` of the plugin `plugin`: `plugin-<plugin>-<server>`, made
/// valid by `mcp_server_name`. Not `plugin_<plugin>_<server>`, Claude Code's own prefix for a
/// plugin's server: a CLI that loads the same plugin from the person's settings and starts its
/// server itself would find that server's tools under the run's `--allowedTools
/// mcp__<name>__*` and the gateway's pass in `approve`, and they would skip Dots' check.
pub fn plugin_server_name(plugin: &str, server: &str) -> String {
    mcp_server_name(&format!("plugin-{plugin}-{server}"))
}

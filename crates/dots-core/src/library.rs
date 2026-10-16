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
        }
    }

    /// The list of `Uses` that names items of this kind.
    pub fn uses_key(&self) -> &'static str {
        match self {
            Self::McpServer => "mcp_servers",
            Self::Skill => "skills",
            Self::Plugin => "plugins",
        }
    }
}

/// The longest MCP server name: the tool prefix `mcp__<name>__` must leave room for the tool.
pub const MAX_SERVER_NAME: usize = 40;
/// The longest skill or plugin name.
pub const MAX_ITEM_NAME: usize = 64;

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

/// An item's config, with its kind: `{"kind": "mcp_server", "config": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum LibraryConfig {
    McpServer(McpServerConfig),
    Skill(SkillConfig),
    Plugin(PluginConfig),
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
}

impl Uses {
    pub fn is_empty(&self) -> bool {
        self.mcp_servers.is_empty() && self.skills.is_empty() && self.plugins.is_empty()
    }

    /// The names of the items of `kind`.
    pub fn names(&self, kind: LibraryKind) -> &[String] {
        match kind {
            LibraryKind::McpServer => &self.mcp_servers,
            LibraryKind::Skill => &self.skills,
            LibraryKind::Plugin => &self.plugins,
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

/// What Dots knows of an MCP server's tools (spec section 3). The tool lists are `tools/list`
/// results as JSON.
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
    }
    Ok(())
}

impl LibraryConfig {
    pub fn kind(&self) -> LibraryKind {
        match self {
            Self::McpServer(_) => LibraryKind::McpServer,
            Self::Skill(_) => LibraryKind::Skill,
            Self::Plugin(_) => LibraryKind::Plugin,
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
                    .chain(env.keys())
                    .chain(env.values());
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
                for part in std::iter::once(url).chain(headers.keys()) {
                    check_plain(part)?;
                }
                Ok(())
            }
            Self::Skill(c) if c.path.trim().is_empty() => invalid("a skill needs its folder"),
            Self::Plugin(c) if c.path.trim().is_empty() => invalid("a plugin needs its folder"),
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
            "an MCP server's command, arguments, folder, environment, URL and header names \
             hold no control or direction character (found U+{:04X})",
            c as u32
        )),
        None => Ok(()),
    }
}

/// The origin (`scheme://host[:port]`, lower case, without credentials) of an http(s) URL, or
/// `None` when the text is not one. A change of origin sends the server's requests elsewhere.
pub fn http_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    (!host.is_empty()).then(|| format!("{scheme}://{}", host.to_ascii_lowercase()))
}

/// Whether a key names a secret: `TOKEN`, `KEY`, `SECRET`, `PASSWORD` or `AUTH` in it, in any
/// case (so also the header `Authorization`). Such a value is better given as `${VAR}`.
pub fn looks_secret(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    ["TOKEN", "KEY", "SECRET", "PASSWORD", "AUTH"]
        .iter()
        .any(|word| key.contains(word))
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

/// The name of the MCP server `server` of the plugin `plugin`: `plugin_<plugin>_<server>`, the
/// tool prefix Claude Code gives it, made valid by `mcp_server_name`.
pub fn plugin_server_name(plugin: &str, server: &str) -> String {
    mcp_server_name(&format!("plugin_{plugin}_{server}"))
}

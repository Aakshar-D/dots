//! Secret values typed into an MCP server's config (env vault spec §11): a secret-looking env
//! value or header with no `${…}` moves into Personal and the config reads `${NAME}` instead,
//! so the value is never stored in the Library. An env value goes under its key
//! (`SF_TOKEN`), an `Authorization` header under `<SERVER>_AUTHORIZATION` with its scheme
//! kept, another header under `<SERVER>_<HEADER>`.

use super::entries::{check_value, check_variable_name, EntryKind};
use super::secrets::MAX_SECRET_BYTES;
use super::service::Vault;
use crate::library::{key_words, looks_secret, LibraryConfig, McpServerConfig};
use crate::mcp::vars::is_var_name;
use crate::store::{auth_scheme, var_name};
use crate::vault::redact;
use crate::Result;

/// The longest variable name Personal holds.
const MAX_NAME: usize = 128;

/// A valid variable name for `raw` (already `A-Z`, `0-9`, `_`): a digit at the start gets a
/// leading `_`, and the name is cut to 128 characters. The same input gives the same name.
fn valid_name(raw: &str) -> String {
    let mut name = String::new();
    if raw.is_empty() || raw.starts_with(|c: char| c.is_ascii_digit()) {
        name.push('_');
    }
    name.push_str(raw);
    name.truncate(MAX_NAME);
    name
}

/// `base` with `_n` after it, the base cut so that the whole stays within 128 characters.
fn suffixed(base: &str, n: usize) -> String {
    let suffix = format!("_{n}");
    let keep = MAX_NAME.saturating_sub(suffix.len()).min(base.len());
    format!("{}{suffix}", &base[..keep])
}

/// Where a value sits in a config.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Env(String),
    Header(String),
}

/// A secret-looking value of a config: where it is, the name it moves under, the value, and
/// the text around `${NAME}` (an `Authorization` header keeps its scheme).
#[derive(Debug, Clone)]
struct Found {
    place: Place,
    name: String,
    value: String,
    prefix: String,
}

/// An `http(s)` URL with no user or password and no query in it: where a secret is, not one.
fn plain_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@') && !rest.contains('?')
}

/// Whether a key's secret words are only `AUTH`, `AUTHORIZATION`, `CREDENTIAL(S)`: such a key
/// holds an address or a path, not a secret, when its value is a plain URL. `SLACK_TOKEN` or
/// `WEBHOOK_SECRET` does not: a webhook URL is the secret.
fn names_a_place(key: &str) -> bool {
    const SECRET_ONLY: &[&str] = &[
        "TOKEN", "KEY", "SECRET", "PASSWORD", "PASSWD", "PWD", "APIKEY",
    ];
    !key_words(key)
        .iter()
        .any(|w| SECRET_ONLY.contains(&w.as_str()))
}

/// A filesystem path: `C:\…`, `\\server\…`, `~/…`, `./…`, `../…`, or `/a/b` with no `+`
/// or `=` (which a base64 value starting with `/` would have) and no space.
fn looks_like_path(value: &str) -> bool {
    let b = value.as_bytes();
    let sep = |c: u8| c == b'/' || c == b'\\';
    let drive = b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2]);
    let relative = ["~/", "~\\", "./", ".\\", "../", "..\\"]
        .iter()
        .any(|p| value.starts_with(p));
    let posix =
        value.starts_with('/') && value[1..].contains('/') && !value.contains(['+', '=', ' ']);
    drive || value.starts_with("\\\\") || relative || posix
}

/// Whether the value under `key` moves to Personal: a secret-looking key, a value that is not
/// empty, not `${…}`, and not a plain URL under an `AUTH`-like key, or a path.
fn hidden(key: &str, value: &str) -> bool {
    let trimmed = value.trim();
    looks_secret(key)
        && !value.contains("${")
        && !trimmed.is_empty()
        && (!plain_url(trimmed) || !names_a_place(key))
        && !looks_like_path(trimmed)
}

/// The secret-looking values of the server `server`'s config.
fn found(server: &str, config: &McpServerConfig) -> Vec<Found> {
    let mut out = Vec::new();
    match config {
        McpServerConfig::Stdio { env, .. } => {
            for (key, value) in env {
                if hidden(key, value) {
                    out.push(Found {
                        place: Place::Env(key.clone()),
                        name: valid_name(&var_name(key)),
                        value: value.clone(),
                        prefix: String::new(),
                    });
                }
            }
        }
        McpServerConfig::Http { headers, .. } => {
            for (key, value) in headers {
                if !hidden(key, value) {
                    continue;
                }
                let (prefix, secret) = if key.eq_ignore_ascii_case("authorization") {
                    if value.split_whitespace().count() > 1 {
                        let scheme = auth_scheme(value);
                        let secret = match value.trim_start().strip_prefix(scheme) {
                            Some(rest) => rest.trim(),
                            None => value.trim(),
                        };
                        (format!("{scheme} "), secret.to_string())
                    } else {
                        // One word is the whole secret: no scheme is added that it never had.
                        (String::new(), value.trim().to_string())
                    }
                } else {
                    (String::new(), value.clone())
                };
                out.push(Found {
                    place: Place::Header(key.clone()),
                    name: valid_name(&format!("{}_{}", var_name(server), var_name(key))),
                    value: secret,
                    prefix,
                });
            }
        }
    }
    out
}

impl Found {
    /// Why this value cannot move to Personal, if it cannot: too long for the secret store, a
    /// name that is not valid, or a value the vault refuses (a NUL).
    fn unmovable(&self) -> Option<&'static str> {
        if self.value.len() > MAX_SECRET_BYTES {
            Some("is too long to move to the vault")
        } else if check_variable_name(&self.name).is_err() {
            Some("has no valid name")
        } else if check_value(EntryKind::Secret, &self.value).is_err() {
            Some("is refused by the vault")
        } else {
            None
        }
    }

    /// A value that stays where it is is at least scrubbed from what Dots keeps and logs.
    fn stay(&self, server: &str, why: &str) {
        tracing::warn!("{server}: a value of {:?} {why}", self.name);
        redact::global().add(&self.name, &self.value);
    }
}

/// What a config that is being moved replaces: it decides the name a value moves under.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Prior<'a> {
    /// Nothing: a new server, or one a version before the vault stored.
    New,
    /// A plugin's server as the Library stores it. A key it reads as `${NAME}` keeps the name,
    /// and Personal keeps its value: the plugin's file differing is only logged.
    Plugin(&'a McpServerConfig),
    /// A server the person changes in the form, as stored, and its item id. A key that read
    /// `${NAME}` keeps the name, and the new value replaces NAME's in Personal, when nothing
    /// else in the Library reads `${NAME}`.
    Edit {
        id: &'a str,
        config: &'a McpServerConfig,
    },
}

/// The name `value` reads as a whole: `${NAME}`, or `<scheme> ${NAME}` for a header.
fn read_name(value: &str) -> Option<String> {
    let value = value.trim();
    let start = value.rfind("${")?;
    let name = value[start + 2..].strip_suffix('}')?;
    (is_var_name(name) && !value[..start].contains('$')).then(|| name.to_string())
}

/// The name the stored `config` reads at `place`, if it reads one there as a whole.
fn stored_name(config: &McpServerConfig, place: &Place) -> Option<String> {
    match (config, place) {
        (McpServerConfig::Stdio { env, .. }, Place::Env(key)) => {
            env.get(key).and_then(|v| read_name(v))
        }
        (McpServerConfig::Http { headers, .. }, Place::Header(key)) => headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .and_then(|(_, v)| read_name(v)),
        _ => None,
    }
}

/// Whether `config` holds a secret-looking value that is not `${…}` and can move to Personal.
pub fn has_movable_literals(server: &str, config: &McpServerConfig) -> bool {
    found(server, config)
        .iter()
        .any(|f| f.unmovable().is_none())
}

/// Logs each secret-looking value of `config` that cannot move to Personal, by name, and
/// scrubs it from what Dots keeps and logs from now on.
pub fn scrub_unmovable(server: &str, config: &McpServerConfig) {
    for f in found(server, config) {
        if let Some(why) = f.unmovable() {
            f.stay(server, why);
        }
    }
}

impl Vault {
    /// `config` with each secret-looking value moved into Personal as a secret, and the names
    /// it moved under. A name Personal has with the same value is reused; one it has with
    /// another value gets `_2`, `_3`, … A value that cannot move (too long for the secret
    /// store, or refused) stays where it is and is scrubbed.
    pub async fn move_literals(
        &self,
        server: &str,
        config: &McpServerConfig,
    ) -> Result<(McpServerConfig, Vec<String>)> {
        self.move_literals_over(server, config, Prior::New).await
    }

    /// `move_literals` for a config that replaces `prior`: a key the stored config read as
    /// `${NAME}` may keep that name (`Prior`).
    pub(crate) async fn move_literals_over(
        &self,
        server: &str,
        config: &McpServerConfig,
        prior: Prior<'_>,
    ) -> Result<(McpServerConfig, Vec<String>)> {
        let incoming = config;
        let mut config = config.clone();
        let mut moved = Vec::new();
        for f in found(server, &config) {
            if let Some(why) = f.unmovable() {
                f.stay(server, why);
                continue;
            }
            let kept = match prior {
                Prior::New => None,
                Prior::Plugin(stored) => match stored_name(stored, &f.place) {
                    Some(name) => {
                        self.keep_personal(server, &name, &f.value).await?;
                        Some(name)
                    }
                    None => None,
                },
                Prior::Edit { id, config: stored } => match stored_name(stored, &f.place) {
                    Some(name) if self.only_reader(id, incoming, &name).await? => {
                        self.replace_personal(&name, &f.value).await?
                    }
                    _ => None,
                },
            };
            let name = match kept {
                Some(name) => name,
                None => self.free_name(&f).await?,
            };
            let text = format!("{}${{{name}}}", f.prefix);
            match (&mut config, &f.place) {
                (McpServerConfig::Stdio { env, .. }, Place::Env(key)) => {
                    env.insert(key.clone(), text);
                }
                (McpServerConfig::Http { headers, .. }, Place::Header(key)) => {
                    headers.insert(key.clone(), text);
                }
                _ => unreachable!("a place of this config's transport"),
            }
            moved.push(name);
        }
        Ok((config, moved))
    }

    /// The name `f` moves under when it keeps none: its own, or one Personal has with the same
    /// value, or the first free one of `_2`, `_3`, … The value is added when the name is new.
    async fn free_name(&self, f: &Found) -> Result<String> {
        let mut name = f.name.clone();
        for n in 2.. {
            // A name whose row exists but whose secret is gone counts as taken.
            if self.store().find_env_entry(&name).await?.is_none() {
                self.add(&name, EntryKind::Secret, &f.value).await?;
                break;
            }
            match self.personal_value(&name).await? {
                Some(v) if *v == f.value => break,
                _ => name = suffixed(&f.name, n),
            }
        }
        Ok(name)
    }

    /// A plugin's re-synced value for the name `name` its server already reads: Personal wins
    /// and keeps its value. A differing value is logged, by name only, and scrubbed.
    async fn keep_personal(&self, server: &str, name: &str, value: &str) -> Result<()> {
        if self
            .personal_value(name)
            .await?
            .is_some_and(|v| *v == value)
        {
            return Ok(());
        }
        tracing::warn!(
            "{server}: the plugin's file has another value for {name}; Personal's is kept"
        );
        redact::global().add(name, value);
        Ok(())
    }

    /// Whether the server `id`, as `incoming` will store it, is the only Library item that
    /// reads `${name}`: `incoming` no longer reads it, and no other server does.
    async fn only_reader(&self, id: &str, incoming: &McpServerConfig, name: &str) -> Result<bool> {
        let reads =
            |c: &McpServerConfig| c.variables().iter().any(|v| v.eq_ignore_ascii_case(name));
        if reads(incoming) {
            return Ok(false);
        }
        for item in self.store().list_library_items().await? {
            if let LibraryConfig::McpServer(config) = &item.config {
                if item.id != id && reads(config) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Replaces the value of the secret `name` with `value`, adding it when Personal lacks the
    /// name. A plain variable is not replaced (`None`): the value moves under another name.
    async fn replace_personal(&self, name: &str, value: &str) -> Result<Option<String>> {
        match self.store().find_env_entry(name).await? {
            None => {
                self.add(name, EntryKind::Secret, value).await?;
            }
            Some(entry) if entry.kind == EntryKind::Secret => {
                if self
                    .personal_value(&entry.name)
                    .await?
                    .is_none_or(|v| *v != value)
                {
                    self.set_value(&entry.name, value).await?;
                }
            }
            Some(_) => return Ok(None),
        }
        Ok(Some(name.to_string()))
    }
}

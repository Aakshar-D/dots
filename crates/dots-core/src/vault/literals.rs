//! Secret values typed into an MCP server's config (env vault spec §11): a secret-looking env
//! value or header with no `${…}` moves into Personal and the config reads `${NAME}` instead,
//! so the value is never stored in the Library. An env value goes under its key
//! (`SF_TOKEN`), an `Authorization` header under `<SERVER>_AUTHORIZATION` with its scheme
//! kept, another header under `<SERVER>_<HEADER>`.

use super::entries::{check_value, check_variable_name, EntryKind};
use super::secrets::MAX_SECRET_BYTES;
use super::service::Vault;
use crate::library::{looks_secret, McpServerConfig};
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

/// An `http(s)` URL with no user or password in it: where a secret is, not one.
fn plain_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@')
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
/// empty, not `${…}`, and not a plain URL or a path.
fn hidden(key: &str, value: &str) -> bool {
    let trimmed = value.trim();
    looks_secret(key)
        && !value.contains("${")
        && !trimmed.is_empty()
        && !plain_url(trimmed)
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
        let mut config = config.clone();
        let mut moved = Vec::new();
        for f in found(server, &config) {
            if let Some(why) = f.unmovable() {
                f.stay(server, why);
                continue;
            }
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
}

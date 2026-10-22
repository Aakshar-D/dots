//! Secret values typed into an MCP server's config (env vault spec §11): a secret-looking env
//! value or header with no `${…}` moves into Personal and the config reads `${NAME}` instead,
//! so the value is never stored in the Library. An env value goes under its key
//! (`SF_TOKEN`), an `Authorization` header under `<SERVER>_AUTHORIZATION` with its scheme
//! kept, another header under `<SERVER>_<HEADER>`.

use super::entries::{check_variable_name, EntryKind};
use super::secrets::MAX_SECRET_BYTES;
use super::service::Vault;
use crate::library::{looks_secret, McpServerConfig};
use crate::store::{auth_scheme, var_name};
use crate::Result;

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

fn hidden(key: &str, value: &str) -> bool {
    looks_secret(key) && !value.contains("${") && !value.trim().is_empty()
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
                        name: var_name(key),
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
                    let scheme = auth_scheme(value);
                    let secret = match value.trim_start().strip_prefix(scheme) {
                        Some(rest) if value.split_whitespace().count() > 1 => rest.trim(),
                        _ => value.trim(),
                    };
                    (format!("{scheme} "), secret.to_string())
                } else {
                    (String::new(), value.clone())
                };
                out.push(Found {
                    place: Place::Header(key.clone()),
                    name: format!("{}_{}", var_name(server), var_name(key)),
                    value: secret,
                    prefix,
                });
            }
        }
    }
    out
}

/// Whether `config` holds a secret-looking value that is not `${…}`.
pub fn has_literals(server: &str, config: &McpServerConfig) -> bool {
    !found(server, config).is_empty()
}

impl Vault {
    /// `config` with each secret-looking value moved into Personal as a secret, and the names
    /// it moved under. A name Personal has with the same value is reused; one it has with
    /// another value gets `_2`, `_3`, … A value too long for the secret store stays where it is.
    pub async fn move_literals(
        &self,
        server: &str,
        config: &McpServerConfig,
    ) -> Result<(McpServerConfig, Vec<String>)> {
        let mut config = config.clone();
        let mut moved = Vec::new();
        for f in found(server, &config) {
            if f.value.len() > MAX_SECRET_BYTES || check_variable_name(&f.name).is_err() {
                tracing::warn!(
                    "{server}: a value of {:?} is too long to move to the vault",
                    f.name
                );
                continue;
            }
            let mut name = f.name.clone();
            for n in 2.. {
                match self.personal_value(&name).await? {
                    None => {
                        self.add(&name, EntryKind::Secret, &f.value).await?;
                        break;
                    }
                    Some(v) if *v == f.value => break,
                    Some(_) => name = format!("{}_{n}", f.name),
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

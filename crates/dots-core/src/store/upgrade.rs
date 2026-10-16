//! The part of migration 0006 that needs code (MCP/skills spec section 10). The SQL migration
//! moves each dot's old `mcp_servers` JSON into the table `upgrade_0006`; when the store opens,
//! `finish_0006` turns those servers into Library items ticked in the dots' `uses`, brings each
//! dot's policy up to date, and drops the table, in one transaction.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::library::{
    check_name, looks_secret, mcp_server_name, CatalogStatus, LibraryConfig, LibraryKind,
    McpServerConfig, Uses, MAX_SERVER_NAME,
};
use crate::policy::{Action, Policy, Preset};
use crate::util::{new_id, now};
use crate::Result;

/// Converts what migration 0006 left waiting, if anything.
pub(super) async fn finish_0006(pool: &SqlitePool) -> Result<()> {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0006'",
    )
    .fetch_one(pool)
    .await?;
    if waiting == 0 {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT u.dot_id, u.mcp_servers, d.policy FROM upgrade_0006 u \
         JOIN dots d ON d.id = u.dot_id ORDER BY d.created_at, d.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    // The items in the order they are made: the oldest dot's servers keep their names.
    let mut items: Vec<(String, McpServerConfig)> = Vec::new();
    let mut dots: Vec<(String, Uses, String)> = Vec::new();
    for (dot_id, servers, policy) in rows {
        let servers = servers
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let mut uses = Uses::default();
        let mut renames: Vec<(String, String)> = Vec::new();
        for (old, raw) in &servers {
            let Some(name) = place(old, raw, &mut items) else {
                tracing::warn!(
                    dot = %dot_id,
                    "migration 0006: the MCP server {old:?} could not be read and was left out"
                );
                continue;
            };
            if !uses.mcp_servers.contains(&name) {
                uses.mcp_servers.push(name.clone());
            }
            renames.push((old.clone(), name));
        }
        let policy = converted_policy(&policy, &renames).unwrap_or_else(|| {
            tracing::warn!(dot = %dot_id, "migration 0006: the policy could not be read");
            policy
        });
        dots.push((dot_id, uses, policy));
    }
    let ts = now();
    for (name, config) in &items {
        let id = new_id();
        sqlx::query(
            "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, \
             updated_at) VALUES (?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(&id)
        .bind(LibraryKind::McpServer.as_str())
        .bind(name)
        .bind(serde_json::to_string(config)?)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO mcp_catalogs (server_id, status) VALUES (?, ?)")
            .bind(&id)
            .bind(CatalogStatus::Unchecked.as_str())
            .execute(&mut *tx)
            .await?;
    }
    for (dot_id, uses, policy) in &dots {
        sqlx::query("UPDATE dots SET uses = ?, policy = ? WHERE id = ?")
            .bind(serde_json::to_string(uses)?)
            .bind(policy)
            .bind(dot_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE upgrade_0006")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The item that the old server `old` with config `raw` becomes: one of `items` with the same
/// name and config, or a new one added to them. A name that is taken by another config gets
/// `-2`, `-3`, ... `None` when `raw` is not a server config Dots can read.
fn place(old: &str, raw: &Value, items: &mut Vec<(String, McpServerConfig)>) -> Option<String> {
    let base = mcp_server_name(old);
    converted(&base, raw)?;
    for n in 1.. {
        let name = if n == 1 {
            base.clone()
        } else {
            let suffix = format!("-{n}");
            let keep = base.len().min(MAX_SERVER_NAME - suffix.len());
            format!("{}{suffix}", &base[..keep])
        };
        if check_name(LibraryKind::McpServer, &name).is_err() {
            continue;
        }
        // The config depends on the name: an Authorization header reads `<NAME>_TOKEN`.
        let config = converted(&name, raw)?;
        match items.iter().find(|(taken, _)| *taken == name) {
            Some((_, existing)) if *existing == config => return Some(name),
            Some(_) => continue,
            None => {
                items.push((name.clone(), config));
                return Some(name);
            }
        }
    }
    unreachable!("some suffix is free")
}

/// An upper-case variable name made from `s`: `-` and other characters become `_`.
fn var_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// A server of a Claude Code `mcpServers` block as a Library config for the server `name`. A
/// secret-looking value that is not a `${VAR}` reference is not copied: an env value becomes
/// `${<KEY>}`, an `Authorization` header `Bearer ${<NAME>_TOKEN}`, another header
/// `${<HEADER>}`. An `sse` server is read as `http`.
fn converted(name: &str, raw: &Value) -> Option<McpServerConfig> {
    let obj = raw.as_object()?;
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let strings = |key: &str| -> BTreeMap<String, String> {
        obj.get(key)
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), text(v))).collect())
            .unwrap_or_default()
    };
    let hidden = |key: &str, value: &str| looks_secret(key) && !value.contains("${");
    let default_type = if obj.contains_key("url") {
        "http"
    } else {
        "stdio"
    };
    let config = match obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(default_type)
    {
        "stdio" => McpServerConfig::Stdio {
            command: obj.get("command")?.as_str()?.to_string(),
            args: obj
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(text).collect())
                .unwrap_or_default(),
            cwd: obj.get("cwd").and_then(Value::as_str).map(String::from),
            env: strings("env")
                .into_iter()
                .map(|(k, v)| {
                    let v = if hidden(&k, &v) {
                        format!(
                            "${{{}}}",
                            k.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
                        )
                    } else {
                        v
                    };
                    (k, v)
                })
                .collect(),
        },
        "http" | "sse" => McpServerConfig::Http {
            url: obj.get("url")?.as_str()?.to_string(),
            headers: strings("headers")
                .into_iter()
                .map(|(k, v)| {
                    let v = if !hidden(&k, &v) {
                        v
                    } else if k.eq_ignore_ascii_case("authorization") {
                        format!("Bearer ${{{}_TOKEN}}", var_name(name))
                    } else {
                        format!("${{{}}}", var_name(&k))
                    };
                    (k, v)
                })
                .collect(),
        },
        _ => return None,
    };
    LibraryConfig::McpServer(config.clone())
        .validate()
        .ok()
        .map(|_| config)
}

/// `tool` with the server renamed, when it names one of `renames` (`mcp__<old>` or
/// `mcp__<old>__...`); the longest old name wins.
fn renamed(tool: &str, renames: &[(String, String)]) -> Option<String> {
    renames
        .iter()
        .filter(|(old, new)| old != new)
        .filter_map(|(old, new)| {
            let rest = tool.strip_prefix(&format!("mcp__{old}"))?;
            (rest.is_empty() || rest.starts_with("__"))
                .then(|| (old.len(), format!("mcp__{new}{rest}")))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, tool)| tool)
}

/// The stored policy brought up to date: the Trusted preset's old `mcp__*` ask rule removed,
/// rules that name a renamed server renamed, and `mcp_default` set when it is missing (`deny`
/// for Read-only, else `ask`). `None` when the stored JSON is not a policy.
fn converted_policy(stored: &str, renames: &[(String, String)]) -> Option<String> {
    let mut value: Value = serde_json::from_str(stored).ok()?;
    let obj = value.as_object_mut()?;
    let preset = obj.get("preset").and_then(Value::as_str).map(String::from);
    if let Some(Value::Array(rules)) = obj.get_mut("rules") {
        if preset.as_deref() == Some(Preset::Trusted.as_str()) {
            rules.retain(|r| !(r["tool"] == "mcp__*" && r["action"] == Action::Ask.as_str()));
        }
        for rule in rules.iter_mut() {
            if let Some(tool) = rule["tool"].as_str().and_then(|t| renamed(t, renames)) {
                rule["tool"] = Value::String(tool);
            }
        }
    }
    if !obj.contains_key("mcp_default") {
        let action = if preset.as_deref() == Some(Preset::ReadOnly.as_str()) {
            Action::Deny
        } else {
            Action::Ask
        };
        obj.insert("mcp_default".into(), json!(action.as_str()));
    }
    // Through `Policy`, so the JSON is written as Dots writes a policy.
    let policy: Policy = serde_json::from_value(value).ok()?;
    serde_json::to_string(&policy).ok()
}

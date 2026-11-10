use serde_json::Value;

use crate::engine::EngineEvent;

const MAX_OUTPUT: usize = 64 * 1024;

/// Maps one line of `claude -p --output-format stream-json --verbose` output to events. A
/// result's window is the largest it names; `LineParser` knows the run's model.
pub fn parse_line(line: &str) -> Vec<EngineEvent> {
    parse_with(line, &mut None)
}

/// Parses a run's lines in order, keeping the model `system/init` names, so that the result's
/// window is that model's (context rules spec section 3), not a helper's.
#[derive(Debug, Default)]
pub struct LineParser {
    model: Option<String>,
}

impl LineParser {
    pub fn parse(&mut self, line: &str) -> Vec<EngineEvent> {
        parse_with(line, &mut self.model)
    }
}

fn parse_with(line: &str, model: &mut Option<String>) -> Vec<EngineEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let raw = || {
        vec![EngineEvent::Raw {
            line: line.to_string(),
        }]
    };
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return raw();
    };
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            let mut out: Vec<EngineEvent> = v
                .get("session_id")
                .and_then(Value::as_str)
                .map(|s| EngineEvent::SessionStarted {
                    session_id: s.to_string(),
                })
                .into_iter()
                .collect();
            out.extend(plugin_errors(&v));
            // A `[1m]` model has a 1M window (context rules spec section 3).
            if let Some(named) = v.get("model").and_then(Value::as_str) {
                *model = Some(named.to_string());
                if crate::context_rules::is_1m(named) {
                    out.push(EngineEvent::Context {
                        fill: None,
                        window: Some(crate::context_rules::WINDOW_1M),
                    });
                }
            }
            if out.is_empty() {
                return raw();
            }
            out
        }
        Some("assistant") => {
            // A helper's calls (the `Task`/`Agent` tool's) name the call that started it.
            let parent = v
                .get("parent_tool_use_id")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty());
            let mut out: Vec<EngineEvent> = blocks(&v)
                .iter()
                .filter_map(|b| assistant_block(b, parent))
                .collect();
            // A helper's turns fill its own context, not the session's.
            if parent.is_none() {
                out.extend(turn_fill(&v));
            }
            out
        }
        // The CLI's progress lines (CLI 2.1.322): a compaction starting or ending, and its
        // estimate of thinking tokens. Only a failed compaction is told, as a notice.
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("status") => {
            match v.get("compact_result").and_then(Value::as_str) {
                Some("failed") => vec![EngineEvent::Notice {
                    text: crate::context_rules::auto_compact_failed(
                        v.get("compact_error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                    ),
                }],
                _ => Vec::new(),
            }
        }
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("thinking_tokens") => {
            Vec::new()
        }
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
            compact_boundary(&v).into_iter().collect()
        }
        Some("user") => blocks(&v).iter().filter_map(tool_result_block).collect(),
        Some("result") => result_events(&v, model.as_deref()),
        Some("rate_limit_event") => limit_events(&v),
        _ => raw(),
    }
}

/// The plugins the CLI could not load, from `system/init`'s `plugin_errors` (MCP/skills spec
/// section 7), each as a notice. Its entries' shape is not documented, so an object is read for
/// a plugin name (`plugin`, `name` or `source`) and an error (`error` or `message`), a string
/// is the error, and anything else is shown as JSON.
fn plugin_errors(init: &Value) -> Vec<EngineEvent> {
    let Some(errors) = init.get("plugin_errors").and_then(Value::as_array) else {
        return Vec::new();
    };
    let field = |e: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| e.get(*k).and_then(Value::as_str))
            .map(str::to_string)
    };
    errors
        .iter()
        .map(|e| {
            let error = match e {
                Value::String(s) => Some(s.clone()),
                _ => field(e, &["error", "message"]),
            };
            let text = match (field(e, &["plugin", "name", "source"]), error) {
                (Some(plugin), Some(error)) => format!("plugin {plugin}: {error}"),
                (None, Some(error)) => format!("plugin error: {error}"),
                _ => format!("plugin error: {e}"),
            };
            EngineEvent::Notice { text }
        })
        .collect()
}

fn blocks(v: &Value) -> Vec<Value> {
    v.pointer("/message/content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn assistant_block(b: &Value, parent: Option<&str>) -> Option<EngineEvent> {
    match b.get("type")?.as_str()? {
        "text" => Some(EngineEvent::AssistantText {
            text: b.get("text")?.as_str()?.to_string(),
        }),
        "tool_use" => Some(EngineEvent::ToolCall {
            id: str_field(b, "id"),
            tool: str_field(b, "name"),
            input: b.get("input").cloned().unwrap_or(Value::Null),
            parent: parent.map(str::to_string),
        }),
        _ => None,
    }
}

fn tool_result_block(b: &Value) -> Option<EngineEvent> {
    if b.get("type")?.as_str()? != "tool_result" {
        return None;
    }
    let output = match b.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    Some(EngineEvent::ToolResult {
        id: str_field(b, "tool_use_id"),
        output: truncate(output),
        is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Keeps the first `MAX_OUTPUT` bytes. Secrets are scrubbed first: a cut inside one would leave
/// a piece that no scrub matches.
fn truncate(s: String) -> String {
    let mut s = crate::vault::redact::global().scrub(&s).into_owned();
    if s.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n[truncated]");
    }
    s
}

/// A `rate_limit_event` line (usage and limits spec section 3): its status and a reading for
/// each window it names, from `unifiedWindows` when the CLI sends it, else the one limiting
/// window (`rateLimitType`) when it has a utilization. A line whose status cannot be read gives
/// nothing; it is never a failure of the run.
fn limit_events(v: &Value) -> Vec<EngineEvent> {
    let Some(info) = v.get("rate_limit_info").filter(|i| i.is_object()) else {
        tracing::debug!("unreadable rate_limit_event: {v}");
        return Vec::new();
    };
    let Some(status) = info.get("status").and_then(Value::as_str) else {
        tracing::debug!("rate_limit_event without a status: {v}");
        return Vec::new();
    };
    let read_at = crate::util::now();
    let reading = |window: &str, w: &Value, fallback_status: &str| {
        let utilization = w.get("utilization").and_then(Value::as_f64)?;
        let resets_at = w.get("resetsAt").and_then(Value::as_i64)?;
        let resets_at = chrono::DateTime::from_timestamp(resets_at, 0)?;
        Some(crate::store::LimitReading {
            window: window.to_string(),
            utilization,
            resets_at: crate::util::timestamp(resets_at),
            status: w
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or(fallback_status)
                .to_string(),
            read_at: read_at.clone(),
        })
    };
    let windows = match info.get("unifiedWindows").and_then(Value::as_object) {
        Some(all) => all
            .iter()
            .filter_map(|(name, w)| reading(name, w, status))
            .collect(),
        None => info
            .get("rateLimitType")
            .and_then(Value::as_str)
            .and_then(|name| reading(name, info, status))
            .into_iter()
            .collect(),
    };
    let resets_at = info
        .get("resetsAt")
        .and_then(Value::as_i64)
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(crate::util::timestamp);
    vec![EngineEvent::Limits {
        status: status.to_string(),
        windows,
        resets_at,
    }]
}

/// The context a main-thread turn sent (context rules spec section 3): its input tokens,
/// cache writes and cache reads. Nothing for a line without usage.
fn turn_fill(v: &Value) -> Option<EngineEvent> {
    let usage = v.pointer("/message/usage")?.as_object()?;
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let fill = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    Some(EngineEvent::Context {
        fill: Some(fill),
        window: None,
    })
}

/// A `system/compact_boundary` line: the CLI compacted the session (context rules spec
/// section 2).
fn compact_boundary(v: &Value) -> Option<EngineEvent> {
    let meta = v.get("compact_metadata")?;
    Some(EngineEvent::Compacted {
        trigger: meta
            .get("trigger")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string(),
        before: meta.get("pre_tokens").and_then(Value::as_u64)?,
        after: meta.get("post_tokens").and_then(Value::as_u64)?,
    })
}

/// The `contextWindow` the result's `modelUsage` names for `model` (the run's, from
/// `system/init`, with or without its `[1m]`), else the largest it names.
fn result_window(v: &Value, model: Option<&str>) -> Option<u64> {
    let all = v.get("modelUsage")?.as_object()?;
    let window = |m: &Value| m.get("contextWindow").and_then(Value::as_u64);
    if let Some(model) = model {
        let lower = model.to_ascii_lowercase();
        let bare = lower.trim_end_matches("[1m]");
        let own = all.iter().find_map(|(name, m)| {
            let name = name.to_ascii_lowercase();
            (name == lower || name == bare || name.trim_end_matches("[1m]") == bare)
                .then(|| window(m))
                .flatten()
        });
        if own.is_some() {
            return own;
        }
    }
    all.values().filter_map(window).max()
}

/// What one CLI call used, from its `result` line (context rules spec section 5): the
/// models' use since the last result when the CLI gives it (a `/compact` result's own
/// `usage` is all zero), else the result's `usage` and `total_cost_usd`. `None` for any other
/// line.
pub fn pass_usage(line: &str) -> Option<crate::engine::PassUsage> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("result") {
        return None;
    }
    let n = |m: &Value, k: &str| m.get(k).and_then(Value::as_u64).unwrap_or(0);
    if let Some(models) = v
        .get("model_usage_since_last_result")
        .and_then(Value::as_object)
    {
        let mut u = crate::engine::PassUsage::default();
        let mut cost = None;
        for m in models.values() {
            let read = n(m, "cacheReadInputTokens");
            u.tokens_in += n(m, "inputTokens") + n(m, "cacheCreationInputTokens") + read;
            u.tokens_out += n(m, "outputTokens");
            u.cache_read += read;
            if let Some(c) = m.get("costUSD").and_then(Value::as_f64) {
                cost = Some(cost.unwrap_or(0.0) + c);
            }
        }
        u.cost_usd = cost;
        return Some(u);
    }
    let usage = v.get("usage").cloned().unwrap_or(Value::Null);
    let read = n(&usage, "cache_read_input_tokens");
    Some(crate::engine::PassUsage {
        tokens_in: n(&usage, "input_tokens") + n(&usage, "cache_creation_input_tokens") + read,
        tokens_out: n(&usage, "output_tokens"),
        cache_read: read,
        cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
    })
}

fn result_events(v: &Value, model: Option<&str>) -> Vec<EngineEvent> {
    let usage = v.get("usage");
    let n = |k: &str| {
        usage
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let mut out = vec![EngineEvent::Usage {
        tokens_in: n("input_tokens")
            + n("cache_creation_input_tokens")
            + n("cache_read_input_tokens"),
        tokens_out: n("output_tokens"),
        cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
        cache_read: n("cache_read_input_tokens"),
    }];
    if let Some(window) = result_window(v, model) {
        out.push(EngineEvent::Context {
            fill: None,
            window: Some(window),
        });
    }
    let subtype = v
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let text = str_field(v, "result");
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    if subtype == "success" && !is_error {
        out.push(EngineEvent::Finished { summary: text });
    } else if text.is_empty() {
        out.push(EngineEvent::Failed {
            error: subtype.to_string(),
        });
    } else {
        out.push(EngineEvent::Failed {
            error: format!("{subtype}: {text}"),
        });
    }
    out
}

#[cfg(test)]
mod redact_tests {
    use super::*;

    #[test]
    fn a_tool_output_is_scrubbed_before_it_is_cut() {
        let secret = "ghp_parse_cut_value_0123456789";
        crate::vault::redact::global().add("DOTS_PARSE_CUT", secret);
        // The cut falls 12 bytes into the secret.
        let output = format!("{}{secret}tail", "o".repeat(MAX_OUTPUT - 12));
        let out = truncate(output);
        assert!(!out.contains(&secret[..8]), "{}", &out[MAX_OUTPUT - 40..]);
    }
}

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::engine::RunContext;

pub fn build_args(ctx: &RunContext, mcp_config_path: &Path) -> Vec<String> {
    let spec = &ctx.dot.spec;
    let mut a: Vec<String> = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--model".into(),
        spec.model.clone(),
        "--max-turns".into(),
        spec.max_turns.to_string(),
        "--permission-prompt-tool".into(),
        "mcp__dots__approve".into(),
        "--mcp-config".into(),
        mcp_config_path.to_string_lossy().to_string(),
        "--strict-mcp-config".into(),
    ];
    // Empty sources keep every settings file (user hooks, and a repo's committed
    // `.claude/settings.json` that could widen the allow list) out of headless runs.
    // Accepted by CLI 2.1.295 (probed 2026-10-09).
    a.push("--setting-sources".into());
    a.push(if spec.use_user_settings {
        "user,project,local".into()
    } else {
        String::new()
    });
    let (allow, deny) = spec.policy.cli_lists();
    if !allow.is_empty() {
        a.push("--allowedTools".into());
        a.extend(allow);
    }
    if !deny.is_empty() {
        a.push("--disallowedTools".into());
        a.extend(deny);
    }
    if let Some(session) = &ctx.session_id {
        a.push("--resume".into());
        a.push(session.clone());
    }
    a
}

pub fn mcp_config(ctx: &RunContext) -> Value {
    let mut servers = serde_json::Map::new();
    if let Some(Value::Object(extra)) = &ctx.dot.spec.mcp_servers {
        for (name, cfg) in extra {
            servers.insert(name.clone(), cfg.clone());
        }
    }
    servers.insert(
        "dots".into(),
        json!({
            "type": "http",
            "url": ctx.mcp_url,
            "headers": { "Authorization": format!("Bearer {}", ctx.mcp_secret) },
        }),
    );
    json!({ "mcpServers": servers })
}

/// The configured path if it is a file; otherwise search PATH.
pub fn resolve_claude(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    resolve_in_dirs(std::env::split_paths(&path))
}

/// On Windows only native `claude.exe` is spawnable; an npm `claude.cmd` shim is followed to
/// `node_modules\@anthropic-ai\claude-code\bin\claude.exe` next to it.
pub fn resolve_in_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for dir in dirs {
        if cfg!(windows) {
            let exe = dir.join("claude.exe");
            if exe.is_file() {
                return Some(exe);
            }
            if dir.join("claude.cmd").is_file() {
                let native = dir
                    .join("node_modules")
                    .join("@anthropic-ai")
                    .join("claude-code")
                    .join("bin")
                    .join("claude.exe");
                if native.is_file() {
                    return Some(native);
                }
            }
        } else {
            let bin = dir.join("claude");
            if bin.is_file() {
                return Some(bin);
            }
        }
    }
    None
}

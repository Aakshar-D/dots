use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::checkins::Tool;
use crate::engine::RunContext;
use crate::model::EngineKind;

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
    let (mut allow, deny) = spec.policy.cli_lists();
    // The gateway checks every call of a run's MCP server (spec section 4), so the CLI does not
    // ask the `approve` tool for them first.
    allow.extend(
        ctx.tools
            .servers()
            .iter()
            .map(|s| format!("mcp__{}__*", s.name())),
    );
    // The built-in browser's gate decides each of its calls (browser spec section 3).
    if ctx.tools.browser().is_some() {
        allow.push(format!("{}*", crate::browser::PREFIX));
    }
    // Dots' own `dots` server checks each `context` call against the policy itself (context
    // library spec section 4).
    if !ctx.tools.contexts().is_empty() {
        allow.push("mcp__dots__context".into());
    }
    // It checks each call of its check-in tools too (check-ins spec section 2).
    allow.extend(Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
    // And of its memory tools (dot memory spec section 3).
    allow.extend(crate::memory::Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
    // And of `propose_app` (app library spec section 6.2), which the dot calls by that name.
    allow.push(format!("mcp__dots__{}", crate::apps::propose::TOOL));
    // And of the run's app tools (app library spec section 6).
    if let Some(apps) = ctx.tools.apps() {
        allow.extend(apps.model_names(EngineKind::Claude));
    }
    // And of `start_workflow`, for a run that may start a workflow (teams spec section 7).
    if !ctx.tools.startable().is_empty() {
        allow.push(
            crate::team_tools::Tool::StartWorkflow
                .model_name(EngineKind::Claude)
                .to_string(),
        );
    }
    if !allow.is_empty() {
        a.push("--allowedTools".into());
        a.extend(allow);
    }
    if !deny.is_empty() {
        a.push("--disallowedTools".into());
        a.extend(deny);
    }
    for dir in ctx.tools.plugin_dirs() {
        a.push("--plugin-dir".into());
        a.push(dir.to_string_lossy().to_string());
    }
    if let Some(session) = &ctx.session_id {
        a.push("--resume".into());
        a.push(session.clone());
    }
    a
}

/// The run's `--mcp-config`: Dots' own `dots` server and, for each MCP session the run has open,
/// an entry of the same name that reaches it through the gateway. The CLI never starts or
/// reaches a server itself, and no `${VAR}` value is written to the file.
pub fn mcp_config(ctx: &RunContext) -> Value {
    let mut servers = serde_json::Map::new();
    for server in ctx.tools.servers() {
        servers.insert(
            server.name().to_string(),
            gateway_entry(&ctx.mcp_url, &ctx.mcp_secret, server.name()),
        );
    }
    if ctx.tools.browser().is_some() {
        servers.insert(
            crate::browser::SERVER.to_string(),
            gateway_entry(&ctx.mcp_url, &ctx.mcp_secret, crate::browser::SERVER),
        );
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

/// The `mcpServers` entry of the run's open session `server`: the gateway route
/// `<mcp_url>/s/<server>` with the run's secret (spec section 5).
pub fn gateway_entry(mcp_url: &str, secret: &str, server: &str) -> Value {
    json!({
        "type": "http",
        "url": format!("{}/s/{server}", mcp_url.trim_end_matches('/')),
        "headers": { "Authorization": format!("Bearer {secret}") },
    })
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

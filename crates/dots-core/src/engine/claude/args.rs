use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::checkins::Tool;
use crate::engine::RunContext;
use crate::model::EngineKind;
use crate::policy::{cli_private_ask_rules, PLUMBING_TOOLS};

/// The CLI's search and listing tools, never in its allow list.
const SEARCH_TOOLS: &[&str] = &["Grep", "Glob", "LS"];

/// The CLI's shell tools.
const COMMAND_TOOLS: &[&str] = &["Bash", "PowerShell"];

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
    // Searches and listings always reach `approve`, even where the policy allows them: the CLI
    // checks a search only against the folder it starts in, and Dots also asks when that
    // folder holds a private place (approval guards spec section 3.4). Dots answers the rest
    // with an allow at once.
    allow.retain(|rule| {
        let name = rule.split('(').next().unwrap_or(rule).trim();
        !SEARCH_TOOLS.contains(&name)
    });
    // While "Always ask before sending or deleting" is on, every shell command reaches
    // `approve`: the CLI would run one its allow list matches (`Bash(git:*)` covers
    // `git push --force`) without asking, and only `Policy::resolve_in` reads a command for
    // sends and deletes. Dots answers the rest with an allow at once.
    if spec.policy.ask_send_delete {
        allow.retain(|rule| {
            let name = rule.split('(').next().unwrap_or(rule).trim();
            !COMMAND_TOOLS.contains(&name)
        });
    }
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
    // And each `graph` call (project graph spec section 8).
    if ctx.tools.graph().is_some() {
        allow.push(crate::graph::model_name(EngineKind::Claude).into());
    }
    // It checks each call of its check-in tools too (check-ins spec section 2).
    allow.extend(Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
    // And of its memory tools (dot memory spec section 3).
    allow.extend(crate::memory::Tool::ALL.map(|t| t.model_name(EngineKind::Claude).to_string()));
    // And of `propose_app` and `copy_app` (app library spec section 6.2), which the dot calls
    // by those names.
    allow.push(format!("mcp__dots__{}", crate::apps::propose::TOOL));
    allow.push(format!("mcp__dots__{}", crate::apps::propose::COPY_TOOL));
    // And of the run's app tools (app library spec section 6).
    if let Some(apps) = ctx.tools.apps() {
        allow.extend(apps.model_names(EngineKind::Claude));
    }
    // And of the workflow tools the run has (teams spec section 7): `start_workflow` for a run
    // that may start a workflow, a step's tools for a run of an open agent step.
    allow.extend(
        crate::team_tools::Tool::ALL
            .into_iter()
            .filter(|t| ctx.tools.offers(*t))
            .map(|t| t.model_name(EngineKind::Claude).to_string()),
    );
    // And of `propose_instructions`, for a run of a chat the person started (conversational
    // chat spec section 4).
    if ctx.tools.instructions() {
        allow.push(crate::instructions::model_name(EngineKind::Claude).to_string());
    }
    // The model's plumbing tools never ask (approval guards spec section 5), unless a deny or
    // an ask rule matches one, as `Policy::resolve_in` reads it: that one is left to the
    // other lists and to the `approve` tool.
    allow.extend(
        PLUMBING_TOOLS
            .iter()
            .filter(|t| spec.policy.plumbing_allowed(t))
            .map(|t| t.to_string()),
    );
    if !allow.is_empty() {
        a.push("--allowedTools".into());
        a.extend(allow);
    }
    if !deny.is_empty() {
        a.push("--disallowedTools".into());
        a.extend(deny);
    }
    // The CLI allows a call that matches `--allowedTools` without asking Dots, so the private
    // folders get ask rules of the CLI's own (`cli_settings`, written by the engine next to
    // the MCP config): an ask rule beats an allow rule there, and the CLI then asks
    // `approve`, where `Policy::resolve_in` decides (approval guards spec section 3.4).
    a.push("--settings".into());
    a.push(settings_path(mcp_config_path).to_string_lossy().to_string());
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

/// The run's `--settings` file, next to its `--mcp-config` (`mcp-RUN.json` gives
/// `mcp-RUN.settings.json`). A file and not an inline argument: a configured program may be a
/// `.cmd`, and cmd.exe would read the JSON's quotes and `*` itself.
pub fn settings_path(mcp_config_path: &Path) -> PathBuf {
    mcp_config_path.with_extension("settings.json")
}

/// What the run's `--settings` file holds: the CLI's ask rules for the private folders.
pub fn cli_settings() -> Value {
    json!({ "permissions": { "ask": cli_private_ask_rules() } })
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

/// The configured path if it is a file; otherwise search PATH, and on macOS then the folders
/// the CLI's installers use (`mac_claude_dirs`): an app opened from the Finder gets a short
/// PATH that has none of them.
pub fn resolve_claude(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured {
        return p.is_file().then(|| p.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        dirs.extend(mac_claude_dirs(std::env::var_os("HOME").map(PathBuf::from)));
    }
    resolve_in_dirs(dirs)
}

/// Where the Claude Code CLI is installed on macOS, given the home folder: the native
/// installer's `~/.local/bin` and `~/.claude/local`, Homebrew's `/opt/homebrew/bin` and
/// `/usr/local/bin`, and the usual npm, Volta and Bun global folders.
pub fn mac_claude_dirs(home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home.filter(|h| !h.as_os_str().is_empty()) {
        for sub in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".volta/bin",
            ".bun/bin",
        ] {
            dirs.push(home.join(sub));
        }
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
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

//! One sample of every type the UI receives or sends, written to
//! `ui/src/api/__fixtures__/contract.json`. The UI's contract test parses each sample with its
//! schema, so a field renamed or added on one side fails the other side's tests.
//! Regenerate after changing a type: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.

use dots_core::engine::EngineEvent;
use dots_core::events::RuntimeEvent;
use dots_core::kit::InstalledPlugin;
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig,
    PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::{
    EnvVariable, LibraryEntry, McpProbe, PluginDetail, ScannedSkill, SkillDetail,
};
use dots_core::mcp::McpTool;
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
use dots_core::schedule::Schedule;
use dots_core::scheduler::MAX_ATTEMPTS;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS, MAX_GLOBAL_INSTRUCTIONS};
use serde_json::{json, Value};

use crate::error::{CmdError, CmdErrorKind};
use crate::state::StartupError;

const TS: &str = "2026-10-09T12:00:00.000Z";

fn to(v: impl serde::Serialize) -> Value {
    serde_json::to_value(v).expect("sample serializes")
}

fn spec() -> DotSpec {
    let mut s = DotSpec::new(
        "nightly-tests",
        "Run the tests and summarize failures.",
        "C:/dev/app",
    );
    s.schedule = Some(Schedule::cron("0 0 2 * * *"));
    s.uses = Uses {
        mcp_servers: vec!["github".into()],
        skills: vec!["pdf".into()],
        plugins: vec!["pdf-tools".into()],
    };
    s
}

fn local_spec() -> DotSpec {
    let mut s = DotSpec::new("notes", "Tidy the notes folder.", "C:/notes");
    s.engine = EngineKind::Local;
    s.model = "google/gemma-4-12b-qat".into();
    s.endpoint_url = Some("http://127.0.0.1:1234".into());
    s.workspace_mode = WorkspaceMode::Folder;
    s.policy = Policy::preset(Preset::ReadOnly);
    s
}

fn dot() -> Dot {
    Dot {
        id: "01J0DOT".into(),
        webhook_token: "tok_abc".into(),
        created_at: TS.into(),
        updated_at: TS.into(),
        spec: spec(),
    }
}

fn stdio_server() -> McpServerConfig {
    McpServerConfig::Stdio {
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
        cwd: None,
        env: [("GITHUB_TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    }
}

fn http_server() -> McpServerConfig {
    McpServerConfig::Http {
        url: "https://mcp.example.com/mcp".into(),
        headers: [(
            "Authorization".to_string(),
            "Bearer ${EXAMPLE_TOKEN}".to_string(),
        )]
        .into(),
    }
}

fn skill_config() -> LibraryConfig {
    LibraryConfig::Skill(SkillConfig {
        path: "C:/skills/pdf".into(),
    })
}

fn plugin_config() -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: "C:/plugins/pdf-tools".into(),
        installed_id: Some("pdf-tools@market".into()),
        run_hooks: false,
    })
}

/// A config of every kind and transport, as `library_add` takes them.
fn library_configs() -> Vec<LibraryConfig> {
    vec![
        LibraryConfig::McpServer(stdio_server()),
        LibraryConfig::McpServer(http_server()),
        skill_config(),
        plugin_config(),
    ]
}

fn library_item(id: &str, name: &str, config: LibraryConfig) -> LibraryItem {
    LibraryItem {
        id: id.into(),
        name: name.into(),
        config,
        owner_id: None,
        created_at: TS.into(),
        updated_at: TS.into(),
    }
}

fn tool() -> McpTool {
    McpTool {
        name: "search_issues".into(),
        title: Some("Search issues".into()),
        description: Some("Search the issues of a repository.".into()),
        input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        annotations: Some(json!({"readOnlyHint": true})),
    }
}

fn catalog(status: CatalogStatus) -> McpCatalog {
    let tools = to([tool()]);
    McpCatalog {
        server_id: "01J0SRV".into(),
        status,
        accepted_fingerprint: Some("fp-accepted".into()),
        accepted_tools: Some(tools.clone()),
        seen_fingerprint: Some("fp-seen".into()),
        seen_tools: Some(tools),
        checked_at: Some(TS.into()),
        error: None,
    }
}

/// An entry of every kind: a server whose list changed, a plugin's own server, a skill that
/// could not be read, and a plugin with every part.
fn library_entries() -> Vec<LibraryEntry> {
    let users = vec![DotRef {
        id: "01J0DOT".into(),
        name: "nightly-tests".into(),
    }];
    let bare = |item: LibraryItem, used_by: Vec<DotRef>| LibraryEntry {
        item,
        used_by,
        catalog: None,
        missing_variables: vec![],
        skill: None,
        plugin: None,
        problem: None,
    };
    let github = library_item(
        "01J0SRV",
        "github",
        LibraryConfig::McpServer(stdio_server()),
    );
    let mut owned = library_item(
        "01J0SRV2",
        "plugin-pdf-tools-render",
        LibraryConfig::McpServer(http_server()),
    );
    owned.owner_id = Some("01J0PLG".into());
    let skill = library_item("01J0SKL", "pdf", skill_config());
    let plugin = library_item("01J0PLG", "pdf-tools", plugin_config());
    vec![
        LibraryEntry {
            catalog: Some(catalog(CatalogStatus::Changed)),
            missing_variables: vec!["GITHUB_TOKEN".into()],
            ..bare(github, users.clone())
        },
        LibraryEntry {
            catalog: Some(catalog(CatalogStatus::Accepted)),
            ..bare(owned, users.clone())
        },
        LibraryEntry {
            skill: Some(SkillDetail {
                description: "Fill and read PDF forms.".into(),
            }),
            problem: Some("SKILL.md not found".into()),
            ..bare(skill, vec![])
        },
        LibraryEntry {
            plugin: Some(PluginDetail {
                version: Some("1.2.0".into()),
                description: Some("PDF tools.".into()),
                skills: vec!["pdf-tools:merge".into()],
                commands: vec!["split".into()],
                agents: vec!["reviewer".into()],
                servers: vec!["plugin-pdf-tools-render".into()],
                has_hooks: true,
                warnings: vec!["agents/broken.md: could not be read".into()],
            }),
            ..bare(plugin, users)
        },
    ]
}

fn run() -> Run {
    Run {
        id: "01J0RUN".into(),
        dot_id: "01J0DOT".into(),
        root_run_id: "01J0RUN".into(),
        parent_run_id: None,
        trigger: TriggerKind::Schedule,
        payload: Some(json!({"scheduled_for": "2026-10-09T02:00:00+01:00"})),
        status: RunStatus::Succeeded,
        session_id: Some("sess-1".into()),
        workspace_path: Some("C:/Users/u/AppData/Local/dots/worktrees/01J0RUN".into()),
        branch: Some("dots/nightly-tests/01J0RUN".into()),
        base_commit: Some("0123abcd".into()),
        summary: Some("All tests pass.".into()),
        error: None,
        tokens_in: 1200,
        tokens_out: 340,
        queued_at: TS.into(),
        started_at: Some(TS.into()),
        ended_at: Some(TS.into()),
        reviewed_at: None,
        retry_of: None,
    }
}

fn approval() -> Approval {
    Approval {
        id: "01J0APR".into(),
        run_id: "01J0RUN".into(),
        tool: "Bash".into(),
        input: json!({"command": "git push origin main"}),
        input_hash: "h1".into(),
        status: ApprovalStatus::Pending,
        note: None,
        parked: false,
        resolved: false,
        created_at: TS.into(),
        decided_at: None,
        self_apply: false,
        apply_state: None,
        applied_at: None,
        mcp_fingerprint: None,
    }
}

fn event() -> RunEventRecord {
    RunEventRecord {
        run_id: "01J0RUN".into(),
        seq: 3,
        ts: TS.into(),
        kind: "tool_call".into(),
        data: json!({"kind": "tool_call", "id": "t1", "tool": "Read", "input": {"file_path": "a.rs"}}),
    }
}

fn status() -> Status {
    Status {
        port: 47321,
        server_error: None,
        claude: ClaudeStatus {
            program: Some("C:/Users/u/AppData/Roaming/npm/claude.exe".into()),
            version: Some("2.1.300".into()),
            tested_version: "2.1.295".into(),
            mismatch: true,
        },
        schedules_paused: false,
        restart_required: false,
        data_dir: "C:/Users/u/AppData/Local/dots".into(),
    }
}

/// The sample of `e`'s own variant. The match has no wildcard arm and the array's length is
/// part of its type, so a new `EngineEvent` variant does not compile until it has a name
/// here and a sample in `engine_events`.
fn engine_event_sample<'a>(samples: &'a [EngineEvent; 10], e: &EngineEvent) -> &'a EngineEvent {
    let [started, text, call, result, usage, raw, message, finished, failed, notice] = samples;
    match e {
        EngineEvent::SessionStarted { .. } => started,
        EngineEvent::AssistantText { .. } => text,
        EngineEvent::ToolCall { .. } => call,
        EngineEvent::ToolResult { .. } => result,
        EngineEvent::Usage { .. } => usage,
        EngineEvent::Raw { .. } => raw,
        EngineEvent::Message { .. } => message,
        EngineEvent::Finished { .. } => finished,
        EngineEvent::Failed { .. } => failed,
        EngineEvent::Notice { .. } => notice,
    }
}

/// One sample of every engine event, as stored in a run event's `data`, in the order
/// `engine_event_sample` names them.
fn engine_events() -> Value {
    let samples = [
        EngineEvent::SessionStarted {
            session_id: "s-1".into(),
        },
        EngineEvent::AssistantText {
            text: "Running the tests.".into(),
        },
        EngineEvent::ToolCall {
            id: "t1".into(),
            tool: "Bash".into(),
            input: json!({"command": "cargo test"}),
        },
        EngineEvent::ToolResult {
            id: "t1".into(),
            output: "test result: ok".into(),
            is_error: false,
        },
        EngineEvent::Usage {
            tokens_in: 1200,
            tokens_out: 340,
        },
        EngineEvent::Raw {
            line: "{\"type\":\"future\"}".into(),
        },
        EngineEvent::Message {
            message: json!({"role": "user", "content": "Run the tests."}),
        },
        EngineEvent::Finished {
            summary: "All tests pass.".into(),
        },
        EngineEvent::Failed {
            error: "timeout".into(),
        },
        EngineEvent::Notice {
            text: "skill pdf: folder not found".into(),
        },
    ];
    for sample in &samples {
        assert!(
            std::ptr::eq(engine_event_sample(&samples, sample), sample),
            "the `{}` sample is not at its variant's place in `engine_events`",
            sample.kind()
        );
    }
    to(samples)
}

/// One schedule of every rule kind, with every kind of bound. The `every` without an anchor is
/// what the UI sends; the core fills the anchor when the dot is saved. Read through the core's
/// type, so a sample in a form the core does not accept fails here.
fn schedules() -> Value {
    let samples = [
        json!({"rule": {"kind": "once", "at": "2026-10-20T09:00"}, "catch_up": true, "until": null}),
        json!({
            "rule": {"kind": "every", "minutes": 90, "anchor": "2026-10-09T12:00:00Z"},
            "catch_up": true,
            "until": {"count": 10}
        }),
        json!({"rule": {"kind": "every", "minutes": 30}, "catch_up": false, "until": null}),
        json!({
            "rule": {"kind": "daily", "time": "09:00"},
            "catch_up": true,
            "until": {"date": "2026-12-31T18:00"}
        }),
        json!({
            "rule": {"kind": "weekly", "days": ["mon", "fri"], "time": "07:30"},
            "catch_up": true,
            "until": null
        }),
        to(Schedule::cron("0 0 2 * * *")),
    ];
    let read: Vec<Schedule> = samples
        .into_iter()
        .map(|v| serde_json::from_value(v).expect("sample schedule"))
        .collect();
    to(read)
}

/// The samples, keyed by the UI schema that parses them. `Enums` lists every variant of every
/// shared enum and `Limits` the validation limits the editor mirrors, with the attempts of a
/// scheduled occurrence that the Dots list and the run view name: the UI parses live data
/// with its schemas, so a variant or limit it does not know must fail a test, not a screen.
/// `NewDotSpec` is the answer of the `new_dot_spec` command, the blank dot of the UI's tests,
/// and `Commands` names every registered command.
pub fn samples() -> Value {
    let settings = Settings {
        local_endpoints: vec!["http://127.0.0.1:1234".into()],
        global_instructions: "Use British spelling.".into(),
        ..Settings::default()
    };
    json!({
        "DotSpec": to(local_spec()),
        "Schedule": schedules(),
        "Dot": to(dot()),
        "Run": to(run()),
        "RunEventRecord": to(event()),
        "Approval": to(approval()),
        "RuntimeEvent": [
            // A retry, queued: it names the run it retries.
            to(RuntimeEvent::RunUpdated { run: Run {
                id: "01J0RUN2".into(),
                root_run_id: "01J0RUN2".into(),
                payload: Some(json!({"scheduled_for": "2026-10-09T02:00:00+01:00", "attempt": 2})),
                status: RunStatus::Queued,
                session_id: None,
                workspace_path: None,
                branch: None,
                base_commit: None,
                summary: None,
                tokens_in: 0,
                tokens_out: 0,
                started_at: None,
                ended_at: None,
                retry_of: Some("01J0RUN".into()),
                ..run()
            }}),
            to(RuntimeEvent::RunEvent { event: event() }),
            to(RuntimeEvent::ApprovalRequested { approval: approval() }),
            to(RuntimeEvent::ApprovalDecided { approval: Approval {
                status: ApprovalStatus::Denied,
                note: Some("not on main".into()),
                parked: true,
                decided_at: Some(TS.into()),
                ..approval()
            }}),
            to(RuntimeEvent::ApprovalUpdated { approval: Approval {
                status: ApprovalStatus::Approved,
                parked: true,
                resolved: true,
                decided_at: Some(TS.into()),
                self_apply: true,
                apply_state: Some(ApplyState::Applied),
                applied_at: Some(TS.into()),
                ..approval()
            }}),
            to(RuntimeEvent::ScheduleUpdated { dot_id: "01J0DOT".into() }),
            to(RuntimeEvent::LibraryUpdated { item_id: "01J0SRV".into() }),
            to(RuntimeEvent::ScheduleAttention {
                dot_id: "01J0DOT".into(),
                run_id: "01J0RUN".into(),
                error: Some("model server unreachable".into()),
            }),
        ],
        // What `approval_rule_candidates` answers for the `Approval` sample.
        "RuleCandidate": to(allow_candidates(
            &approval().tool,
            &approval().input,
            Some(std::path::Path::new("C:/Users/u/AppData/Local/dots/worktrees/01J0RUN")),
        )),
        "Status": to(status()),
        "Settings": to(&settings),
        "SettingsSaved": to(SettingsSaved { settings: settings.clone(), restart_required: true }),
        // Every field of the schedule's view is set, so the UI's schema checks each one.
        "DotOverview": to(DotOverview {
            dot: dot(),
            active_run: None,
            last_run: Some(run()),
            schedule_state: Some(ScheduleView {
                status: ScheduleStatus::Active,
                fired_count: 3,
                next_fire: Some("2026-10-10T02:00:00+01:00".into()),
                retry_at: Some("2026-10-09T13:02:00+01:00".into()),
                retry_attempt: Some(2),
                attention: Some(AttentionRun {
                    run_id: "01J0RUN0".into(),
                    error: Some("model server unreachable".into()),
                }),
            }),
        }),
        "Inbox": to(Inbox {
            approvals: vec![PendingApproval {
                approval: approval(),
                dot_id: "01J0DOT".into(),
                dot_name: "nightly-tests".into(),
            }],
            review_runs: vec![ReviewRun {
                run: run(),
                dot_name: "nightly-tests".into(),
                worktree_exists: true,
            }],
            attention: vec![NeedsAttention {
                dot_id: "01J0DOT".into(),
                dot_name: "nightly-tests".into(),
                run_id: "01J0RUN".into(),
                error: Some("model server unreachable".into()),
                failed_at: TS.into(),
            }],
            tools_changed: vec![ToolsChanged {
                server_id: "01J0SRV".into(),
                name: "github".into(),
                dots: vec![DotRef {
                    id: "01J0DOT".into(),
                    name: "nightly-tests".into(),
                }],
            }],
        }),
        // What `library_add` takes and the Library commands answer: a config of every kind and
        // transport, and entries of every kind (a plugin's own server among them).
        "LibraryConfig": to(library_configs()),
        "McpServerConfig": to([stdio_server(), http_server()]),
        "LibraryItem": to([
            library_item("01J0SRV", "github", LibraryConfig::McpServer(stdio_server())),
            library_item("01J0SKL", "pdf", skill_config()),
        ]),
        "LibraryEntry": to(library_entries()),
        "McpCatalog": to(catalog(CatalogStatus::Changed)),
        "McpProbe": to(McpProbe { tools: vec![tool()], fingerprint: "fp-seen".into() }),
        "ScannedSkill": to([
            ScannedSkill {
                name: "pdf".into(),
                description: "Fill and read PDF forms.".into(),
                path: "C:/skills/pdf".into(),
                name_error: None,
            },
            ScannedSkill {
                name: "bad name".into(),
                description: String::new(),
                path: "C:/skills/bad".into(),
                name_error: Some("skill name \"bad name\": use A-Z, a-z, 0-9".into()),
            },
        ]),
        "InstalledPlugin": to([InstalledPlugin {
            id: "pdf-tools@market".into(),
            name: "pdf-tools".into(),
            marketplace: Some("market".into()),
            scope: Some("user".into()),
            install_path: "C:/Users/u/.claude/plugins/cache/market/pdf-tools/1.2.0".into(),
            version: Some("1.2.0".into()),
            exists: true,
        }]),
        "EnvVariable": to([
            EnvVariable { name: "GITHUB_TOKEN".into(), set: false },
            EnvVariable { name: "PATH".into(), set: true },
        ]),
        "WorktreeDiff": to(WorktreeDiff {
            files: vec![
                FileDiff {
                    path: "src/lib.rs".into(),
                    status: FileStatus::Modified,
                    patch: "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n".into(),
                    binary: false,
                },
                FileDiff {
                    path: "logo.png".into(),
                    status: FileStatus::Added,
                    patch: String::new(),
                    binary: true,
                },
            ],
            truncated: false,
            exists: true,
        }),
        "CmdError": to(CmdError { kind: CmdErrorKind::Conflict, message: "run 01J0RUN is already succeeded".into() }),
        "RunEventData": engine_events(),
        "NewDotSpec": to(crate::commands::new_dot_spec()),
        "Commands": crate::COMMANDS,
        "Enums": {
            "EngineKind": EngineKind::ALL,
            "WorkspaceMode": WorkspaceMode::ALL,
            "TriggerKind": TriggerKind::ALL,
            "RunStatus": RunStatus::ALL,
            "ApprovalStatus": ApprovalStatus::ALL,
            "ApplyState": ApplyState::ALL,
            "Action": Action::ALL,
            "Preset": Preset::ALL,
            "FileStatus": FileStatus::ALL,
            "ScheduleStatus": ScheduleStatus::ALL,
            "CmdErrorKind": CmdErrorKind::ALL,
            "LibraryKind": LibraryKind::ALL,
            "CatalogStatus": CatalogStatus::ALL,
        },
        "Limits": {
            "max_timeout_secs": MAX_TIMEOUT_SECS,
            "max_approval_wait_secs": MAX_APPROVAL_WAIT_SECS,
            "max_concurrent_runs": MAX_CONCURRENT_RUNS,
            "max_schedule_attempts": MAX_ATTEMPTS,
            "max_global_instructions": MAX_GLOBAL_INSTRUCTIONS,
        },
        "StartupError": to(StartupError {
            message: "Dots could not start: database disk image is malformed".into(),
            log_dir: "C:/Users/u/AppData/Local/dots/logs".into(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/src/api/__fixtures__/contract.json")
    }

    #[test]
    fn contract_fixture_is_up_to_date() {
        let text = serde_json::to_string_pretty(&super::samples()).unwrap() + "\n";
        if std::env::var_os("UPDATE_CONTRACT").is_some() {
            std::fs::create_dir_all(fixture().parent().unwrap()).unwrap();
            std::fs::write(fixture(), &text).unwrap();
        }
        let on_disk = std::fs::read_to_string(fixture())
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            on_disk == text,
            "ui/src/api/__fixtures__/contract.json is stale; run \
             `UPDATE_CONTRACT=1 cargo test -p dots-app contract` and fix ui/src/api/types.ts"
        );
    }
}

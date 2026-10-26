//! One sample of every type the UI receives or sends, written to
//! `ui/src/api/__fixtures__/contract.json`. The UI's contract test parses each sample with its
//! schema, so a field renamed or added on one side fails the other side's tests.
//! Regenerate after changing a type: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.

use dots_core::browser::BrowserStatus;
use dots_core::context::Document;
use dots_core::engine::EngineEvent;
use dots_core::events::RuntimeEvent;
use dots_core::kit::InstalledPlugin;
use dots_core::library::{
    CatalogStatus, ContextConfig, LibraryConfig, LibraryItem, LibraryKind, McpCatalog,
    McpServerConfig, PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::{
    ContextDetail, ContextDocuments, EnvVariable, LibraryEntry, McpProbe, PluginDetail,
    ScannedSkill, ScopedCatalog, SkillDetail,
};
use dots_core::mcp::McpTool;
use dots_core::model::*;
use dots_core::policy::{allow_candidates, Action, Policy, Preset};
use dots_core::schedule::Schedule;
use dots_core::scheduler::MAX_ATTEMPTS;
use dots_core::settings::{Settings, MAX_CONCURRENT_RUNS, MAX_GLOBAL_INSTRUCTIONS};
use dots_core::skill_creator::{DraftRequest, DraftSummary};
use dots_core::vault::org::{
    BaoAuth, JoinPreview, OrgEntry, OrgStatus, OrgSummary, PersonalMode, SourceCreds, UseRules,
};
use dots_core::vault::{
    Consumer, EntryKind, EnvEntry, GrantState, GrantStatus, Missing, Unresolved, ValueSource,
};
use serde_json::{json, Value};

use crate::error::{CmdError, CmdErrorKind};
use crate::state::StartupError;
use crate::update::UpdateNote;

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
        context: vec!["company".into()],
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
        has_webhook_token: true,
        created_at: TS.into(),
        updated_at: TS.into(),
        seen_at: Some(TS.into()),
        org_id: None,
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

fn context_config() -> LibraryConfig {
    LibraryConfig::Context(ContextConfig {
        path: "C:/notes/company".into(),
        description: "How we work: style, people, runbooks".into(),
        every_dot: true,
    })
}

/// A config of every kind and transport, as `library_add` takes them.
fn library_configs() -> Vec<LibraryConfig> {
    vec![
        LibraryConfig::McpServer(stdio_server()),
        LibraryConfig::McpServer(http_server()),
        skill_config(),
        plugin_config(),
        context_config(),
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
        catalogs: vec![],
        confirmed_origins: Default::default(),
        missing_variables: vec![],
        skill: None,
        plugin: None,
        context: None,
        problem: None,
        dots_folder: false,
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
    let company = library_item("01J0CTX", "company", context_config());
    vec![
        LibraryEntry {
            catalog: Some(catalog(CatalogStatus::Changed)),
            // Its catalog in Personal and in the org joined.
            catalogs: vec![
                ScopedCatalog {
                    scope: "personal".into(),
                    catalog: catalog(CatalogStatus::Changed),
                    stored: true,
                    used_by: users.clone(),
                },
                // Checked in the org, and used by none of its dots.
                ScopedCatalog {
                    scope: "acme-eng".into(),
                    catalog: catalog(CatalogStatus::Accepted),
                    stored: true,
                    used_by: vec![],
                },
            ],
            missing_variables: vec!["GITHUB_TOKEN".into()],
            ..bare(github, users.clone())
        },
        LibraryEntry {
            catalog: Some(catalog(CatalogStatus::Accepted)),
            catalogs: vec![ScopedCatalog {
                scope: "personal".into(),
                catalog: catalog(CatalogStatus::Accepted),
                stored: true,
                used_by: users.clone(),
            }],
            // The origins confirmed for an http server, by scope.
            confirmed_origins: [
                (
                    "acme-eng".to_string(),
                    vec!["https://mcp.acme.example".to_string()],
                ),
                (
                    "personal".to_string(),
                    vec!["https://mcp.example.com".to_string()],
                ),
            ]
            .into(),
            ..bare(owned, users.clone())
        },
        LibraryEntry {
            skill: Some(SkillDetail {
                description: "Fill and read PDF forms.".into(),
            }),
            problem: Some("SKILL.md not found".into()),
            dots_folder: true,
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
            ..bare(plugin, users.clone())
        },
        LibraryEntry {
            context: Some(ContextDetail {
                documents: 42,
                complete: true,
            }),
            ..bare(company, users)
        },
    ]
}

/// A collection's first documents, more of them left out.
fn context_documents() -> ContextDocuments {
    ContextDocuments {
        documents: vec![Document {
            path: "people/ana.md".into(),
            title: "Ana Lima".into(),
            summary: "Owns billing".into(),
        }],
        total: 42,
        truncated: true,
        complete: true,
    }
}

/// A question that waits, an answered one, a dismissed one, and a note of Dots' own that was
/// read: every optional field of a message is set in one of them.
fn messages() -> Vec<Message> {
    let open = Message {
        id: "01J0MSG1".into(),
        dot_id: "01J0DOT".into(),
        dot_name: "nightly-tests".into(),
        run_id: Some("01J0RUN".into()),
        kind: MessageKind::Question,
        text: "Ship the release?".into(),
        choices: Some(vec!["yes".into(), "no".into()]),
        created_at: TS.into(),
        read_at: None,
        answered_at: None,
        answer_run_id: None,
        dismissed_at: None,
    };
    vec![
        open.clone(),
        Message {
            id: "01J0MSG2".into(),
            read_at: Some(TS.into()),
            answered_at: Some(TS.into()),
            answer_run_id: Some("01J0RUN3".into()),
            ..open.clone()
        },
        Message {
            id: "01J0MSG3".into(),
            choices: None,
            dismissed_at: Some(TS.into()),
            ..open.clone()
        },
        Message {
            id: "01J0MSG4".into(),
            run_id: None,
            kind: MessageKind::Note,
            text: "A follow-up was skipped because the dot is off. It was due Tue 21 Oct 09:00: \
                   check CI"
                .into(),
            choices: None,
            read_at: Some(TS.into()),
            ..open
        },
    ]
}

/// A pinned memory the dot saved in a run, and one the person wrote.
fn memories() -> Vec<Memory> {
    let dots = Memory {
        id: "01J0MEM1".into(),
        dot_id: "01J0DOT".into(),
        text: "Prefers invoices as PDF.".into(),
        pinned: true,
        source: MemorySource::Dot,
        run_id: Some("01J0RUN".into()),
        created_at: TS.into(),
        updated_at: TS.into(),
    };
    vec![
        dots.clone(),
        Memory {
            id: "01J0MEM2".into(),
            text: "The staging server is staging.example.com.".into(),
            pinned: false,
            source: MemorySource::Person,
            run_id: None,
            ..dots
        },
    ]
}

/// A folder watcher that is on, an email one Dots paused on an error, a GitHub one it paused on
/// a flood (watchers spec sections 3 and 4).
fn watchers() -> Vec<Watcher> {
    let folder = Watcher {
        id: "01J0WAT1".into(),
        dot_id: "01J0DOT".into(),
        name: "Invoices folder".into(),
        config: WatcherConfig::Folder(FolderConfig {
            path: r"C:\Users\u\Downloads\Invoices".into(),
            pattern: "*.pdf".into(),
            events: FolderEvent::ALL.to_vec(),
            recursive: true,
        }),
        enabled: true,
        paused_reason: None,
        last_error: None,
        last_event_at: Some(TS.into()),
        created_at: TS.into(),
        updated_at: TS.into(),
    };
    vec![
        folder.clone(),
        Watcher {
            id: "01J0WAT2".into(),
            name: "Billing mail".into(),
            config: WatcherConfig::Email(EmailConfig {
                address: "me@gmail.com".into(),
                secret: "GMAIL_APP_PASSWORD".into(),
                label: "Dots".into(),
                from: Some("billing@acme.com".into()),
                subject: Some("invoice".into()),
                has_attachment: None,
            }),
            paused_reason: Some(PausedReason::Error),
            last_error: Some(
                "Gmail refused the app password for me@gmail.com, or IMAP is off for it.".into(),
            ),
            last_event_at: None,
            ..folder.clone()
        },
        Watcher {
            id: "01J0WAT3".into(),
            name: "Repo events".into(),
            config: WatcherConfig::Github(GithubConfig {
                repos: vec!["owner/repo".into()],
                events: vec![GithubEvent::Issue, GithubEvent::PullRequest],
                secret: "GITHUB_TOKEN".into(),
                label: None,
                authors: Vec::new(),
                skip_bots: true,
                branches: Vec::new(),
            }),
            paused_reason: Some(PausedReason::Flood),
            ..folder
        },
    ]
}

/// A pending follow-up that failed to start once, and one that fired.
fn follow_ups() -> Vec<FollowUp> {
    let pending = FollowUp {
        id: "01J0FUP1".into(),
        dot_id: "01J0DOT".into(),
        from_run_id: "01J0RUN".into(),
        due_at: "2026-10-21T08:00:00.000Z".into(),
        note: "Check the build.".into(),
        status: FollowUpStatus::Pending,
        fired_run_id: None,
        attempts: 1,
        retry_at: Some("2026-10-21T08:01:00.000Z".into()),
        created_at: TS.into(),
    };
    vec![
        pending.clone(),
        FollowUp {
            id: "01J0FUP2".into(),
            status: FollowUpStatus::Fired,
            fired_run_id: Some("01J0RUN4".into()),
            attempts: 0,
            retry_at: None,
            ..pending
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
        cost_usd: None,
        hold_released: false,
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
fn engine_event_sample<'a>(samples: &'a [EngineEvent; 11], e: &EngineEvent) -> &'a EngineEvent {
    let [started, text, call, result, usage, limits, raw, message, finished, failed, notice] =
        samples;
    match e {
        EngineEvent::SessionStarted { .. } => started,
        EngineEvent::AssistantText { .. } => text,
        EngineEvent::ToolCall { .. } => call,
        EngineEvent::ToolResult { .. } => result,
        EngineEvent::Usage { .. } => usage,
        EngineEvent::Limits { .. } => limits,
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
            cost_usd: Some(1.32),
        },
        EngineEvent::Limits {
            status: "allowed".into(),
            windows: vec![dots_core::store::LimitReading {
                window: "five_hour".into(),
                utilization: 0.42,
                resets_at: TS.into(),
                status: "allowed".into(),
                read_at: TS.into(),
            }],
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

/// One note of each state (`UpdateNote`).
fn update_notes() -> Vec<UpdateNote> {
    vec![
        UpdateNote::None,
        UpdateNote::Ready {
            version: "0.2.0".into(),
            working: vec!["Daily digest".into()],
            error: Some("The backup failed: disk full".into()),
        },
        UpdateNote::Waiting {
            version: "0.2.0".into(),
            waiting_for: vec!["Daily digest".into()],
        },
        UpdateNote::Installing {
            version: "0.2.0".into(),
        },
        UpdateNote::Failed {
            error: "The backup failed: disk full".into(),
        },
    ]
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
    let mut samples = json!({
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
            needs_you: 0,
            unread: 0,
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
            tools_changed: tools_changed()[..1].to_vec(),
            watchers: vec![PausedWatcher {
                watcher_id: "01J0WAT2".into(),
                dot_id: "01J0DOT".into(),
                dot_name: "nightly-tests".into(),
                kind: WatcherKind::Email,
                name: "Billing mail".into(),
                reason: PausedReason::Error,
                error: Some(
                    "Gmail refused the app password for me@gmail.com, or IMAP is off for it."
                        .into(),
                ),
                at: TS.into(),
            }],
            held: vec![],
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
            "max_memories": dots_core::memory::MAX_PER_DOT,
            "max_memory_text": dots_core::memory::MAX_TEXT,
            "max_watcher_name": dots_core::watchers::MAX_NAME_CHARS,
        },
        "StartupError": to(StartupError {
            message: "Dots could not start: database disk image is malformed".into(),
            log_dir: "C:/Users/u/AppData/Local/dots/logs".into(),
            backup: "C:/Users/u/AppData/Local/dots/backups/dots-0.1.0-2026-10-20-091700.db".into(),
        }),
    });
    // Past the `json!` macro's recursion limit, so added here.
    samples["ContextDocuments"] = to(context_documents());
    samples["Message"] = to(messages());
    samples["FollowUp"] = to(follow_ups());
    samples["Memory"] = to(memories());
    samples["Watcher"] = to(watchers());
    samples["Enums"]["MessageKind"] = to(MessageKind::ALL);
    samples["Enums"]["FollowUpStatus"] = to(FollowUpStatus::ALL);
    samples["Enums"]["MemorySource"] = to(MemorySource::ALL);
    samples["Enums"]["WatcherKind"] = to(WatcherKind::ALL);
    samples["Enums"]["PausedReason"] = to(PausedReason::ALL);
    samples["Enums"]["FolderEvent"] = to(FolderEvent::ALL);
    samples["Enums"]["GithubEvent"] = to(GithubEvent::ALL);
    samples["RuntimeEvent"]
        .as_array_mut()
        .expect("the RuntimeEvent samples")
        .extend([
            to(RuntimeEvent::MessagePosted {
                message: messages()[0].clone(),
            }),
            to(RuntimeEvent::MessageUpdated {
                message: messages()[1].clone(),
            }),
            to(RuntimeEvent::FollowUpsUpdated {
                dot_id: "01J0DOT".into(),
            }),
            to(RuntimeEvent::DotSeen {
                dot_id: "01J0DOT".into(),
            }),
            to(RuntimeEvent::MemoriesUpdated {
                dot_id: "01J0DOT".into(),
            }),
            to(RuntimeEvent::WatcherUpdated {
                dot_id: "01J0DOT".into(),
                watcher_id: "01J0WAT1".into(),
            }),
            to(RuntimeEvent::WatcherPaused {
                dot_id: "01J0DOT".into(),
                watcher_id: "01J0WAT3".into(),
                name: "Repo events".into(),
                reason: PausedReason::Flood,
                error: None,
            }),
            to(RuntimeEvent::RunUpdated {
                run: Run {
                    id: "01J0RUN3".into(),
                    root_run_id: "01J0RUN3".into(),
                    trigger: TriggerKind::Watcher,
                    payload: Some(json!({
                        "source": "folder",
                        "event": "created",
                        "path": r"C:\Users\u\Downloads\Invoices\invoice-12.pdf",
                        "relative": "invoice-12.pdf",
                        "size": 1200,
                        "watcher_id": "01J0WAT1",
                        "watcher_name": "Invoices folder",
                    })),
                    ..run()
                },
            }),
            to(RuntimeEvent::OrgUpdated {
                org_id: "acme-eng".into(),
            }),
        ]);
    samples["ChatGptStatus"] = to([
        dots_core::chatgpt::ChatGptStatus {
            configured: false,
            signing_in: false,
            account: None,
            expires_at: None,
            setup_error: Some("ChatGPT sign-in needs a registered OpenAI client for Dots.".into()),
        },
        dots_core::chatgpt::ChatGptStatus {
            configured: true,
            signing_in: true,
            account: None,
            expires_at: None,
            setup_error: None,
        },
        dots_core::chatgpt::ChatGptStatus {
            configured: true,
            signing_in: false,
            account: Some(dots_core::chatgpt::ChatGptAccount {
                name: Some("Alex".into()),
                email: Some("alex@example.com".into()),
            }),
            expires_at: Some("2026-10-23T00:00:00Z".into()),
            setup_error: None,
        },
    ]);
    samples["UpdateNote"] = to(update_notes());
    samples["BrowserStatus"] = to(BrowserStatus {
        edge: Some("C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe".into()),
        npx: Some("C:/Program Files/nodejs/npx.cmd".into()),
    });
    samples["DraftRequest"] = to([
        DraftRequest {
            idea: "Check a PR for missing tests".into(),
            model: "sonnet".into(),
            current: None,
            change: None,
        },
        DraftRequest {
            idea: "Check a PR for missing tests".into(),
            model: "sonnet".into(),
            current: Some("---\nname: pr-tests\n---\nList the changed functions.\n".into()),
            change: Some("Mention snapshot tests".into()),
        },
    ]);
    samples["EnvEntry"] = to([
        EnvEntry {
            name: "GITHUB_TOKEN".into(),
            kind: EntryKind::Secret,
            value: None,
            created_at: TS.into(),
            updated_at: TS.into(),
            used_by: vec![DotRef {
                id: "01J0DOT".into(),
                name: "nightly-tests".into(),
            }],
            value_missing: false,
        },
        EnvEntry {
            name: "LOG_LEVEL".into(),
            kind: EntryKind::Plain,
            value: Some("debug".into()),
            created_at: TS.into(),
            updated_at: TS.into(),
            used_by: vec![],
            value_missing: false,
        },
        // A secret whose credential was deleted outside Dots.
        EnvEntry {
            name: "NPM_TOKEN".into(),
            kind: EntryKind::Secret,
            value: None,
            created_at: TS.into(),
            updated_at: TS.into(),
            used_by: vec![],
            value_missing: true,
        },
    ]);
    samples["Enums"]["EntryKind"] = to(EntryKind::ALL);
    samples["DraftSummary"] = to(DraftSummary {
        name: "pr-tests".into(),
        description: "Checks a PR for missing tests.".into(),
    });
    // The Inbox's sample keeps one, Personal's; these are one of each scope.
    samples["ToolsChanged"] = to(tools_changed());
    samples["OrgSummary"] = to(org_summaries());
    samples["JoinPreview"] = to(join_previews());
    samples["OrgEntry"] = to([
        OrgEntry {
            name: "API_BASE_URL".into(),
            kind: EntryKind::Plain,
            value: Some("https://api.acme.example".into()),
            locked: true,
            used_by: vec![DotRef {
                id: "01J0DOTACME".into(),
                name: "acme-release".into(),
            }],
        },
        OrgEntry {
            name: "GITHUB_TOKEN".into(),
            kind: EntryKind::Secret,
            value: None,
            locked: false,
            used_by: vec![],
        },
    ]);
    samples["SourceCreds"] = source_creds();
    samples["GrantStatus"] = to(grant_statuses());
    samples["Enums"]["GrantState"] = to(GrantState::ALL);
    samples["Enums"]["ValueSource"] = to(ValueSource::ALL);
    samples["Enums"]["PersonalMode"] = to(PersonalMode::ALL);
    samples
}

/// What `dot_variables` answers for acme-release, a dot of Acme Engineering (`personal:
/// fallback`, `shell_secrets: false`): a secret from the org; a plain value from Personal; a
/// name the org lacks; a name its `use` rules deny; a secret its shell may not have. Then a
/// grant of a dot whose org was removed. The texts are the core's own.
fn grant_statuses() -> Vec<GrantStatus> {
    let text = |name: &str, consumer: Consumer, why: Unresolved, org: &str| {
        Missing {
            name: name.into(),
            consumer,
            why,
            org: Some(org.into()),
        }
        .text("acme-release")
    };
    let acme = "Acme Engineering";
    let github = || Consumer::Mcp("github".into());
    let status = |consumer: &Consumer, name: &str, status: GrantState| GrantStatus {
        consumer: consumer.key(),
        name: name.into(),
        status,
        text: None,
        source: None,
        secret: false,
    };
    vec![
        GrantStatus {
            source: Some(ValueSource::Org),
            secret: true,
            ..status(&github(), "GITHUB_TOKEN", GrantState::Ok)
        },
        GrantStatus {
            source: Some(ValueSource::Personal),
            ..status(&Consumer::Shell, "LOG_LEVEL", GrantState::Ok)
        },
        GrantStatus {
            text: Some(text("API_KEY", github(), Unresolved::NotDefined, acme)),
            ..status(&github(), "API_KEY", GrantState::NotDefined)
        },
        GrantStatus {
            text: Some(text(
                "PROD_DB_URL",
                github(),
                Unresolved::DeniedByPolicy,
                acme,
            )),
            ..status(&github(), "PROD_DB_URL", GrantState::DeniedByPolicy)
        },
        GrantStatus {
            text: Some(text(
                "GITHUB_TOKEN",
                Consumer::Shell,
                Unresolved::ShellSecretsForbidden,
                acme,
            )),
            source: Some(ValueSource::Org),
            secret: true,
            ..status(
                &Consumer::Shell,
                "GITHUB_TOKEN",
                GrantState::ShellSecretsForbidden,
            )
        },
        GrantStatus {
            text: Some(text(
                "LOG_LEVEL",
                Consumer::Shell,
                Unresolved::OrgRemoved,
                "acme-eng",
            )),
            ..status(&Consumer::Shell, "LOG_LEVEL", GrantState::OrgRemoved)
        },
    ]
}

/// A changed tool list in Personal, with the dot that uses it, and the same server's in an
/// org, used by no dot of that org.
fn tools_changed() -> Vec<ToolsChanged> {
    let personal = ToolsChanged {
        server_id: "01J0SRV".into(),
        name: "github".into(),
        scope: "personal".into(),
        dots: vec![DotRef {
            id: "01J0DOT".into(),
            name: "nightly-tests".into(),
        }],
    };
    vec![
        personal.clone(),
        ToolsChanged {
            scope: "acme-eng".into(),
            dots: vec![],
            ..personal
        },
    ]
}

/// An org of every status (`org_list`, and what `org_join`, `org_update` and `org_refresh`
/// answer): every optional field is set in one of them.
fn org_summaries() -> Vec<OrgSummary> {
    let connected = OrgSummary {
        id: "acme-eng".into(),
        name: "Acme Engineering".into(),
        source_kind: "openbao".into(),
        address: "https://bao.acme.internal".into(),
        key_fingerprint: "6E2F 0A1B 9C3D 4E5F 7A81 B2C3 D4E5 F607".into(),
        status: OrgStatus::Connected,
        status_text: "connected".into(),
        last_fetch_at: Some(TS.into()),
        copy_until: Some("2026-10-10T12:00:00.000Z".into()),
        policy_summary: vec![
            "org: acme-eng".into(),
            "personal: fallback (a personal entry is used when the org has none)".into(),
        ],
        personal_mode: Some(PersonalMode::Fallback),
        shell_secrets: false,
        use_rules: Some(UseRules {
            allow: vec!["*".into()],
            deny: vec!["PROD_*".into()],
        }),
    };
    let status = |status: OrgStatus, text: &str| OrgSummary {
        status,
        status_text: text.into(),
        ..connected.clone()
    };
    vec![
        connected.clone(),
        status(
            OrgStatus::Offline {
                until: Some("2026-10-10T12:00:00.000Z".into()),
            },
            "offline (connection refused); using the saved copy until 2026-10-10T12:00:00.000Z",
        ),
        OrgSummary {
            copy_until: None,
            ..status(
                OrgStatus::Offline { until: None },
                "offline, with no saved copy",
            )
        },
        OrgSummary {
            copy_until: None,
            ..status(OrgStatus::Revoked, "access revoked (permission denied)")
        },
        status(
            OrgStatus::PolicyInvalid {
                error: "signature does not match".into(),
            },
            "policy invalid: signature does not match; using the saved copy until \
             2026-10-10T12:00:00.000Z",
        ),
        OrgSummary {
            copy_until: None,
            ..status(
                OrgStatus::Expired,
                "secrets expired offline; reconnect to refresh",
            )
        },
        OrgSummary {
            id: "design".into(),
            name: "Design".into(),
            source_kind: "sops".into(),
            address: "C:/Users/u/Sync/acme/design.enc.json".into(),
            last_fetch_at: None,
            copy_until: None,
            policy_summary: vec![],
            personal_mode: None,
            use_rules: None,
            ..status(
                OrgStatus::Pending,
                "waiting for the admin to add this PC's key",
            )
        },
    ]
}

/// A code for a new org, and one that changes a joined org's path and key (`org_preview`).
fn join_previews() -> Vec<JoinPreview> {
    let new = JoinPreview {
        org_id: "acme-eng".into(),
        name: "Acme Engineering".into(),
        source_kind: "openbao".into(),
        address: "https://bao.acme.internal".into(),
        key_fingerprint: "6E2F 0A1B 9C3D 4E5F 7A81 B2C3 D4E5 F607".into(),
        auth: Some(BaoAuth::Token),
        existing: false,
        changes: vec![],
        key_changed: false,
        source_changed: false,
    };
    vec![
        new.clone(),
        JoinPreview {
            existing: true,
            key_fingerprint: "0A1B 2C3D 4E5F 6071 8293 A4B5 C6D7 E8F9".into(),
            changes: vec![
                "path changes from dots to dots2".into(),
                "the policy key changes from 6E2F 0A1B 9C3D 4E5F 7A81 B2C3 D4E5 F607 to 0A1B \
                 2C3D 4E5F 6071 8293 A4B5 C6D7 E8F9: the key that signs the org's policy is new, \
                 so accept this code only if your admin told you the key changed"
                    .into(),
            ],
            key_changed: true,
            source_changed: true,
            ..new
        },
    ]
}

/// Sign-in details of every kind, as `org_join` and `org_update` take them. Read through the
/// core's type, so a sample in a form the core does not accept fails here. No value is real.
fn source_creds() -> Value {
    let samples = [
        json!({"kind": "token", "token": "hvs.sample-token"}),
        json!({"kind": "login", "username": "jdoe", "password": "sample-password"}),
        json!({"kind": "machine", "client_id": "sample-id", "client_secret": "sample-secret"}),
        json!({"kind": "age", "identity": "AGE-SECRET-KEY-1SAMPLE"}),
    ];
    let read: Vec<SourceCreds> = samples
        .into_iter()
        .map(|v| serde_json::from_value(v).expect("sample sign-in details"))
        .collect();
    to(read)
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

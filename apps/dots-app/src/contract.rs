//! One sample of every type the UI receives or sends, written to
//! `ui/src/api/__fixtures__/contract.json`. The UI's contract test parses each sample with its
//! schema, so a field renamed or added on one side fails the other side's tests.
//! Regenerate after changing a type: `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.

use dots_core::events::RuntimeEvent;
use dots_core::model::*;
use dots_core::policy::{Policy, Preset};
use dots_core::settings::Settings;
use serde_json::{json, Value};

use crate::error::CmdError;
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
    s.schedule = Some("0 0 2 * * *".into());
    s.mcp_servers = Some(json!({"github": {"type": "http", "url": "http://127.0.0.1:9000/mcp"}}));
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

/// The samples, keyed by the UI schema that parses them.
pub fn samples() -> Value {
    let settings = Settings {
        local_endpoints: vec!["http://127.0.0.1:1234".into()],
        ..Settings::default()
    };
    json!({
        "DotSpec": to(local_spec()),
        "Dot": to(dot()),
        "Run": to(run()),
        "RunEventRecord": to(event()),
        "Approval": to(approval()),
        "RuntimeEvent": [
            to(RuntimeEvent::RunUpdated { run: run() }),
            to(RuntimeEvent::RunEvent { event: event() }),
            to(RuntimeEvent::ApprovalRequested { approval: approval() }),
            to(RuntimeEvent::ApprovalDecided { approval: Approval {
                status: ApprovalStatus::Denied,
                note: Some("not on main".into()),
                parked: true,
                decided_at: Some(TS.into()),
                ..approval()
            }}),
        ],
        "Status": to(status()),
        "Settings": to(&settings),
        "SettingsSaved": to(SettingsSaved { settings: settings.clone(), restart_required: true }),
        "DotOverview": to(DotOverview {
            dot: dot(),
            active_run: None,
            last_run: Some(run()),
            next_fire: Some("2026-10-10T02:00:00+01:00".into()),
        }),
        "Inbox": to(Inbox {
            approvals: vec![PendingApproval {
                approval: approval(),
                dot_id: "01J0DOT".into(),
                dot_name: "nightly-tests".into(),
            }],
            review_runs: vec![ReviewRun { run: run(), dot_name: "nightly-tests".into() }],
        }),
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
        }),
        "CmdError": to(CmdError { kind: "conflict".into(), message: "run 01J0RUN is already succeeded".into() }),
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

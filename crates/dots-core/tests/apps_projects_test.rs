//! Projects in a running Dots (app library spec sections 8 and 12): Dots installs it at start;
//! the person gives a task to a local dot with "Can start dots" on; the dot reads the task,
//! checks a subtask, comments and sets it In review; Activity shows its start, with the run, and
//! its summary.

mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::apps::{expenses, write_app};
use common::{spec, wait_status};
use dots_core::apps::bundled::PROJECTS;
use dots_core::apps::call::Actor;
use dots_core::apps::source::read_folder;
use dots_core::library::AppOrigin;
use dots_core::model::{EngineKind, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use llm::{call as llm_call, messages, text as llm_text, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;

async fn start(bundled: bool) -> (TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    cfg.bundled_apps = bundled;
    let rt = Runtime::start(cfg).await.unwrap();
    (dir, rt)
}

async fn person(rt: &Runtime, tool: &str, input: Value) -> Value {
    rt.apps()
        .call(PROJECTS, tool, input, Actor::Person)
        .await
        .unwrap()
}

#[tokio::test]
async fn dots_installs_projects_at_start_and_a_test_runtime_asks_for_it() {
    let (_dir, rt) = start(false).await;
    assert!(matches!(
        rt.apps().get(PROJECTS).await,
        Err(Error::NotFound(_))
    ));
    rt.shutdown();
    let (_dir, rt) = start(true).await;
    let (_, config) = rt.apps().get(PROJECTS).await.unwrap();
    assert!(config.every_dot);
    rt.shutdown();
}

#[tokio::test]
async fn a_dot_given_a_task_reads_it_reports_and_leaves_it_in_review() {
    let (dir, rt) = start(true).await;
    let task = person(
        &rt,
        "create_task",
        json!({ "title": "Signup form", "spec": "Email and password" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let sub = person(
        &rt,
        "add_subtask",
        json!({ "task_id": task, "title": "Form markup" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let llm = Llm::start(vec![
        llm_call("c1", "projects__get_task", json!({ "id": task })),
        llm_call(
            "c2",
            "projects__set_subtask",
            json!({ "id": sub, "done": true }),
        ),
        llm_call(
            "c3",
            "projects__comment",
            json!({ "task_id": task, "text": "Markup done." }),
        ),
        llm_call(
            "c4",
            "projects__update_task",
            json!({ "id": task, "status": "in_review" }),
        ),
        llm_text("Built the form."),
    ])
    .await;
    let mut s = spec("builder");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{}/v1", llm.endpoint()));
    s.model = "test-model".into();
    let builder = rt.create_dot(s, None, None).await.unwrap();
    rt.apps().set_can_start_dots(PROJECTS, true).await.unwrap();

    person(
        &rt,
        "update_task",
        json!({ "id": task, "assignee": "builder" }),
    )
    .await;
    let got = person(&rt, "get_task", json!({ "id": task })).await;
    assert_eq!(got["status"], "in_progress");
    let started = got["activity"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["field"] == "started")
        .cloned()
        .unwrap();
    let run_id = started["run_id"].as_str().unwrap().to_string();
    let run = rt.store().get_run(&run_id).await.unwrap();
    assert_eq!(
        (run.trigger, run.dot_id.as_str()),
        (TriggerKind::App, builder.id.as_str())
    );
    wait_status(rt.store(), &run_id, RunStatus::Succeeded, 30).await;
    let prompt = messages(&llm.requests()[0])
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        prompt.contains("You are assigned the Projects task \"Signup form\""),
        "{prompt}"
    );

    // The app hears the run's end once nothing in its topic waits.
    let mut rows = Vec::new();
    for _ in 0..200 {
        let got = person(&rt, "get_task", json!({ "id": task, "activity": 100 })).await;
        rows = got["activity"].as_array().unwrap().clone();
        if rows.iter().any(|r| r["field"] == "ended") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let seen: Vec<(String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r["kind"].as_str().unwrap().to_string(),
                r["field"].as_str().unwrap_or_default().to_string(),
                r["actor"]["name"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let sub_field = format!("subtask:{sub}");
    assert_eq!(
        seen.iter()
            .map(|(k, f, a)| (k.as_str(), f.as_str(), a.as_str()))
            .collect::<Vec<_>>(),
        [
            ("change", "created", "You"),
            ("change", sub_field.as_str(), "You"),
            ("change", "assignee", "You"),
            ("change", "status", "You"),
            ("run", "started", "builder"),
            ("change", sub_field.as_str(), "builder"),
            ("comment", "", "builder"),
            ("change", "status", "builder"),
            ("run", "ended", "builder"),
        ]
    );
    let ended = rows.last().unwrap();
    assert_eq!(ended["text"], "Built the form.");
    assert_eq!(ended["run_id"], run_id.as_str());
    let got = person(&rt, "get_task", json!({ "id": task })).await;
    assert_eq!(got["status"], "in_review");
    assert_eq!(got["subtasks"][0]["done"], true);

    // The screen opens the run it started, and no other.
    let link = rt.apps().run_link(PROJECTS, &run_id).await.unwrap();
    assert_eq!(
        (link.dot_id.as_str(), link.root_run_id.as_str()),
        (builder.id.as_str(), run_id.as_str())
    );
    let other = rt.run_now(&builder.id).await.unwrap();
    // Another installed app does not open Projects' run: the topic must be its own.
    let src = tempfile::tempdir().unwrap();
    write_app(src.path(), &expenses());
    rt.apps()
        .install(read_folder(src.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    for (app, run) in [
        (PROJECTS, other.id.as_str()),
        (PROJECTS, "nope"),
        ("expenses", run_id.as_str()),
        ("nothing", run_id.as_str()),
    ] {
        let refused = rt.apps().run_link(app, run).await;
        assert!(
            matches!(refused, Err(Error::NotFound(_))),
            "{app} {run}: {refused:?}"
        );
    }
    rt.shutdown();
}

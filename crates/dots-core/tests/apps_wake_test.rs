//! Apps waking dots (app library spec section 7): with "Can start dots" on, `ctx.dots.start`
//! queues a run of a dot that uses the app as a topic of its own, only after the call commits;
//! one open topic per (dot, tag), 20 starts an hour, none while schedules are paused; and the
//! app's `on_run_end` hears a run's end once nothing in its topic waits.

mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{apps::write_app, spec, wait_status};
use dots_core::apps::call::{Actor, STARTS_PAUSED};
use dots_core::apps::source::read_folder;
use dots_core::library::AppOrigin;
use dots_core::model::{Dot, DotSpec, EngineKind, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Runtime};
use llm::{call as llm_call, messages, text as llm_text, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;

const MANIFEST: &str = r#"{
  "name": "chores",
  "title": "Chores",
  "description": "Chores dots do",
  "tools": [
    {
      "name": "assign",
      "description": "Give a chore to a dot",
      "input": { "type": "object", "properties": { "dot": { "type": "string" }, "chore": { "type": "string" } }, "required": ["dot", "chore"] },
      "writes": true
    },
    {
      "name": "assign_many",
      "description": "Give n chores to a dot",
      "input": { "type": "object", "properties": { "dot": { "type": "string" }, "n": { "type": "integer" } }, "required": ["dot", "n"] },
      "writes": true
    },
    {
      "name": "ends",
      "description": "The ends the app heard",
      "input": { "type": "object" }
    }
  ],
  "permissions": ["start_dots"],
  "on_run_end": "run_ended"
}"#;

const LOGIC: &str = r#"
export function assign({ dot, chore }, ctx) {
  ctx.db.exec("INSERT INTO assigned (chore) VALUES (?)", [chore]);
  return ctx.dots.start(dot, `Do the chore: ${chore}`, chore);
}
export function assign_many({ dot, n }, ctx) {
  const ids = [];
  for (let i = 0; i < n; i++) ids.push(ctx.dots.start(dot, `Chore ${i}`, `c${i}`));
  return ids;
}
export function ends(_, ctx) {
  return ctx.db.query("SELECT run_id, root_run_id, tag, status, summary FROM ends ORDER BY rowid");
}
export function run_ended(e, ctx) {
  ctx.db.exec("INSERT INTO ends (run_id, root_run_id, tag, status, summary) VALUES (?, ?, ?, ?, ?)",
    [e.run_id, e.root_run_id, e.tag, e.status, e.summary]);
  return null;
}
"#;

const MIGRATION: &str = "CREATE TABLE ends (run_id TEXT, root_run_id TEXT, tag TEXT, status TEXT, \
    summary TEXT);\nCREATE TABLE assigned (chore TEXT);\n";

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

/// A runtime with Chores installed and "Can start dots" switched on.
async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    let src = dir.path().join("chores");
    write_app(
        &src,
        &[
            ("app.json", MANIFEST.to_string()),
            ("logic.js", LOGIC.to_string()),
            (
                "ui/index.html",
                "<!doctype html><title>Chores</title>\n".to_string(),
            ),
            ("migrations/0001_chores.sql", MIGRATION.to_string()),
        ],
    );
    rt.apps()
        .install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    rt.apps().set_can_start_dots("chores", true).await.unwrap();
    Rt { dir, rt }
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
    s.uses.apps = vec!["chores".into()];
    s
}

async fn dot(t: &Rt, endpoint: &str, name: &str) -> Dot {
    t.rt.create_dot(local_spec(t, endpoint, name), None, None)
        .await
        .unwrap()
}

async fn assign(t: &Rt, dot: &Dot, chore: &str) -> Result<String, String> {
    t.rt.apps()
        .call(
            "chores",
            "assign",
            json!({"dot": dot.id, "chore": chore}),
            Actor::Person,
        )
        .await
        .map(|v| v.as_str().unwrap().to_string())
}

async fn ends(t: &Rt) -> Vec<Value> {
    let v =
        t.rt.apps()
            .call("chores", "ends", json!({}), Actor::Person)
            .await
            .unwrap();
    v.as_array().unwrap().clone()
}

/// The ends the app heard, once there are `n`.
async fn ends_when(t: &Rt, n: usize) -> Vec<Value> {
    for _ in 0..200 {
        let heard = ends(t).await;
        if heard.len() >= n {
            return heard;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the app heard {:?}, not {n} ends", ends(t).await);
}

#[tokio::test]
async fn a_started_dot_works_in_a_topic_of_its_own_and_the_app_hears_its_end() {
    let t = start().await;
    let llm = Llm::start(vec![llm_text("Dishes done."), llm_text("Dishes again.")]).await;
    let builder = dot(&t, &llm.endpoint(), "builder").await;

    let id = assign(&t, &builder, "dishes").await.unwrap();
    let run = t.rt.store().get_run(&id).await.unwrap();
    assert_eq!(run.trigger, TriggerKind::App);
    assert_eq!(run.root_run_id, id);
    assert_eq!(run.dot_id, builder.id);
    assert_eq!(
        run.payload,
        Some(
            json!({"message": "Do the chore: dishes", "app": "chores", "app_title": "Chores",
                    "tag": "dishes"})
        )
    );
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 30).await;
    let prompt = messages(&llm.requests()[0])
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        prompt.contains("## Message from the app Chores\n\n> Do the chore: dishes\n"),
        "{prompt}"
    );
    assert_eq!(
        ends_when(&t, 1).await,
        [
            json!({"run_id": id, "root_run_id": id, "tag": "dishes", "status": "succeeded",
                "summary": "Dishes done."})
        ]
    );
    // The topic ended, so the same tag opens a new one.
    let again = assign(&t, &builder, "dishes").await.unwrap();
    assert_ne!(again, id);
    wait_status(t.rt.store(), &again, RunStatus::Succeeded, 30).await;
    assert_eq!(ends_when(&t, 2).await[1]["run_id"], again);
    t.rt.shutdown();
}

#[tokio::test]
async fn while_its_topic_waits_a_start_returns_it_and_the_app_hears_the_end_later() {
    let t = start().await;
    let llm = Llm::start(vec![
        llm_call("q1", "ask", json!({"question": "Which sink?"})),
        llm_text("Asked."),
        llm_text("Both sinks done."),
    ])
    .await;
    let builder = dot(&t, &llm.endpoint(), "builder").await;
    let id = assign(&t, &builder, "sinks").await.unwrap();
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 30).await;
    // The open question keeps the topic waiting: the same start gives the same topic, and the
    // app hears nothing yet.
    assert_eq!(assign(&t, &builder, "sinks").await.unwrap(), id);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(ends(&t).await.is_empty());

    let question =
        t.rt.checkins()
            .list_messages(Some(&builder.id))
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.run_id.as_deref() == Some(id.as_str()))
            .unwrap();
    let answer = t.rt.reply(&question.id, "Both.").await.unwrap();
    let answer = wait_status(t.rt.store(), &answer.id, RunStatus::Succeeded, 30).await;
    let heard = ends_when(&t, 1).await;
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0]["run_id"], answer.id);
    assert_eq!(heard[0]["root_run_id"], answer.root_run_id);
    assert_eq!(heard[0]["summary"], "Both sinks done.");
    t.rt.shutdown();
}

#[tokio::test]
async fn the_switch_the_dots_the_pause_and_the_hour_bound_the_starts() {
    let t = start().await;
    // A model nobody answers: started runs fail at once, which these checks do not mind.
    let builder = dot(&t, "http://127.0.0.1:1", "builder").await;
    let mut other_spec = local_spec(&t, "http://127.0.0.1:1", "other");
    other_spec.uses.apps.clear();
    let other = t.rt.create_dot(other_spec, None, None).await.unwrap();
    let runs_of = |dot: &Dot| {
        let (store, id) = (t.rt.store().clone(), dot.id.clone());
        async move { store.list_runs(Some(&id), 100).await.unwrap().len() }
    };

    let refused = assign(&t, &other, "dishes").await.unwrap_err();
    assert!(
        refused.contains(&format!("no dot {} uses this app", other.id)),
        "{refused}"
    );
    assert_eq!(runs_of(&other).await, 0);

    // Over the hour's 20 the whole call rolls back, and nothing is queued.
    let many = |n: u32| {
        t.rt.apps().call(
            "chores",
            "assign_many",
            json!({"dot": builder.id, "n": n}),
            Actor::Person,
        )
    };
    let over = many(21).await.unwrap_err();
    assert!(over.contains("as many dots as the hour allows"), "{over}");
    assert_eq!(runs_of(&builder).await, 0);
    assert_eq!(many(20).await.unwrap().as_array().unwrap().len(), 20);
    assert_eq!(runs_of(&builder).await, 20);
    let spent = assign(&t, &builder, "one more").await.unwrap_err();
    assert!(spent.contains("as many dots as the hour allows"), "{spent}");

    t.rt.set_schedules_paused(true).await.unwrap();
    assert!(assign(&t, &builder, "paused")
        .await
        .unwrap_err()
        .contains(STARTS_PAUSED));
    t.rt.set_schedules_paused(false).await.unwrap();

    // With the switch off, the app's code has no start, and its call fails as a whole.
    t.rt.apps()
        .set_can_start_dots("chores", false)
        .await
        .unwrap();
    let off = assign(&t, &builder, "off").await.unwrap_err();
    assert!(off.contains("not a function"), "{off}");
    assert_eq!(runs_of(&builder).await, 20);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_review_of_an_ended_topic_tells_the_app_nothing_again() {
    let t = start().await;
    let llm = Llm::start(vec![
        llm_call("q1", "ask", json!({"question": "Which sink?"})),
        llm_text("Asked."),
        llm_text("Both sinks done."),
    ])
    .await;
    let builder = dot(&t, &llm.endpoint(), "builder").await;
    let id = assign(&t, &builder, "sinks").await.unwrap();
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 30).await;
    let question =
        t.rt.checkins()
            .list_messages(Some(&builder.id))
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.run_id.as_deref() == Some(id.as_str()))
            .unwrap();
    let answer = t.rt.reply(&question.id, "Both.").await.unwrap();
    let answer = wait_status(t.rt.store(), &answer.id, RunStatus::Succeeded, 30).await;
    assert_eq!(ends_when(&t, 1).await.len(), 1);
    // A review sends every run of the thread anew: none of them is news to the app.
    t.rt.mark_reviewed(&answer.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let heard = ends(&t).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0]["run_id"], answer.id);
    t.rt.shutdown();
}

#[tokio::test]
async fn an_app_cannot_start_a_dot_that_is_off() {
    let t = start().await;
    let llm = Llm::start(vec![]).await;
    let builder = dot(&t, &llm.endpoint(), "builder").await;
    t.rt.set_dot_enabled(&builder.id, false).await.unwrap();
    let refused = assign(&t, &builder, "dishes").await.unwrap_err();
    assert!(
        refused.contains(&format!("no dot {} uses this app", builder.id)),
        "{refused}"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn catching_up_tells_no_end_twice() {
    let t = start().await;
    let llm = Llm::start(vec![llm_text("Dishes done.")]).await;
    let builder = dot(&t, &llm.endpoint(), "builder").await;
    let id = assign(&t, &builder, "dishes").await.unwrap();
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 30).await;
    assert_eq!(ends_when(&t, 1).await.len(), 1);
    t.rt.apps().catch_up().await.unwrap();
    assert_eq!(ends(&t).await.len(), 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_follow_up_keeps_the_topic_waiting_until_it_is_cancelled() {
    let t = start().await;
    let llm = Llm::start(vec![
        llm_call(
            "f1",
            "follow_up",
            json!({"note": "See that the dishes dried.", "in": "2h"}),
        ),
        llm_text("I will look again later."),
    ])
    .await;
    let builder = dot(&t, &llm.endpoint(), "builder").await;
    let id = assign(&t, &builder, "dishes").await.unwrap();
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 30).await;
    let pending = t.rt.follow_ups().list(Some(&builder.id)).await.unwrap();
    assert_eq!(pending.len(), 1);
    // The dot means to go on: the app hears no end, and the same chore is the same topic.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(ends(&t).await.is_empty());
    assert_eq!(assign(&t, &builder, "dishes").await.unwrap(), id);
    // Cancelled, nothing waits: the app hears the end, though no run of the topic ended.
    t.rt.follow_ups().cancel(&pending[0].id).await.unwrap();
    let heard = ends_when(&t, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0]["run_id"], id);
    t.rt.shutdown();
}

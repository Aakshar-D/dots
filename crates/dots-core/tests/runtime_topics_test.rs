mod common;

use std::path::PathBuf;

use common::{spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::model::{DotSpec, RunStatus, WorkspaceMode};
use dots_core::{Config, Runtime};
use serde_json::Value;
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
}

/// Where the fake CLI reads its script.
fn script_path(dir: &TempDir) -> PathBuf {
    dir.path().join("script.json")
}

/// The fake CLI's usual script: a session and a success.
fn succeeds(dir: &TempDir) {
    let script = serde_json::json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false,
                  "result": "fake done", "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    std::fs::write(script_path(dir), script.to_string()).unwrap();
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script_path(&dir).to_string_lossy().to_string(),
    ));
    succeeds(&dir);
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt, args_out }
}

fn folder_spec(dir: &TempDir, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

impl Rt {
    /// The prompt and arguments of the last run the fake CLI ran.
    fn last(&self) -> (String, Vec<String>) {
        let rec: Value =
            serde_json::from_str(&std::fs::read_to_string(&self.args_out).unwrap()).unwrap();
        let args = rec["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        (rec["prompt"].as_str().unwrap().to_string(), args)
    }

    async fn said(&self, dot: &str, text: &str, parent: Option<&str>) -> dots_core::model::Run {
        let run = self.rt.chat(dot, text, parent).await.unwrap();
        wait_status(self.rt.store(), &run.id, RunStatus::Succeeded, 15).await
    }
}

fn resumes(args: &[String]) -> bool {
    args.iter().any(|a| a == "--resume")
}

#[tokio::test]
async fn a_topic_a_message_moved_into_starts_afresh_with_what_was_said() {
    let t = start().await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "mover"), None, None)
            .await
            .unwrap();
    let a1 = t.said(&dot.id, "plan the trip", None).await;
    let a2 = t.said(&dot.id, "book the train", Some(&a1.id)).await;
    let b1 = t.said(&dot.id, "fix the sink", None).await;
    let mut events = t.rt.subscribe();

    let root = t.rt.move_run(&a2.id, Some(&b1.id)).await.unwrap();
    assert_eq!(root, b1.id);
    let mut changed = false;
    while let Ok(ev) = events.try_recv() {
        if matches!(&ev, RuntimeEvent::TopicsChanged { dot_id } if *dot_id == dot.id) {
            changed = true;
        }
    }
    assert!(changed, "topics_changed announced");

    // The target reads along its line: its own run, then the moved one, which is its end; a
    // reply to that end is taken.
    let thread = t.rt.run_thread(&b1.id).await.unwrap();
    let ids: Vec<&str> = thread.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec![b1.id.as_str(), a2.id.as_str()]);
    // The target's next message starts a fresh session and is told the topic so far.
    let next = t.said(&dot.id, "and the tap?", Some(&a2.id)).await;
    let (prompt, args) = t.last();
    assert!(!resumes(&args), "{args:?}");
    let so_far = prompt
        .find("## This conversation so far")
        .expect("the conversation so far");
    let message = prompt.find("## Message from the user").unwrap();
    assert!(so_far < message);
    assert!(prompt[so_far..message].contains("book the train"));
    assert!(prompt[so_far..message].contains("fix the sink"));
    assert!(!prompt[so_far..message].contains("plan the trip"));
    assert_eq!(next.session_id.as_deref(), Some("fake-session"));

    // Only once: the one after it resumes the new session.
    t.said(&dot.id, "thanks", Some(&next.id)).await;
    let (prompt, args) = t.last();
    assert!(resumes(&args));
    assert_eq!(prompt, "thanks");

    // The source topic starts afresh too, without the moved message.
    t.said(&dot.id, "and the hotel?", Some(&a1.id)).await;
    let (prompt, args) = t.last();
    assert!(!resumes(&args));
    let so_far = prompt.find("## This conversation so far").unwrap();
    assert!(prompt[so_far..].contains("plan the trip"));
    assert!(!prompt[so_far..].contains("book the train"));
    t.rt.shutdown();
}

#[tokio::test]
async fn topics_are_renamed_archived_and_merged_and_a_message_unarchives() {
    let t = start().await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "tidy"), None, None)
            .await
            .unwrap();
    let a1 = t.said(&dot.id, "first", None).await;
    let b1 = t.said(&dot.id, "second", None).await;

    let info = t.rt.rename_topic(&a1.id, Some("Errands")).await.unwrap();
    assert_eq!(info.title.as_deref(), Some("Errands"));
    let info = t.rt.set_topic_archived(&a1.id, true).await.unwrap();
    assert!(info.archived_at.is_some());

    // A message into an archived topic brings it back.
    t.said(&dot.id, "more", Some(&a1.id)).await;
    let rows = t.rt.list_topics(&dot.id).await.unwrap();
    let a = rows.iter().find(|r| r.root_run_id == a1.id).unwrap();
    assert_eq!(a.archived_at, None);
    assert_eq!(a.title.as_deref(), Some("Errands"));

    t.rt.merge_topics(&b1.id, &a1.id).await.unwrap();
    assert_eq!(t.rt.run_thread(&a1.id).await.unwrap().len(), 3);
    let rows = t.rt.list_topics(&dot.id).await.unwrap();
    assert!(rows.iter().all(|r| r.root_run_id != b1.id));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_fresh_start_that_fails_leaves_the_next_run_to_start_afresh() {
    let t = start().await;
    let dot =
        t.rt.create_dot(folder_spec(&t.dir, "again"), None, None)
            .await
            .unwrap();
    let a1 = t.said(&dot.id, "plan the trip", None).await;
    let a2 = t.said(&dot.id, "book the train", Some(&a1.id)).await;
    t.rt.move_run(&a2.id, None).await.unwrap();

    // The first run after the move fails.
    std::fs::write(
        script_path(&t.dir),
        serde_json::json!([
            {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
            {"emit": {"type": "result", "subtype": "error_during_execution", "is_error": true,
                      "result": "boom", "usage": {"input_tokens": 1, "output_tokens": 1}}}
        ])
        .to_string(),
    )
    .unwrap();
    let failed =
        t.rt.chat(&dot.id, "and the hotel?", Some(&a1.id))
            .await
            .unwrap();
    wait_status(t.rt.store(), &failed.id, RunStatus::Failed, 15).await;
    let (prompt, args) = t.last();
    assert!(!resumes(&args));
    assert!(prompt.contains("## This conversation so far"));

    // The next one starts afresh again, and is told the topic so far.
    succeeds(&t.dir);
    let next = t.said(&dot.id, "try again", Some(&failed.id)).await;
    let (prompt, args) = t.last();
    assert!(!resumes(&args), "{args:?}");
    assert!(prompt.contains("## This conversation so far"));
    assert!(prompt.contains("plan the trip"));

    // Spent by the run that worked.
    t.said(&dot.id, "thanks", Some(&next.id)).await;
    let (_, args) = t.last();
    assert!(resumes(&args));
    t.rt.shutdown();
}

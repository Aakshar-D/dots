//! The runner and the context rules (context rules spec sections 4 to 7): the fill a run
//! reports is kept; a background turn of a full session starts fresh with a hand-off seed, a
//! fairly full one is compacted first; a chat is compacted and starts fresh only when asked;
//! a resume is left alone; a failed compaction is a notice and the turn goes ahead.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::context_rules::{ContextRules, EventKind, Request, SEED_HEADING};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Compaction, Engine, EngineEvent};
use dots_core::events::new_bus;
use dots_core::model::{
    Dot, EngineKind, MemorySource, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
    engine: ScriptedEngine,
}

fn turn_steps(session: &str, fill: u64) -> Vec<Step> {
    vec![
        Step::Emit(EngineEvent::SessionStarted {
            session_id: session.into(),
        }),
        Step::Emit(EngineEvent::Context {
            fill: Some(fill),
            window: None,
        }),
        Step::Emit(EngineEvent::Context {
            fill: None,
            window: Some(200_000),
        }),
        Step::Emit(EngineEvent::Finished {
            summary: "done".into(),
        }),
    ]
}

async fn env(engine: ScriptedEngine) -> Env {
    let (dir, store) = temp_store().await;
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(engine.clone()));
    let runner = Runner::new(
        store.clone(),
        new_bus(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        dir,
        store,
        runner,
        engine,
    }
}

impl Env {
    async fn dot(&self) -> Dot {
        let mut s = spec("Scout");
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.store.create_dot(&s, None, "default").await.unwrap()
    }

    /// An ended run of `dot` in session `S1` whose last turn filled `fill` of a 200k window.
    async fn earlier(&self, dot: &Dot, fill: u64) -> Run {
        let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
        new.payload = Some(json!({"message": "Fix the parser"}));
        let r = self.store.create_run(&new).await.unwrap();
        self.store.set_session(&r.id, "S1").await.unwrap();
        self.store
            .set_run_context(&r.id, Some(fill), Some(200_000))
            .await
            .unwrap();
        self.store
            .set_workspace(&r.id, &self.dir.path().to_string_lossy(), None, None)
            .await
            .unwrap();
        self.store
            .finish_run(&r.id, RunStatus::Succeeded, Some("Parser fixed"), None)
            .await
            .unwrap();
        self.store.get_run(&r.id).await.unwrap()
    }

    /// The next turn of `parent`'s session, by `trigger`.
    async fn next(&self, parent: &Run, trigger: TriggerKind) -> Run {
        let mut new = NewRun::new(&parent.dot_id, trigger);
        new.payload = Some(json!({"message": "And the tests?"}));
        new.parent_run_id = Some(parent.id.clone());
        new.root_run_id = Some(parent.root_run_id.clone());
        new.session_id = parent.session_id.clone();
        new.workspace_path = parent.workspace_path.clone();
        let run = self.runner.enqueue(new).await.unwrap();
        wait_status(&self.store, &run.id, RunStatus::Succeeded, 10).await
    }

    fn asked(&self) -> Vec<String> {
        self.engine
            .log()
            .into_iter()
            .filter(|l| !l.starts_with("prompt "))
            .collect()
    }

    fn prompt(&self) -> String {
        self.engine
            .log()
            .into_iter()
            .rev()
            .find_map(|l| l.strip_prefix("prompt ").map(str::to_string))
            .unwrap()
    }

    async fn kinds(&self, run_id: &str) -> Vec<String> {
        self.store
            .list_events(run_id, 0)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect()
    }
}

#[tokio::test]
async fn a_runs_fill_and_window_are_kept_on_the_run_not_as_events() {
    let e = env(ScriptedEngine::new(turn_steps("S9", 42_000))).await;
    let dot = e.dot().await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let run = wait_status(&e.store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(
        (run.context_fill, run.context_window),
        (Some(42_000), Some(200_000))
    );
    assert!(!e.kinds(&run.id).await.contains(&"context".to_string()));
    // A first run continues no session: nothing before it.
    assert_eq!(e.asked(), vec!["start -"]);
}

#[tokio::test]
async fn a_full_background_session_starts_fresh_with_a_hand_off() {
    let engine = ScriptedEngine::new(turn_steps("S2", 30_000))
        .summarizing(Ok("We fixed the parser; the tests are next.".into()));
    let e = env(engine).await;
    let dot = e.dot().await;
    e.store
        .add_memory(
            &dot.id,
            "The repo uses tabs.",
            &dots_core::memory::same_key("The repo uses tabs."),
            MemorySource::Person,
            None,
            50,
        )
        .await
        .unwrap();
    let first = e.earlier(&dot, 170_000).await;
    let run = e.next(&first, TriggerKind::FollowUp).await;
    assert_eq!(e.asked(), vec!["handoff S1", "start -"]);
    let prompt = e.prompt();
    assert!(prompt.contains(SEED_HEADING), "{prompt}");
    assert!(prompt.contains("We fixed the parser; the tests are next."));
    assert!(prompt.contains("- The person: Fix the parser"));
    // A first prompt: the instructions and the dot's memories.
    assert!(prompt.contains("The repo uses tabs."), "{prompt}");
    // The new session; the topic is the same.
    assert_eq!(run.session_id.as_deref(), Some("S2"));
    assert_eq!(run.root_run_id, first.id);
    let rows = e.store.list_context_events(&run.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, EventKind::Clear);
    assert_eq!(rows[0].trigger, "auto");
    assert_eq!(rows[0].from_session.as_deref(), Some("S1"));
    assert_eq!(
        (rows[0].fill_before, rows[0].fill_after),
        (170_000, Some(30_000))
    );
    assert!(e.kinds(&run.id).await.contains(&"notice".to_string()));
}

#[tokio::test]
async fn a_fairly_full_session_is_compacted_before_the_turn() {
    let engine = ScriptedEngine::new(turn_steps("S1", 40_000)).compacting(Ok(Compaction {
        before: 125_000,
        after: 30_000,
    }));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 125_000).await;
    let run = e.next(&first, TriggerKind::FollowUp).await;
    assert_eq!(e.asked(), vec!["compact S1", "start S1"]);
    assert!(!e.prompt().contains(SEED_HEADING));
    let rows = e.store.list_context_events(&run.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, EventKind::Compact);
    assert_eq!(
        (rows[0].fill_before, rows[0].fill_after),
        (125_000, Some(30_000))
    );
    assert!(e.kinds(&run.id).await.contains(&"compacted".to_string()));
    assert_eq!(e.store.context_counts(&dot.id).await.unwrap(), (1, 0));
}

#[tokio::test]
async fn a_chat_over_the_clear_threshold_is_compacted_not_started_fresh() {
    let engine = ScriptedEngine::new(turn_steps("S1", 40_000))
        .compacting(Ok(Compaction {
            before: 170_000,
            after: 30_000,
        }))
        .summarizing(Ok("note".into()));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 170_000).await;
    e.next(&first, TriggerKind::Chat).await;
    assert_eq!(e.asked(), vec!["compact S1", "start S1"]);
}

#[tokio::test]
async fn a_chat_starts_fresh_when_the_person_asked_and_the_request_is_spent() {
    let engine = ScriptedEngine::new(turn_steps("S2", 20_000)).summarizing(Ok("note".into()));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 20_000).await;
    e.store
        .set_thread_request(&first.id, Some(Request::Fresh))
        .await
        .unwrap();
    let run = e.next(&first, TriggerKind::Chat).await;
    assert_eq!(e.asked(), vec!["handoff S1", "start -"]);
    assert_eq!(e.store.thread_state(&first.id).await.unwrap().request, None);
    let rows = e.store.list_context_events(&run.id).await.unwrap();
    assert_eq!(rows[0].trigger, "manual");
    // The next one continues the new session as it is.
    e.next(&run, TriggerKind::Chat).await;
    assert_eq!(e.asked()[2..], ["start S2".to_string()]);
}

#[tokio::test]
async fn a_resume_is_left_alone() {
    let engine = ScriptedEngine::new(turn_steps("S1", 40_000))
        .compacting(Ok(Compaction {
            before: 190_000,
            after: 30_000,
        }))
        .summarizing(Ok("note".into()));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 190_000).await;
    e.store
        .set_thread_request(&first.id, Some(Request::Compact))
        .await
        .unwrap();
    e.next(&first, TriggerKind::Resume).await;
    assert_eq!(e.asked(), vec!["start S1"]);
    // The request waits for a turn it applies to.
    assert_eq!(
        e.store.thread_state(&first.id).await.unwrap().request,
        Some(Request::Compact)
    );
}

#[tokio::test]
async fn rules_switched_off_for_the_dot_leave_a_full_session_alone() {
    let engine = ScriptedEngine::new(turn_steps("S1", 40_000)).summarizing(Ok("note".into()));
    let e = env(engine).await;
    let mut dot = e.dot().await;
    dot.spec.context_rules = Some(ContextRules {
        enabled: false,
        ..ContextRules::default()
    });
    let dot = e.store.update_dot(&dot.id, &dot.spec).await.unwrap();
    let first = e.earlier(&dot, 190_000).await;
    e.next(&first, TriggerKind::FollowUp).await;
    assert_eq!(e.asked(), vec!["start S1"]);
}

#[tokio::test]
async fn a_failed_compaction_is_a_notice_and_the_turn_goes_ahead() {
    let engine = ScriptedEngine::new(turn_steps("S1", 140_000))
        .compacting(Err("claude exited with code 1".into()));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 130_000).await;
    let run = e.next(&first, TriggerKind::FollowUp).await;
    assert_eq!(e.asked(), vec!["compact S1", "start S1"]);
    let notices: Vec<String> = e
        .store
        .list_events(&run.id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "notice")
        .map(|ev| ev.data["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        notices,
        vec![
            "Compacting the conversation failed: claude exited with code 1; it goes on as it was."
        ]
    );
    assert!(e
        .store
        .list_context_events(&run.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_failed_hand_off_starts_fresh_without_a_note() {
    let engine = ScriptedEngine::new(turn_steps("S2", 20_000)).summarizing(Err("timeout".into()));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 180_000).await;
    e.next(&first, TriggerKind::FollowUp).await;
    assert_eq!(e.asked(), vec!["handoff S1", "start -"]);
    assert!(e.prompt().contains("(No hand-off note could be written.)"));
}

#[tokio::test]
async fn keep_going_is_spent_once_the_fill_is_under_the_clear_threshold() {
    let engine = ScriptedEngine::new(turn_steps("S1", 20_000));
    let e = env(engine).await;
    let dot = e.dot().await;
    let first = e.earlier(&dot, 20_000).await;
    e.store.set_keep_going(&first.id, true).await.unwrap();
    e.next(&first, TriggerKind::Chat).await;
    assert!(!e.store.thread_state(&first.id).await.unwrap().keep_going);
}

#[tokio::test]
async fn the_cli_compacts_by_itself_at_the_compact_share_and_that_is_recorded() {
    let engine = ScriptedEngine::new(vec![
        Step::Emit(EngineEvent::Compacted {
            trigger: "auto".into(),
            before: 130_000,
            after: 25_000,
        }),
        Step::Emit(EngineEvent::Finished {
            summary: "done".into(),
        }),
    ]);
    let e = env(engine).await;
    let dot = e.dot().await;
    let run = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let run = wait_status(&e.store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(run.context_fill, Some(25_000));
    let rows = e.store.list_context_events(&run.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trigger, "auto");
}

//! What needs the person (needs-you spec sections 2 and 3): each dot's open questions and
//! pending approvals, what is new since the person last looked at it, `mark_dot_seen`, and the
//! dot being viewed, which stays seen as its runs end and its notes arrive. Runs are stored but
//! never started (`max_concurrent_runs` is 0).

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::spec;
use dots_core::checkins::Tool;
use dots_core::events::RuntimeEvent;
use dots_core::model::{Dot, MessageKind, NewRun, Run, RunStatus, TriggerKind};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

struct Rt {
    _dir: TempDir,
    rt: Runtime,
}

/// A runtime that stores runs and never starts one.
async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { _dir: dir, rt }
}

impl Rt {
    fn store(&self) -> &Store {
        self.rt.store()
    }

    async fn dot(&self, name: &str) -> Dot {
        self.rt.create_dot(spec(name)).await.unwrap()
    }

    /// A run of `dot` started by `trigger`, still queued.
    async fn queued(&self, dot: &Dot, trigger: TriggerKind) -> Run {
        self.store()
            .create_run(&NewRun::new(&dot.id, trigger))
            .await
            .unwrap()
    }

    /// A run of `dot` started by `trigger` that ended with `status`, stored without an event.
    async fn ended(&self, dot: &Dot, trigger: TriggerKind, status: RunStatus) -> Run {
        let run = self.queued(dot, trigger).await;
        self.store()
            .finish_run(&run.id, status, None, None)
            .await
            .unwrap()
    }

    /// The run's call of a check-in tool, which must be delivered.
    async fn checkin(&self, run: &Run, tool: Tool, input: Value) {
        let answer = self.rt.checkins().call(&run.id, tool, &input).await;
        assert!(answer.is_ok(), "{answer:?}");
    }

    /// The dot's `(needs_you, unread)` in the overview.
    async fn counts(&self, dot: &Dot) -> (u32, u32) {
        let rows = self.rt.dot_overview().await.unwrap();
        let row = rows.iter().find(|r| r.dot.id == dot.id).unwrap();
        (row.needs_you, row.unread)
    }

    /// Waits (at most 5 s) until the dot's counts are `want`.
    async fn counts_become(&self, dot: &Dot, want: (u32, u32)) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let counts = self.counts(dot).await;
            if counts == want {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "counts {counts:?}, want {want:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Times are stored to the millisecond: what ends after a `mark_dot_seen` must end later.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

/// Waits (at most 5 s) for the `DotSeen` of `dot_id`.
async fn seen(events: &mut Receiver<RuntimeEvent>, dot_id: &str) {
    let wait = async {
        loop {
            if let RuntimeEvent::DotSeen { dot_id: id } = events.recv().await.unwrap() {
                if id == dot_id {
                    return;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait)
        .await
        .expect("a dot_seen event for the dot");
}

#[tokio::test]
async fn a_dot_counts_what_needs_the_person_and_what_is_new_since_it_was_seen() {
    let t = start().await;
    let dot = t.dot("worker").await;
    let other = t.dot("other").await;
    // Never seen: nothing that ended or was said is new, so an upgrade shows no wall of results.
    let old = t
        .ended(&dot, TriggerKind::Schedule, RunStatus::Succeeded)
        .await;
    t.checkin(&old, Tool::Notify, json!({"text": "Done before."}))
        .await;
    assert_eq!(t.rt.get_dot(&dot.id).await.unwrap().seen_at, None);
    assert_eq!(t.counts(&dot).await, (0, 0));

    // Needs you: an open question and a pending approval. A dismissed question and an expired
    // approval are closed.
    let asking = t
        .ended(&dot, TriggerKind::Manual, RunStatus::Succeeded)
        .await;
    t.checkin(&asking, Tool::Ask, json!({"question": "Ship it?"}))
        .await;
    let asked_before = t
        .ended(&dot, TriggerKind::Manual, RunStatus::Succeeded)
        .await;
    t.checkin(&asked_before, Tool::Ask, json!({"question": "Old?"}))
        .await;
    let old_question = t
        .store()
        .list_messages(Some(&dot.id))
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.kind == MessageKind::Question && m.text == "Old?")
        .unwrap();
    t.rt.checkins().dismiss(&old_question.id).await.unwrap();
    let waiting = t.queued(&dot, TriggerKind::Manual).await;
    t.store()
        .create_approval(&waiting.id, "Bash", &json!({"command": "ls"}))
        .await
        .unwrap();
    let expired = t.queued(&dot, TriggerKind::Manual).await;
    t.store()
        .create_approval(&expired.id, "Bash", &json!({"command": "rm -r out"}))
        .await
        .unwrap();
    t.store().expire_pending_for_run(&expired.id).await.unwrap();
    assert_eq!(t.counts(&dot).await, (2, 2));

    // New after a seen: runs that ended, whatever started them (a chat too) and however they
    // ended, and notes. A run still queued is not.
    t.rt.mark_dot_seen(&dot.id).await.unwrap();
    tick().await;
    t.ended(&dot, TriggerKind::Chat, RunStatus::Succeeded).await;
    t.ended(&dot, TriggerKind::Schedule, RunStatus::Failed)
        .await;
    t.ended(&dot, TriggerKind::Webhook, RunStatus::Cancelled)
        .await;
    let noting = t.queued(&dot, TriggerKind::Manual).await;
    t.checkin(&noting, Tool::Notify, json!({"text": "Halfway."}))
        .await;
    assert_eq!(t.counts(&dot).await, (2, 6));
    assert_eq!(t.counts(&other).await, (0, 0));
    assert_eq!(t.rt.unread_total().await.unwrap(), 6);

    // Seen again: only what needs the person is left.
    tick().await;
    t.rt.mark_dot_seen(&dot.id).await.unwrap();
    assert_eq!(t.counts(&dot).await, (2, 2));
    assert_eq!(t.rt.unread_total().await.unwrap(), 2);
}

#[tokio::test]
async fn marking_a_dot_seen_keeps_the_time_and_says_so() {
    let t = start().await;
    let dot = t.dot("worker").await;
    let mut events = t.rt.subscribe();
    t.rt.mark_dot_seen(&dot.id).await.unwrap();
    seen(&mut events, &dot.id).await;
    let stored = t.rt.get_dot(&dot.id).await.unwrap();
    let at = stored.seen_at.clone().expect("seen_at is set");
    assert!(at.ends_with('Z') && at >= dot.created_at, "{at}");
    // Looking at a dot does not change it.
    assert_eq!(stored.updated_at, dot.updated_at);
    assert!(matches!(
        t.rt.mark_dot_seen("no-such-dot").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn the_dot_being_viewed_stays_seen_as_its_runs_end_and_its_notes_arrive() {
    let t = start().await;
    let viewed = t.dot("viewed").await;
    let other = t.dot("other").await;
    t.rt.mark_dot_seen(&viewed.id).await.unwrap();
    t.rt.mark_dot_seen(&other.id).await.unwrap();
    tick().await;
    let mut events = t.rt.subscribe();
    assert_eq!(t.rt.viewing(), None);
    t.rt.set_viewing(Some(&viewed.id));
    assert_eq!(t.rt.viewing().as_deref(), Some(viewed.id.as_str()));

    // A run of the viewed dot ends (cancelled, which publishes it): the core marks it seen.
    let run = t.queued(&viewed, TriggerKind::Schedule).await;
    t.rt.cancel_run(&run.id).await.unwrap();
    seen(&mut events, &viewed.id).await;
    t.counts_become(&viewed, (0, 0)).await;
    // The other dot's run is new.
    let elsewhere = t.queued(&other, TriggerKind::Schedule).await;
    t.rt.cancel_run(&elsewhere.id).await.unwrap();
    assert_eq!(t.counts(&other).await, (0, 1));

    // A note of the viewed dot.
    let noting = t.queued(&viewed, TriggerKind::Manual).await;
    tick().await;
    t.checkin(&noting, Tool::Notify, json!({"text": "Look at this."}))
        .await;
    seen(&mut events, &viewed.id).await;
    t.counts_become(&viewed, (0, 0)).await;

    // Viewed no more: what ends is new again.
    t.rt.set_viewing(None);
    tick().await;
    t.rt.cancel_run(&noting.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(t.counts(&viewed).await, (0, 1));
}

#[tokio::test]
async fn a_deleted_dot_is_no_longer_viewed_or_counted() {
    let t = start().await;
    let dot = t.dot("gone").await;
    let run = t
        .ended(&dot, TriggerKind::Manual, RunStatus::Succeeded)
        .await;
    t.checkin(&run, Tool::Ask, json!({"question": "Still there?"}))
        .await;
    t.rt.set_viewing(Some(&dot.id));
    assert_eq!(t.rt.unread_total().await.unwrap(), 1);
    t.rt.delete_dot(&dot.id).await.unwrap();
    assert_eq!(t.rt.viewing(), None);
    assert!(t.rt.dot_overview().await.unwrap().is_empty());
    assert_eq!(t.rt.unread_total().await.unwrap(), 0);
}

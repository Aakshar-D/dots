//! Snooze and Done on the Inbox (snooze and done spec sections 4.1 to 4.4): item keys and
//! titles, the sections without hidden items, the Back, Snoozed and Done lists, the Not
//! delivered section, the checks, and the count. The runtime runs on a `FakeClock`, the queue
//! is held before any run is created, and every item is built through the store.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::apps::propose::AppProposal;
use dots_core::events::RuntimeEvent;
use dots_core::inbox::{entries, title_cut, InboxKey, InboxKind};
use dots_core::instructions::{InstructionProposal, InstructionStatus};
use dots_core::limits::FIVE_HOUR;
use dots_core::model::{
    Approval, Dot, DotSpec, FolderConfig, Message, NewRun, PausedReason, Run, RunStatus,
    ScheduleChange, TriggerKind, WatcherConfig, WorkspaceMode,
};
use dots_core::schedule::Schedule;
use dots_core::scheduler::Clock;
use dots_core::store::{LimitReading, Store};
use dots_core::util::timestamp;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-04T10:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

struct T {
    dir: TempDir,
    rt: Runtime,
    clock: Arc<FakeClock>,
}

/// A runtime at `t0` with its queue held. With `usage_hold`, Claude's five-hour window is
/// full until `t0 + 5h`, so queued background runs are held for usage.
async fn start_with(usage_hold: bool) -> T {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    if usage_hold {
        let store = Store::open(&data.join(dots_core::store::DB_FILE))
            .await
            .unwrap();
        store
            .save_limit_readings(&[LimitReading {
                window: FIVE_HOUR.into(),
                utilization: 1.0,
                resets_at: timestamp(t0() + TimeDelta::hours(5)),
                status: "allowed".into(),
                read_at: timestamp(t0()),
            }])
            .await
            .unwrap();
        store.close().await;
    }
    let clock = FakeClock::new(t0());
    let mut cfg = Config::new(data);
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    // No dispatcher may claim the runs these tests create.
    rt.hold_runs(true).await;
    T { dir, rt, clock }
}

async fn start() -> T {
    start_with(false).await
}

fn key(kind: InboxKind, id: &str) -> InboxKey {
    InboxKey::new(kind, id)
}

fn invalid(e: Error) -> String {
    match e {
        Error::Invalid(m) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn not_found(e: Error) -> String {
    match e {
        Error::NotFound(m) => m,
        other => panic!("expected NotFound, got {other:?}"),
    }
}

impl T {
    fn store(&self) -> &Store {
        self.rt.store()
    }

    async fn dot_with(&self, name: &str, schedule: Option<Schedule>) -> Dot {
        let mut s: DotSpec = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.schedule = schedule;
        self.rt.create_dot(s, None, None).await.unwrap()
    }

    async fn dot(&self, name: &str) -> Dot {
        self.dot_with(name, None).await
    }

    async fn queued(&self, dot: &Dot, trigger: TriggerKind) -> Run {
        self.store()
            .create_run(&NewRun::new(&dot.id, trigger))
            .await
            .unwrap()
    }

    async fn ended(&self, dot: &Dot, trigger: TriggerKind, status: RunStatus) -> Run {
        let run = self.queued(dot, trigger).await;
        assert!(self.store().claim_run(&run.id).await.unwrap());
        self.store()
            .mark_delivered(&run.id, Utc::now())
            .await
            .unwrap();
        self.store()
            .finish_run(&run.id, status, None, Some("boom"))
            .await
            .unwrap()
    }

    async fn question(&self, dot: &Dot, text: &str) -> Message {
        let run = self
            .ended(dot, TriggerKind::Manual, RunStatus::Succeeded)
            .await;
        self.store()
            .add_question(&dot.id, &run.id, text, None)
            .await
            .unwrap()
            .unwrap()
    }

    async fn note(&self, dot: &Dot, text: &str) -> Message {
        self.store().add_note(&dot.id, None, text).await.unwrap()
    }

    /// A pending approval of a run that waits for it.
    async fn approval(&self, dot: &Dot) -> Approval {
        let run = self.queued(dot, TriggerKind::Manual).await;
        self.store()
            .set_status(&run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();
        self.store()
            .create_approval(&run.id, "Bash", &json!({"command": "ls"}))
            .await
            .unwrap()
    }

    /// A chat run that failed before its engine started: not delivered.
    async fn undelivered(&self, dot: &Dot) -> Run {
        let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
        new.payload = Some(json!({ "message": "Ship it", "typed": true }));
        let run = self.store().create_run(&new).await.unwrap();
        assert!(self.store().claim_run(&run.id).await.unwrap());
        let run = self
            .store()
            .finish_run(
                &run.id,
                RunStatus::Failed,
                None,
                Some("claude CLI not found"),
            )
            .await
            .unwrap();
        assert!(run.undelivered_at.is_some());
        run
    }

    /// A paused watcher of `dot`, paused at `at`; returns its id.
    async fn paused_watcher(&self, dot: &Dot, at: &str) -> String {
        let folder = self.dir.path().to_string_lossy().to_string();
        let w = self
            .store()
            .add_watcher(
                &dot.id,
                "inbox",
                &WatcherConfig::Folder(FolderConfig::new(&folder)),
                None,
            )
            .await
            .unwrap();
        self.store()
            .pause_watcher(&w.id, PausedReason::Error, Some("gone"), at)
            .await
            .unwrap()
            .unwrap();
        w.id
    }

    /// A scheduled dot whose schedule needs attention; returns the failed run.
    async fn attention(&self, name: &str) -> Run {
        let every: Schedule = serde_json::from_value(json!({
            "rule": {"kind": "every", "minutes": 720}, "catch_up": true, "until": null
        }))
        .unwrap();
        let dot = self.dot_with(name, Some(every)).await;
        let run = self
            .ended(&dot, TriggerKind::Schedule, RunStatus::Failed)
            .await;
        let change = ScheduleChange {
            attention_run_id: Some(Some(run.id.clone())),
            ..Default::default()
        };
        self.store()
            .update_schedule_state(&dot.id, &change, t0())
            .await
            .unwrap()
            .unwrap();
        // The scheduler marks such a run reviewed when it judges it: it is not a run to review.
        self.store()
            .mark_thread_reviewed(&run.root_run_id)
            .await
            .unwrap();
        run
    }

    async fn instructions(&self, dot: &Dot, id: &str) {
        let run = self
            .ended(dot, TriggerKind::Chat, RunStatus::Succeeded)
            .await;
        self.store()
            .add_instruction_proposal(&InstructionProposal {
                id: id.into(),
                dot_id: dot.id.clone(),
                run_id: run.id.clone(),
                root_run_id: run.root_run_id.clone(),
                base: "Do the thing.".into(),
                instructions: "Do the thing better.".into(),
                reason: "Why".into(),
                status: InstructionStatus::Pending,
                answer_run_id: None,
                created_at: timestamp(t0()),
                decided_at: None,
            })
            .await
            .unwrap();
    }

    async fn app_review(&self, dot: &Dot, id: &str) {
        let run = self.queued(dot, TriggerKind::Manual).await;
        let proposal: AppProposal = serde_json::from_value(json!({
            "id": id, "dot_id": dot.id, "run_id": run.id, "root_run_id": run.root_run_id,
            "app_name": "notes", "app_id": null, "base_version": null, "note": "Built it.",
            "summary": {
                "title": "Notes", "description": "", "tools": [], "permissions": [],
                "tests": {"results": []}, "files": 0, "bytes": 0, "migrations": [],
                "update_from": null
            },
            "status": "pending", "answer": null, "answer_run_id": null,
            "created_at": timestamp(t0()), "decided_at": null
        }))
        .unwrap();
        self.store().add_app_proposal(&proposal).await.unwrap();
    }

    async fn snooze(&self, items: &[InboxKey], minutes: i64) {
        self.rt
            .inbox_snooze(items, self.clock.now() + TimeDelta::minutes(minutes))
            .await
            .unwrap();
    }

    /// Every key of the whole Inbox, hidden items included, through the public functions.
    async fn shown_keys(&self) -> Vec<InboxKey> {
        let inbox = self.rt.inbox().await.unwrap();
        let messages: Vec<Message> = self
            .store()
            .waiting_messages()
            .await
            .unwrap()
            .into_iter()
            .filter(|m| !inbox.hidden_messages.contains(&m.id))
            .collect();
        entries(&inbox, &messages)
            .into_iter()
            .map(|e| e.key)
            .collect()
    }
}

#[tokio::test]
async fn every_kind_has_its_key_and_title() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let question = t.question(&dot, "Ship it?\nOr wait?").await;
    let note = t.note(&dot, "Half the tests pass.").await;
    let approval = t.approval(&dot).await;
    let undelivered = t.undelivered(&dot).await;
    let failed = t.ended(&dot, TriggerKind::Manual, RunStatus::Failed).await;
    let watcher = t.paused_watcher(&dot, "2026-11-04T09:00:00.000Z").await;
    let attention = t.attention("nightly").await;
    t.instructions(&dot, "ins1").await;
    t.app_review(&dot, "app1").await;

    let inbox = t.rt.inbox().await.unwrap();
    let messages = t.store().waiting_messages().await.unwrap();
    let got: Vec<(InboxKey, String)> = entries(&inbox, &messages)
        .into_iter()
        .map(|e| (e.key, e.title))
        .collect();
    let want = vec![
        (
            key(InboxKind::Message, &note.id),
            "courier says: Half the tests pass.".to_string(),
        ),
        (
            key(InboxKind::Message, &question.id),
            "courier asks: Ship it?".to_string(),
        ),
        (
            key(InboxKind::Approval, &approval.id),
            "courier asks to use Bash".to_string(),
        ),
        (
            key(InboxKind::NotDelivered, &undelivered.id),
            "courier didn't get your message".to_string(),
        ),
        (
            key(InboxKind::Review, &failed.id),
            "courier: run to review".to_string(),
        ),
        (
            key(InboxKind::Attention, &attention.id),
            "nightly's schedule needs attention".to_string(),
        ),
        (
            key(
                InboxKind::Watcher,
                &format!("{watcher}@2026-11-04T09:00:00.000Z"),
            ),
            "courier's watcher inbox paused".to_string(),
        ),
        (
            key(InboxKind::Instructions, "ins1"),
            "courier proposed new instructions".to_string(),
        ),
        (
            key(InboxKind::AppReview, "app1"),
            "courier sent Notes for review".to_string(),
        ),
    ];
    // The note and the question were made in order, but maybe in the same millisecond: compare
    // the messages as a set, then the rest in order.
    let mut got_messages = got[..2].to_vec();
    let mut want_messages = want[..2].to_vec();
    got_messages.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    want_messages.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    assert_eq!(got_messages, want_messages);
    assert_eq!(got[2..], want[2..]);

    let long = "x".repeat(81);
    let cut = title_cut(&long);
    assert_eq!(cut.chars().count(), 80);
    assert!(cut.ends_with('…'));
    assert_eq!(title_cut(&"y".repeat(80)), "y".repeat(80));
}

#[tokio::test]
async fn a_snoozed_item_leaves_its_section_and_shows_in_snoozed() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let approval = t.approval(&dot).await;
    let failed = t.ended(&dot, TriggerKind::Manual, RunStatus::Failed).await;
    let attention = t.attention("nightly").await;
    let watcher = t.paused_watcher(&dot, "2026-11-04T09:00:00.000Z").await;
    t.instructions(&dot, "ins1").await;
    t.app_review(&dot, "app1").await;
    let undelivered = t.undelivered(&dot).await;

    let before = t.rt.inbox().await.unwrap();
    assert_eq!(before.approvals.len(), 1);
    assert_eq!(before.review_runs.len(), 1);
    assert_eq!(before.attention.len(), 1);
    assert_eq!(before.watchers.len(), 1);
    assert_eq!(before.instructions.len(), 1);
    assert_eq!(before.app_reviews.len(), 1);
    assert_eq!(before.not_delivered.len(), 1);

    // Each a minute later than the one before: the Snoozed list is soonest first.
    let keys = [
        key(InboxKind::Approval, &approval.id),
        key(InboxKind::Review, &failed.id),
        key(InboxKind::Attention, &attention.id),
        key(
            InboxKind::Watcher,
            &format!("{watcher}@2026-11-04T09:00:00.000Z"),
        ),
        key(InboxKind::Instructions, "ins1"),
        key(InboxKind::AppReview, "app1"),
        key(InboxKind::NotDelivered, &undelivered.id),
    ];
    for (i, k) in keys.iter().enumerate().rev() {
        t.snooze(std::slice::from_ref(k), 60 + i as i64).await;
    }
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.approvals.is_empty());
    assert!(inbox.review_runs.is_empty());
    assert!(inbox.attention.is_empty());
    assert!(inbox.watchers.is_empty());
    assert!(inbox.instructions.is_empty());
    assert!(inbox.app_reviews.is_empty());
    assert!(inbox.not_delivered.is_empty());
    assert!(inbox.back.is_empty() && inbox.done.is_empty());
    let snoozed: Vec<InboxKey> = inbox.snoozed.iter().map(|m| m.key()).collect();
    assert_eq!(snoozed, keys);
    let first = &inbox.snoozed[0];
    assert_eq!(first.title, "courier asks to use Bash");
    assert_eq!(first.dot_id.as_deref(), Some(dot.id.as_str()));
    assert_eq!(
        first.snoozed_until.as_deref(),
        Some(timestamp(t0() + TimeDelta::minutes(60)).as_str())
    );
}

#[tokio::test]
async fn done_items_show_for_seven_days() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let question = t.question(&dot, "Ship it?").await;
    let k = key(InboxKind::Message, &question.id);
    t.rt.inbox_done(&k).await.unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.done.len(), 1);
    assert_eq!(inbox.done[0].title, "courier asks: Ship it?");
    assert_eq!(
        inbox.done[0].done_at.as_deref(),
        Some(timestamp(t0()).as_str())
    );
    assert_eq!(inbox.hidden_messages, vec![question.id.clone()]);

    t.clock
        .set(t0() + TimeDelta::days(7) - TimeDelta::milliseconds(1));
    assert_eq!(t.rt.inbox().await.unwrap().done.len(), 1);

    t.clock
        .set(t0() + TimeDelta::days(7) + TimeDelta::milliseconds(1));
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.done.is_empty());
    // No longer listed, and still hidden.
    assert_eq!(inbox.hidden_messages, vec![question.id.clone()]);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    assert_eq!(t.store().dot_counts(&dot.id).await.unwrap().0, 0);
}

#[tokio::test]
async fn an_approval_can_be_snoozed_but_not_done() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let approval = t.approval(&dot).await;
    let k = key(InboxKind::Approval, &approval.id);
    match t.rt.inbox_done(&k).await.unwrap_err() {
        Error::Conflict(m) => assert_eq!(m, "An approval is decided, not marked done."),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert_eq!(t.store().count_marks().await.unwrap(), 0);

    t.snooze(std::slice::from_ref(&k), 60).await;
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.approvals.is_empty());
    assert_eq!(inbox.snoozed.len(), 1);
    let run = t.store().get_run(&approval.run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::AwaitingApproval);
    assert_eq!(
        t.store().get_approval(&approval.id).await.unwrap().status,
        approval.status
    );
}

#[tokio::test]
async fn deciding_a_snoozed_approval_takes_it_away() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let approval = t.approval(&dot).await;
    t.snooze(&[key(InboxKind::Approval, &approval.id)], 60)
        .await;
    assert_eq!(t.rt.inbox().await.unwrap().snoozed.len(), 1);
    t.rt.decide_approval(&approval.id, true, None, None)
        .await
        .unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.snoozed.is_empty());
    assert!(inbox.approvals.is_empty());
}

#[tokio::test]
async fn snooze_checks_its_input() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let question = t.question(&dot, "Ship it?").await;
    let k = key(InboxKind::Message, &question.id);
    let now = t.clock.now();

    assert!(matches!(
        t.rt.inbox_snooze(&[], now + TimeDelta::hours(1)).await,
        Err(Error::Invalid(_))
    ));
    for until in [now, now - TimeDelta::minutes(1), now + TimeDelta::days(366)] {
        let e =
            t.rt.inbox_snooze(std::slice::from_ref(&k), until)
                .await
                .unwrap_err();
        assert_eq!(invalid(e), "Pick a time in the future.");
    }
    t.rt.inbox_snooze(std::slice::from_ref(&k), now + TimeDelta::days(365))
        .await
        .unwrap();
    t.rt.inbox_unsnooze(&k).await.unwrap();

    let unknown = key(InboxKind::Review, "no-such-run");
    let e =
        t.rt.inbox_snooze(&[k.clone(), unknown.clone()], now + TimeDelta::hours(1))
            .await
            .unwrap_err();
    assert_eq!(not_found(e), "That is no longer in the Inbox.");
    assert_eq!(t.store().count_marks().await.unwrap(), 0);
    let e = t.rt.inbox_done(&unknown).await.unwrap_err();
    assert_eq!(not_found(e), "That is no longer in the Inbox.");

    let e = t.rt.inbox_unsnooze(&k).await.unwrap_err();
    assert_eq!(not_found(e), "That is not snoozed.");
    let e = t.rt.inbox_undo_done(&k).await.unwrap_err();
    assert_eq!(not_found(e), "That is not done.");
    // A done item is not snoozed, and a snoozed one is not done.
    t.rt.inbox_done(&k).await.unwrap();
    let e = t.rt.inbox_unsnooze(&k).await.unwrap_err();
    assert_eq!(not_found(e), "That is not snoozed.");
}

#[tokio::test]
async fn unsnooze_and_undo_put_it_back_without_a_tag() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let failed = t.ended(&dot, TriggerKind::Manual, RunStatus::Failed).await;
    let question = t.question(&dot, "Ship it?").await;
    let review = key(InboxKind::Review, &failed.id);
    let message = key(InboxKind::Message, &question.id);
    t.snooze(std::slice::from_ref(&review), 60).await;
    t.rt.inbox_done(&message).await.unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.review_runs.is_empty());
    assert_eq!(inbox.hidden_messages, vec![question.id.clone()]);

    t.rt.inbox_unsnooze(&review).await.unwrap();
    t.rt.inbox_undo_done(&message).await.unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.review_runs.len(), 1);
    assert!(inbox.hidden_messages.is_empty());
    assert!(inbox.back.is_empty() && inbox.snoozed.is_empty() && inbox.done.is_empty());
    assert_eq!(t.store().count_marks().await.unwrap(), 0);
}

#[tokio::test]
async fn a_watcher_paused_again_is_shown() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let first = "2026-11-04T09:00:00.000Z";
    let watcher = t.paused_watcher(&dot, first).await;
    t.snooze(
        &[key(InboxKind::Watcher, &format!("{watcher}@{first}"))],
        60,
    )
    .await;
    assert!(t.rt.inbox().await.unwrap().watchers.is_empty());

    t.store().resume_watcher(&watcher).await.unwrap();
    let again = "2026-11-04T09:30:00.000Z";
    t.store()
        .pause_watcher(&watcher, PausedReason::Flood, None, again)
        .await
        .unwrap()
        .unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.watchers.len(), 1);
    assert_eq!(inbox.watchers[0].at, again);
    // The old mark names an item that left: not listed.
    assert!(inbox.snoozed.is_empty());
    assert_eq!(t.store().dot_counts(&dot.id).await.unwrap().0, 1);
}

#[tokio::test]
async fn not_delivered_has_its_own_section() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let run = t.undelivered(&dot).await;
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.not_delivered.len(), 1);
    assert_eq!(inbox.not_delivered[0].run.id, run.id);
    assert_eq!(inbox.not_delivered[0].dot_name, "courier");
    assert!(inbox.review_runs.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    t.rt.discard_delivery(&run.id).await.unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.not_delivered.is_empty());
    assert!(inbox.review_runs.is_empty());
}

#[tokio::test]
async fn inbox_count_leaves_out_hidden_items() {
    let t = start_with(true).await;
    let dot = t.dot("courier").await;
    let question = t.question(&dot, "Ship it?").await;
    let failed = t.ended(&dot, TriggerKind::Manual, RunStatus::Failed).await;
    let held_a = t.queued(&dot, TriggerKind::Schedule).await;
    let held_b = t.queued(&dot, TriggerKind::Watcher).await;
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.held.len(), 2, "{:?}", inbox.held);
    // The question, the run to review and one line for the held runs.
    assert_eq!(t.rt.inbox_count().await.unwrap(), 3);

    t.snooze(&[key(InboxKind::Message, &question.id)], 60).await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 2);
    t.rt.inbox_done(&key(InboxKind::Review, &failed.id))
        .await
        .unwrap();
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    // Some held runs hidden: the line stays. All of them: it goes.
    t.snooze(&[key(InboxKind::Held, &held_a.id)], 60).await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    t.snooze(&[key(InboxKind::Held, &held_b.id)], 60).await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    assert!(t.shown_keys().await.is_empty());
}

#[tokio::test]
async fn each_change_sends_inbox_updated() {
    let t = start().await;
    let dot = t.dot("courier").await;
    let question = t.question(&dot, "Ship it?").await;
    let k = key(InboxKind::Message, &question.id);
    let mut events = t.rt.subscribe();
    fn updates(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> usize {
        let mut n = 0;
        while let Ok(ev) = events.try_recv() {
            if matches!(ev, RuntimeEvent::InboxUpdated) {
                n += 1;
            }
        }
        n
    }
    t.snooze(std::slice::from_ref(&k), 60).await;
    assert_eq!(updates(&mut events), 1);
    t.rt.inbox_unsnooze(&k).await.unwrap();
    assert_eq!(updates(&mut events), 1);
    t.rt.inbox_done(&k).await.unwrap();
    assert_eq!(updates(&mut events), 1);
    t.rt.inbox_undo_done(&k).await.unwrap();
    assert_eq!(updates(&mut events), 1);
    // A refused call sends nothing.
    assert!(t.rt.inbox_undo_done(&k).await.is_err());
    assert_eq!(updates(&mut events), 0);
}

//! A check-in call performed (check-ins spec sections 2, 3 and 5): a note and a question are
//! stored and announced, one question per run; a follow-up is scheduled within its limits, in
//! order; calls made at once cannot pass the limits; and a call that cannot be stored is not
//! delivered, without failing the run.

mod common;

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::checkins::{local_label, CheckIns, Tool, ASKED, ASKED_ALREADY, SENT};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::followups::{FollowUps, TOO_FAR, TOO_MANY, TOO_SOON};
use dots_core::model::{FollowUpStatus, MessageKind, NewRun, Run, TriggerKind};
use dots_core::store::Store;
use dots_core::util::timestamp;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

struct Setup {
    dir: TempDir,
    store: Store,
    checkins: CheckIns,
    follow_ups: Arc<FollowUps>,
    events: Receiver<RuntimeEvent>,
    /// A queued run of the dot `checker`: the run that calls the tools.
    run: Run,
}

/// Check-ins on a fake clock at `T0`.
async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let follow_ups = FollowUps::new(store.clone(), bus.clone(), FakeClock::new(t0()));
    let checkins = CheckIns::new(store.clone(), bus.clone(), follow_ups.clone());
    let dot = store.create_dot(&spec("checker")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    Setup {
        dir,
        store,
        checkins,
        follow_ups,
        events: bus.subscribe(),
        run,
    }
}

impl Setup {
    async fn call(&self, tool: Tool, input: Value) -> Result<String, String> {
        self.checkins.call(&self.run.id, tool, &input).await
    }
}

#[tokio::test]
async fn a_note_is_stored_and_announced() {
    let mut t = setup().await;
    assert_eq!(
        t.call(Tool::Notify, json!({"text": " Half way. "})).await,
        Ok(SENT.to_string())
    );
    let stored = t.store.list_messages(None).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, MessageKind::Note);
    assert_eq!(stored[0].text, "Half way.");
    assert_eq!(stored[0].run_id.as_deref(), Some(t.run.id.as_str()));
    match t.events.try_recv().unwrap() {
        RuntimeEvent::MessagePosted { message } => assert_eq!(message, stored[0]),
        other => panic!("{other:?}"),
    }
    // A broken rule is answered, and nothing is stored or announced.
    assert_eq!(
        t.call(Tool::Notify, json!({"text": ""})).await,
        Err("`text` must not be blank.".to_string())
    );
    assert_eq!(t.store.list_messages(None).await.unwrap().len(), 1);
    assert!(t.events.try_recv().is_err());
}

#[tokio::test]
async fn a_run_asks_one_question_with_its_choices() {
    let t = setup().await;
    assert_eq!(
        t.call(
            Tool::Ask,
            json!({"question": "Ship it?", "choices": ["yes", "no"]})
        )
        .await,
        Ok(ASKED.to_string())
    );
    assert_eq!(
        t.call(Tool::Ask, json!({"question": "Really?"})).await,
        Err(ASKED_ALREADY.to_string())
    );
    let stored = t.store.list_messages(None).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].kind, MessageKind::Question);
    assert_eq!(stored[0].text, "Ship it?");
    assert_eq!(
        stored[0].choices,
        Some(vec!["yes".to_string(), "no".to_string()])
    );
    // Another run of the dot asks its own.
    let other = t
        .store
        .create_run(&NewRun::new(&t.run.dot_id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(
        t.checkins
            .call(&other.id, Tool::Ask, &json!({"question": "And you?"}))
            .await,
        Ok(ASKED.to_string())
    );
}

#[tokio::test]
async fn a_follow_up_is_scheduled_within_its_limits_in_order() {
    let mut t = setup().await;
    let answer = t
        .call(
            Tool::FollowUp,
            json!({"note": "Check the build.", "in": "2h"}),
        )
        .await
        .unwrap();
    let pending = t.follow_ups.list(Some(&t.run.dot_id)).await.unwrap();
    assert_eq!(pending.len(), 1);
    let due = t0() + TimeDelta::hours(2);
    assert_eq!(pending[0].note, "Check the build.");
    assert_eq!(pending[0].from_run_id, t.run.id);
    assert_eq!(pending[0].due_at, timestamp(due));
    assert_eq!(pending[0].created_at, T0);
    assert_eq!(
        answer,
        format!(
            "Follow-up {} scheduled for {}.",
            pending[0].id,
            local_label(due)
        )
    );
    assert!(matches!(
        t.events.try_recv().unwrap(),
        RuntimeEvent::FollowUpsUpdated { dot_id } if dot_id == t.run.dot_id
    ));
    for later in ["3h", "4h"] {
        assert!(t
            .call(Tool::FollowUp, json!({"note": "n", "in": later}))
            .await
            .is_ok());
    }
    // The limits in order: a call that breaks two of them is told the first.
    assert_eq!(
        t.call(Tool::FollowUp, json!({"note": "n", "in": "1m"}))
            .await,
        Err(TOO_SOON.to_string())
    );
    assert_eq!(
        t.call(Tool::FollowUp, json!({"note": "n", "in": "31d"}))
            .await,
        Err(TOO_FAR.to_string())
    );
    assert_eq!(
        t.call(Tool::FollowUp, json!({"note": "n", "in": "1h"}))
            .await,
        Err(TOO_MANY.to_string())
    );
    // Cancelling one makes room.
    let cancelled = t.follow_ups.cancel(&pending[0].id).await.unwrap();
    assert_eq!(cancelled.status, FollowUpStatus::Cancelled);
    assert!(t
        .call(Tool::FollowUp, json!({"note": "n", "in": "1h"}))
        .await
        .is_ok());
}

/// Review Focus 2: Claude may issue several tool calls in one turn.
#[tokio::test]
async fn parallel_calls_store_one_question_and_three_follow_ups() {
    let t = setup().await;
    let first = json!({"question": "First?"});
    let second = json!({"question": "Second?"});
    let (a, b) = tokio::join!(
        t.checkins.call(&t.run.id, Tool::Ask, &first),
        t.checkins.call(&t.run.id, Tool::Ask, &second),
    );
    let mut asked = [a, b];
    asked.sort();
    assert_eq!(
        asked,
        [Ok(ASKED.to_string()), Err(ASKED_ALREADY.to_string())]
    );

    let inputs: Vec<Value> = ["1h", "2h", "3h", "4h"]
        .iter()
        .map(|later| json!({"note": "n", "in": later}))
        .collect();
    let (a, b, c, d) = tokio::join!(
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[0]),
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[1]),
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[2]),
        t.checkins.call(&t.run.id, Tool::FollowUp, &inputs[3]),
    );
    let refused: Vec<String> = [a, b, c, d].into_iter().filter_map(Result::err).collect();
    assert_eq!(refused, [TOO_MANY.to_string()]);
    assert_eq!(t.follow_ups.list(None).await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_call_that_cannot_be_stored_is_not_delivered() {
    let t = setup().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        t.dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("ALTER TABLE messages RENAME TO messages_away")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let answer = t
        .call(Tool::Notify, json!({"text": "Half way."}))
        .await
        .unwrap_err();
    assert!(
        answer.starts_with("Not delivered: ") && answer.ends_with('.'),
        "{answer}"
    );
    let unknown = t
        .checkins
        .call("no-such-run", Tool::Ask, &json!({"question": "q"}))
        .await
        .unwrap_err();
    assert_eq!(unknown, "Not delivered: not found: run no-such-run.");
}

/// A dot in a loop cannot flood the Inbox (spec section 1, anti-goals): a run sends at most
/// `MAX_NOTES` notes, even when it sends them all at once; Dots' own notes and another run's
/// notes do not count.
#[tokio::test]
async fn a_run_sends_at_most_ten_notes_even_at_once() {
    use dots_core::checkins::{MAX_NOTES, NOTES_SPENT};
    let s = setup().await;
    assert_eq!(MAX_NOTES, 10);
    let calls: Vec<_> = (0..12)
        .map(|i| {
            let (checkins, run_id) = (s.checkins.clone(), s.run.id.clone());
            tokio::spawn(async move {
                checkins
                    .call(&run_id, Tool::Notify, &json!({"text": format!("note {i}")}))
                    .await
            })
        })
        .collect();
    let mut answers = Vec::new();
    for call in calls {
        answers.push(call.await.unwrap());
    }
    let sent = answers.iter().filter(|a| a.as_deref() == Ok(SENT)).count();
    let refused = answers
        .iter()
        .filter(|a| a.as_ref().err().map(String::as_str) == Some(NOTES_SPENT))
        .count();
    assert_eq!((sent, refused), (10, 2), "{answers:?}");
    assert_eq!(s.store.list_messages(None).await.unwrap().len(), 10);
    // Dots' own notes and another run's are not this run's.
    s.store
        .add_note(&s.run.dot_id, None, "A follow-up was skipped.")
        .await
        .unwrap();
    let other = s
        .store
        .create_run(&NewRun::new(&s.run.dot_id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(
        s.checkins
            .call(&other.id, Tool::Notify, &json!({"text": "fresh"}))
            .await,
        Ok(SENT.to_string())
    );
}

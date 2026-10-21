//! The `messages` and `follow_ups` rows (check-ins spec section 7): round trips, the one
//! question per run and the pending limit that the inserts check themselves, answering and
//! firing in one transaction with their run, and the rows going with their dot.

mod common;

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::model::{
    Dot, FollowUpStatus, Message, MessageKind, NewRun, Run, RunStatus, TriggerKind,
    QUESTION_ANSWERED, QUESTION_DISMISSED,
};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;

const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// A dot and a queued run of it.
async fn dot_and_run(store: &Store, name: &str) -> (Dot, Run) {
    let dot = store.create_dot(&spec(name)).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    (dot, run)
}

/// The chat run that answers a question of `run`.
fn answer_of(run: &Run) -> NewRun {
    let mut new = NewRun::new(&run.dot_id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "yes"}));
    new.parent_run_id = Some(run.id.clone());
    new.root_run_id = Some(run.root_run_id.clone());
    new
}

/// Lets the clock move on, so rows written next sort after the ones before.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

fn ids(list: Vec<Message>) -> Vec<String> {
    list.into_iter().map(|m| m.id).collect()
}

#[tokio::test]
async fn notes_and_questions_round_trip_and_name_their_dot() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "asker").await;
    let note = store
        .add_note(&dot.id, Some(&run.id), "Half way there.")
        .await
        .unwrap();
    assert_eq!(note.kind, MessageKind::Note);
    assert_eq!(note.dot_name, "asker");
    assert_eq!(note.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(note.choices, None);
    assert!(note.waits());
    tick().await;
    let own = store
        .add_note(&dot.id, None, "A follow-up was skipped.")
        .await
        .unwrap();
    assert_eq!(own.run_id, None);
    tick().await;
    let choices = vec!["yes".to_string(), "no".to_string()];
    let question = store
        .add_question(&dot.id, &run.id, "Ship it?", Some(&choices))
        .await
        .unwrap()
        .expect("the run's first question");
    assert_eq!(question.kind, MessageKind::Question);
    assert_eq!(question.choices, Some(choices));
    assert_eq!(store.get_message(&question.id).await.unwrap(), question);
    // Newest first, of one dot or of all.
    assert_eq!(
        ids(store.list_messages(Some(&dot.id)).await.unwrap()),
        [question.id.clone(), own.id.clone(), note.id.clone()]
    );
    assert_eq!(store.list_messages(None).await.unwrap().len(), 3);
    assert!(store
        .list_messages(Some("no-such-dot"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.count_waiting_messages().await.unwrap(), 3);

    let read = store.mark_message_read(&note.id).await.unwrap();
    assert!(read.read_at.is_some());
    assert!(!read.waits());
    // A second read keeps the first time.
    assert_eq!(
        store.mark_message_read(&note.id).await.unwrap().read_at,
        read.read_at
    );
    // A question that was read still waits for its answer.
    assert!(store.mark_message_read(&question.id).await.unwrap().waits());
    assert_eq!(store.count_waiting_messages().await.unwrap(), 2);
    assert!(matches!(
        store.get_message("nope").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.mark_message_read("nope").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn a_run_asks_one_question_and_two_asks_at_once_store_one() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "asker").await;
    let (a, b) = tokio::join!(
        store.add_question(&dot.id, &run.id, "First?", None),
        store.add_question(&dot.id, &run.id, "Second?", None),
    );
    let stored: Vec<Message> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(stored.len(), 1, "one question per run");
    assert!(store
        .add_question(&dot.id, &run.id, "Third?", None)
        .await
        .unwrap()
        .is_none());
    // A note does not count, and another run asks its own.
    store
        .add_note(&dot.id, Some(&run.id), "a note")
        .await
        .unwrap();
    let other = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store
        .add_question(&dot.id, &other.id, "Again?", None)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn answering_stores_the_run_with_the_answer_and_closes_the_question() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "asker").await;
    let question = store
        .add_question(&dot.id, &run.id, "Ship it?", None)
        .await
        .unwrap()
        .unwrap();
    let (answered, child) = store
        .answer_question(&question.id, &answer_of(&run))
        .await
        .unwrap();
    assert_eq!(child.status, RunStatus::Queued);
    assert_eq!(child.trigger, TriggerKind::Chat);
    assert_eq!(child.parent_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(answered.answer_run_id.as_deref(), Some(child.id.as_str()));
    assert!(answered.answered_at.is_some());
    assert!(!answered.waits());
    assert!(answered.refusal().is_some());

    // A closed question is refused, and no run is stored for it.
    let again = store.answer_question(&question.id, &answer_of(&run)).await;
    assert!(
        matches!(&again, Err(Error::Conflict(m)) if m == QUESTION_ANSWERED),
        "{again:?}"
    );
    let dismissed = store.dismiss_question(&question.id).await;
    assert!(
        matches!(&dismissed, Err(Error::Conflict(m)) if m == QUESTION_ANSWERED),
        "{dismissed:?}"
    );
    assert_eq!(
        store.list_thread(&run.root_run_id).await.unwrap().len(),
        2,
        "only the first answer stored a run"
    );

    let other = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let second = store
        .add_question(&dot.id, &other.id, "Which branch?", None)
        .await
        .unwrap()
        .unwrap();
    assert!(second.refusal().is_none());
    let closed = store.dismiss_question(&second.id).await.unwrap();
    assert!(closed.dismissed_at.is_some());
    assert!(!closed.waits());
    // A second dismiss changes nothing.
    assert_eq!(store.dismiss_question(&second.id).await.unwrap(), closed);
    let late = store.answer_question(&second.id, &answer_of(&other)).await;
    assert!(
        matches!(&late, Err(Error::Conflict(m)) if m == QUESTION_DISMISSED),
        "{late:?}"
    );
    assert_eq!(
        store.list_thread(&other.root_run_id).await.unwrap().len(),
        1
    );

    let note = store.add_note(&dot.id, None, "a note").await.unwrap();
    assert!(matches!(
        store.dismiss_question(&note.id).await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        store.answer_question(&note.id, &answer_of(&run)).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn follow_ups_round_trip_and_four_at_once_store_the_three_allowed() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "later").await;
    let at = |minutes: i64| t0() + TimeDelta::minutes(minutes);
    let (a, b, c, d) = tokio::join!(
        store.add_follow_up(&dot.id, &run.id, at(30), "third", 3, t0()),
        store.add_follow_up(&dot.id, &run.id, at(10), "first", 3, t0()),
        store.add_follow_up(&dot.id, &run.id, at(20), "second", 3, t0()),
        store.add_follow_up(&dot.id, &run.id, at(40), "fourth", 3, t0()),
    );
    let stored: Vec<_> = [a, b, c, d].into_iter().flat_map(Result::unwrap).collect();
    assert_eq!(stored.len(), 3, "four at once store three");
    let pending = store.list_pending_follow_ups(Some(&dot.id)).await.unwrap();
    assert_eq!(pending.len(), 3);
    assert!(
        pending.windows(2).all(|w| w[0].due_at <= w[1].due_at),
        "soonest first: {pending:#?}"
    );
    let first = pending[0].clone();
    assert_eq!(first.status, FollowUpStatus::Pending);
    assert_eq!(first.from_run_id, run.id);
    assert_eq!(first.created_at, timestamp(t0()));
    assert_eq!(first.attempts, 0);
    assert_eq!(first.retry_at, None);
    assert_eq!(store.get_follow_up(&first.id).await.unwrap(), first);
    assert!(store
        .list_pending_follow_ups(Some("no-such-dot"))
        .await
        .unwrap()
        .is_empty());

    // A failed start is counted; then the row fires, and its run is stored with it.
    assert!(store.retry_follow_up(&first.id, at(11)).await.unwrap());
    let retried = store.get_follow_up(&first.id).await.unwrap();
    assert_eq!(retried.attempts, 1);
    assert_eq!(retried.retry_at, Some(timestamp(at(11))));
    let mut new = NewRun::new(&dot.id, TriggerKind::FollowUp);
    new.payload = Some(json!({"message": "later", "follow_up_id": first.id}));
    let fired = store.fire_follow_up(&first.id, &new).await.unwrap();
    assert_eq!(fired.trigger, TriggerKind::FollowUp);
    assert_eq!(fired.status, RunStatus::Queued);
    let row = store.get_follow_up(&first.id).await.unwrap();
    assert_eq!(row.status, FollowUpStatus::Fired);
    assert_eq!(row.fired_run_id.as_deref(), Some(fired.id.as_str()));
    assert_eq!(row.retry_at, None);
    assert!(matches!(
        store.fire_follow_up(&first.id, &new).await,
        Err(Error::Conflict(_))
    ));
    assert!(!store.retry_follow_up(&first.id, at(12)).await.unwrap());
    // A fired one no longer counts: there is room for one more.
    assert!(store
        .add_follow_up(&dot.id, &run.id, at(50), "fifth", 3, t0())
        .await
        .unwrap()
        .is_some());

    let pending = store.list_pending_follow_ups(Some(&dot.id)).await.unwrap();
    let cancelled = store.cancel_follow_up(&pending[0].id).await.unwrap();
    assert_eq!(cancelled.status, FollowUpStatus::Cancelled);
    assert!(matches!(
        store.cancel_follow_up(&pending[0].id).await,
        Err(Error::Conflict(_))
    ));
    assert!(store.skip_follow_up(&pending[1].id, true).await.unwrap());
    let skipped = store.get_follow_up(&pending[1].id).await.unwrap();
    assert_eq!(skipped.status, FollowUpStatus::Skipped);
    assert_eq!(skipped.attempts, 1);
    assert!(!store.skip_follow_up(&pending[1].id, false).await.unwrap());
    assert_eq!(store.list_pending_follow_ups(None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_deleted_dot_takes_its_messages_and_follow_ups_with_it() {
    let (_dir, store) = temp_store().await;
    let (dot, run) = dot_and_run(&store, "gone").await;
    let note = store.add_note(&dot.id, Some(&run.id), "bye").await.unwrap();
    let follow_up = store
        .add_follow_up(
            &dot.id,
            &run.id,
            t0() + TimeDelta::hours(1),
            "later",
            3,
            t0(),
        )
        .await
        .unwrap()
        .unwrap();
    store.delete_dot(&dot.id).await.unwrap();
    assert!(matches!(
        store.get_message(&note.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.get_follow_up(&follow_up.id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(store.count_waiting_messages().await.unwrap(), 0);
    assert!(store
        .list_pending_follow_ups(None)
        .await
        .unwrap()
        .is_empty());
}

mod common;

use chrono::{DateTime, Duration, Utc};
use common::{spec, temp_store};
use dots_core::instructions::{InstructionProposal, InstructionStatus};
use dots_core::model::{Dot, NewRun, Run, RunStatus, TriggerKind};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;

const NOW: &str = "2026-11-04T12:00:00.000Z";

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// `minutes` after 2026-11-01 09:00 UTC.
fn at(minutes: i64) -> String {
    timestamp(t("2026-11-01T09:00:00.000Z") + Duration::minutes(minutes))
}

async fn dot(store: &Store, name: &str) -> Dot {
    store
        .create_dot(&spec(name), None, "default")
        .await
        .unwrap()
}

/// An ended chat run queued at `queued`, continuing `parent` (none starts a topic).
async fn chat(store: &Store, dot: &Dot, parent: Option<&Run>, queued: &str) -> Run {
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({ "message": format!("at {queued}"), "typed": true }));
    if let Some(p) = parent {
        new.parent_run_id = Some(p.id.clone());
        new.root_run_id = Some(p.root_run_id.clone());
        new.session_id = p.session_id.clone();
    } else {
        new.session_id = Some(format!("session-{queued}"));
    }
    let run = store.create_run(&new).await.unwrap();
    store.set_run_queued_at(&run.id, queued).await.unwrap();
    store
        .set_status(&run.id, RunStatus::Succeeded)
        .await
        .unwrap();
    store
        .set_run_times(&run.id, queued, Some(queued))
        .await
        .unwrap();
    store.get_run(&run.id).await.unwrap()
}

fn invalid(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_topic_gets_a_title_and_is_archived_and_unarchived() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let root = chat(&store, &d, None, &at(0)).await;
    assert!(store.list_topics(&d.id).await.unwrap().is_empty());

    let info = store
        .set_topic_title(&root.id, Some("  Trip plans  "))
        .await
        .unwrap();
    assert_eq!(info.title.as_deref(), Some("Trip plans"));
    assert_eq!(info.dot_id, d.id);

    let info = store
        .set_topic_archived(&root.id, true, t(NOW))
        .await
        .unwrap();
    assert_eq!(info.archived_at.as_deref(), Some(NOW));
    assert_eq!(info.title.as_deref(), Some("Trip plans"));

    let info = store
        .set_topic_archived(&root.id, false, t(NOW))
        .await
        .unwrap();
    assert_eq!(info.archived_at, None);

    // Empty clears the title; a long one is refused.
    let info = store.set_topic_title(&root.id, Some("   ")).await.unwrap();
    assert_eq!(info.title, None);
    let long = "x".repeat(101);
    assert!(store.set_topic_title(&root.id, Some(&long)).await.is_err());

    assert_eq!(store.list_topics(&d.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn only_a_topic_s_root_names_it() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let root = chat(&store, &d, None, &at(0)).await;
    let reply = chat(&store, &d, Some(&root), &at(1)).await;
    invalid(
        store
            .set_topic_title(&reply.id, Some("x"))
            .await
            .unwrap_err(),
    );
    invalid(
        store
            .set_topic_archived(&reply.id, true, t(NOW))
            .await
            .unwrap_err(),
    );
}

#[tokio::test]
async fn a_message_moves_with_its_replies_to_another_topic() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let a1 = chat(&store, &d, None, &at(0)).await;
    let a2 = chat(&store, &d, Some(&a1), &at(1)).await;
    let a3 = chat(&store, &d, Some(&a2), &at(2)).await;
    let b1 = chat(&store, &d, None, &at(10)).await;
    let b2 = chat(&store, &d, Some(&b1), &at(11)).await;
    store
        .add_instruction_proposal(&InstructionProposal {
            id: "p1".into(),
            dot_id: d.id.clone(),
            run_id: a3.id.clone(),
            root_run_id: a1.id.clone(),
            base: "Do the thing.".into(),
            instructions: "Do it twice.".into(),
            reason: "Why".into(),
            status: InstructionStatus::Pending,
            answer_run_id: None,
            created_at: at(2),
            decided_at: None,
        })
        .await
        .unwrap();

    let root = store.move_runs(&a2.id, Some(&b1.id)).await.unwrap();
    assert_eq!(root, b1.id);

    let a: Vec<String> = store
        .list_thread(&a1.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(a, vec![a1.id.clone()]);
    let b: Vec<String> = store
        .list_thread(&b1.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    // Read along the line: the target's runs, then the moved ones, though they are older.
    assert_eq!(
        b,
        vec![b1.id.clone(), b2.id.clone(), a2.id.clone(), a3.id.clone()]
    );
    let moved = store.get_run(&a2.id).await.unwrap();
    assert_eq!(moved.parent_run_id.as_deref(), Some(b2.id.as_str()));
    assert_eq!(
        store
            .get_run(&a3.id)
            .await
            .unwrap()
            .parent_run_id
            .as_deref(),
        Some(a2.id.as_str())
    );
    assert_eq!(
        store
            .get_instruction_proposal("p1")
            .await
            .unwrap()
            .root_run_id,
        b1.id
    );
    // Both topics start their next run afresh, once.
    assert!(store.take_topic_restart(&a1.id).await.unwrap());
    assert!(!store.take_topic_restart(&a1.id).await.unwrap());
    assert!(store.take_topic_restart(&b1.id).await.unwrap());
}

#[tokio::test]
async fn a_message_moved_to_a_new_topic_becomes_its_root() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let a1 = chat(&store, &d, None, &at(0)).await;
    let a2 = chat(&store, &d, Some(&a1), &at(1)).await;
    let a3 = chat(&store, &d, Some(&a2), &at(2)).await;

    let root = store.move_runs(&a2.id, None).await.unwrap();
    assert_eq!(root, a2.id);
    let head = store.get_run(&a2.id).await.unwrap();
    assert_eq!(head.root_run_id, a2.id);
    assert_eq!(head.parent_run_id, None);
    assert_eq!(store.get_run(&a3.id).await.unwrap().root_run_id, a2.id);
    assert_eq!(store.list_thread(&a1.id).await.unwrap().len(), 1);
    assert!(store.take_topic_restart(&a2.id).await.unwrap());
    assert!(store.take_topic_restart(&a1.id).await.unwrap());
}

#[tokio::test]
async fn moves_that_would_break_a_topic_are_refused() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let other = dot(&store, "b").await;
    let a1 = chat(&store, &d, None, &at(0)).await;
    let a2 = chat(&store, &d, Some(&a1), &at(1)).await;
    let b1 = chat(&store, &d, None, &at(10)).await;
    let c1 = chat(&store, &other, None, &at(20)).await;

    // The root moves only with its whole topic: a merge.
    let m = invalid(store.move_runs(&a1.id, Some(&b1.id)).await.unwrap_err());
    assert!(m.contains("merge"), "{m}");
    // Into its own topic, into another dot's, into a run that is no topic's root.
    invalid(store.move_runs(&a2.id, Some(&a1.id)).await.unwrap_err());
    invalid(store.move_runs(&a2.id, Some(&c1.id)).await.unwrap_err());
    invalid(store.move_runs(&a2.id, Some(&a2.id)).await.unwrap_err());

    // A topic with a run that has not ended, on either side.
    let b2 = chat(&store, &d, Some(&b1), &at(11)).await;
    store.set_status(&b2.id, RunStatus::Running).await.unwrap();
    invalid(store.move_runs(&a2.id, Some(&b1.id)).await.unwrap_err());
    store
        .set_status(&a2.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    invalid(store.move_runs(&a2.id, None).await.unwrap_err());

    // Nothing moved, and no topic is marked to restart.
    assert_eq!(store.get_run(&a2.id).await.unwrap().root_run_id, a1.id);
    assert!(!store.take_topic_restart(&a1.id).await.unwrap());
    assert!(!store.take_topic_restart(&b1.id).await.unwrap());
}

#[tokio::test]
async fn a_topic_an_app_started_is_not_moved() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let mut new = NewRun::new(&d.id, TriggerKind::App);
    new.payload = Some(json!({ "message": "from the app" }));
    let app_root = store
        .start_app_run("app-1", "tag", &new, &at(0))
        .await
        .unwrap();
    store
        .set_status(&app_root.id, RunStatus::Succeeded)
        .await
        .unwrap();
    let a1 = chat(&store, &d, None, &at(5)).await;
    let a2 = chat(&store, &d, Some(&a1), &at(6)).await;
    invalid(
        store
            .move_runs(&a2.id, Some(&app_root.id))
            .await
            .unwrap_err(),
    );
    invalid(store.merge_topics(&app_root.id, &a1.id).await.unwrap_err());
}

#[tokio::test]
async fn a_merge_joins_two_topics_under_the_second() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let a1 = chat(&store, &d, None, &at(0)).await;
    let a2 = chat(&store, &d, Some(&a1), &at(1)).await;
    let b1 = chat(&store, &d, None, &at(10)).await;
    store.set_topic_title(&a1.id, Some("A")).await.unwrap();
    store.set_topic_title(&b1.id, Some("B")).await.unwrap();

    store.merge_topics(&a1.id, &b1.id).await.unwrap();
    let b: Vec<String> = store
        .list_thread(&b1.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(b, vec![b1.id.clone(), a1.id.clone(), a2.id.clone()]);
    assert_eq!(
        store
            .get_run(&a1.id)
            .await
            .unwrap()
            .parent_run_id
            .as_deref(),
        Some(b1.id.as_str())
    );
    let topics = store.list_topics(&d.id).await.unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].root_run_id, b1.id);
    assert_eq!(topics[0].title.as_deref(), Some("B"));
    assert!(store.take_topic_restart(&b1.id).await.unwrap());

    // A topic into itself, or a run that is no root.
    invalid(store.merge_topics(&b1.id, &b1.id).await.unwrap_err());
    invalid(store.merge_topics(&a2.id, &b1.id).await.unwrap_err());
}

#[tokio::test]
async fn idle_topics_are_archived_but_not_the_current_or_newest_or_busy_one() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let e = dot(&store, "b").await;
    // Dot a: an old chat topic, a busy old one, one unarchived lately, and the current chat
    // topic (also old).
    let old = chat(&store, &d, None, &at(0)).await;
    let busy = chat(&store, &d, None, &at(1)).await;
    let busy_reply = chat(&store, &d, Some(&busy), &at(2)).await;
    store
        .set_status(&busy_reply.id, RunStatus::Queued)
        .await
        .unwrap();
    let kept = chat(&store, &d, None, &at(3)).await;
    let current = chat(&store, &d, None, &at(4)).await;
    // Dot b: its current chat topic and a newer run topic, its most recently active; both old.
    let e_current = chat(&store, &e, None, &at(0)).await;
    let manual = store
        .create_run(&NewRun::new(&e.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.set_run_queued_at(&manual.id, &at(5)).await.unwrap();
    store
        .set_status(&manual.id, RunStatus::Succeeded)
        .await
        .unwrap();
    store
        .set_run_times(&manual.id, &at(5), Some(&at(5)))
        .await
        .unwrap();

    let now = t("2026-11-01T09:00:00.000Z") + Duration::days(40);
    // Unarchived 5 days ago: that counts as activity.
    store
        .set_topic_archived(&kept.id, false, now - Duration::days(5))
        .await
        .unwrap();
    // Off: nothing.
    assert!(store.archive_idle_topics(now, 0).await.unwrap().is_empty());

    let archived = store.archive_idle_topics(now, 30).await.unwrap();
    let roots: Vec<&str> = archived.iter().map(|i| i.root_run_id.as_str()).collect();
    assert_eq!(roots, vec![old.id.as_str()]);
    assert_eq!(
        archived[0].archived_at.as_deref(),
        Some(timestamp(now).as_str())
    );
    for left in [&busy, &kept, &current, &e_current, &manual] {
        let info = store.topic_info(&left.id).await.unwrap();
        assert!(info.and_then(|i| i.archived_at).is_none(), "{}", left.id);
    }
    // Archived ones are not archived again.
    assert!(store.archive_idle_topics(now, 30).await.unwrap().is_empty());
}

#[tokio::test]
async fn unused_grants_follow_the_moved_runs_and_an_archived_target_comes_back() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let a1 = chat(&store, &d, None, &at(0)).await;
    let a2 = chat(&store, &d, Some(&a1), &at(1)).await;
    let b1 = chat(&store, &d, None, &at(10)).await;
    store
        .create_mcp_grant(&a1.id, "Bash", "h1", None)
        .await
        .unwrap();
    store
        .set_topic_archived(&b1.id, true, t(NOW))
        .await
        .unwrap();

    store.move_runs(&a2.id, Some(&b1.id)).await.unwrap();
    assert!(!store.take_grant(&a1.id, "Bash", "h1").await.unwrap());
    assert!(store.take_grant(&b1.id, "Bash", "h1").await.unwrap());
    let info = store.topic_info(&b1.id).await.unwrap().unwrap();
    assert_eq!(info.archived_at, None);

    // A merge into an archived topic brings it back too.
    let c1 = chat(&store, &d, None, &at(20)).await;
    store
        .set_topic_archived(&b1.id, true, t(NOW))
        .await
        .unwrap();
    store.merge_topics(&c1.id, &b1.id).await.unwrap();
    let info = store.topic_info(&b1.id).await.unwrap().unwrap();
    assert_eq!(info.archived_at, None);
}

#[tokio::test]
async fn a_run_added_to_an_archived_topic_unarchives_it() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let a1 = chat(&store, &d, None, &at(0)).await;
    store
        .set_topic_archived(&a1.id, true, t(NOW))
        .await
        .unwrap();
    // Any run, not only a person's message: here a follow-up.
    let mut new = NewRun::new(&d.id, TriggerKind::FollowUp);
    new.parent_run_id = Some(a1.id.clone());
    new.root_run_id = Some(a1.id.clone());
    store.create_run(&new).await.unwrap();
    let info = store.topic_info(&a1.id).await.unwrap().unwrap();
    assert_eq!(info.archived_at, None);
}

#[tokio::test]
async fn a_topic_with_something_waiting_is_not_archived() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    let asks = chat(&store, &d, None, &at(0)).await;
    let follows = chat(&store, &d, None, &at(1)).await;
    let idle = chat(&store, &d, None, &at(2)).await;
    let _current = chat(&store, &d, None, &at(3)).await;
    store
        .add_question(&d.id, &asks.id, "Which one?", None)
        .await
        .unwrap()
        .unwrap();
    let now = t("2026-11-01T09:00:00.000Z") + Duration::days(40);
    store
        .add_follow_up(
            &d.id,
            &follows.id,
            now + Duration::days(1),
            "check",
            10,
            now,
        )
        .await
        .unwrap()
        .unwrap();
    let archived = store.archive_idle_topics(now, 30).await.unwrap();
    let roots: Vec<&str> = archived.iter().map(|i| i.root_run_id.as_str()).collect();
    assert_eq!(roots, vec![idle.id.as_str()]);
}

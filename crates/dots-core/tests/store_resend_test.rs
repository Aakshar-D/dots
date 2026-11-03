//! Resending a person's message that Dots was answering when it stopped (crash resend spec).

mod common;

use common::{spec, temp_store};
use dots_core::model::{NewRun, Run, RunStatus, TriggerKind};
use dots_core::store::Store;
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::ConnectOptions;

/// A run of `trigger` that was working when Dots stopped.
async fn running(store: &Store, new: NewRun) -> Run {
    let run = store.create_run(&new).await.unwrap();
    assert!(store.claim_run(&run.id).await.unwrap());
    store.get_run(&run.id).await.unwrap()
}

fn message(dot: &str, text: &str) -> NewRun {
    let mut new = NewRun::new(dot, TriggerKind::Chat);
    new.payload = Some(json!({ "message": text, "typed": true }));
    new
}

/// The runs that retry `id`.
async fn resends_of(store: &Store, dot: &str, id: &str) -> Vec<Run> {
    store
        .list_runs_by_status(RunStatus::Queued)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.dot_id == dot && r.retry_of.as_deref() == Some(id))
        .collect()
}

#[tokio::test]
async fn an_interrupted_message_is_resent() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    // A new chat: its run set a session and a folder of its own before Dots stopped.
    let fresh = running(&store, message(&dot.id, "hello")).await;
    store.set_session(&fresh.id, "own-session").await.unwrap();
    store
        .set_workspace(&fresh.id, "/tmp/own", Some("dots/own"), Some("abc"))
        .await
        .unwrap();
    // A reply in a topic whose last run ended.
    let first = store.create_run(&message(&dot.id, "first")).await.unwrap();
    store.claim_run(&first.id).await.unwrap();
    store.set_session(&first.id, "topic-session").await.unwrap();
    store
        .set_workspace(&first.id, "/tmp/topic", Some("dots/topic"), Some("def"))
        .await
        .unwrap();
    store
        .finish_run(&first.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    let first = store.get_run(&first.id).await.unwrap();
    let mut reply = message(&dot.id, "and then?");
    reply.parent_run_id = Some(first.id.clone());
    reply.root_run_id = Some(first.root_run_id.clone());
    reply.session_id = first.session_id.clone();
    reply.workspace_path = first.workspace_path.clone();
    reply.branch = first.branch.clone();
    let reply = running(&store, reply).await;

    let ids = store.recover_interrupted().await.unwrap();
    assert_eq!(ids.len(), 2);

    for old in [&fresh, &reply] {
        let failed = store.get_run(&old.id).await.unwrap();
        assert_eq!(failed.status, RunStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("interrupted"));
    }

    let again = resends_of(&store, &dot.id, &fresh.id).await;
    assert_eq!(again.len(), 1);
    let again = &again[0];
    assert_eq!(again.trigger, TriggerKind::Chat);
    assert_eq!(
        again.payload,
        Some(json!({ "message": "hello", "typed": true, "resent": "restart" }))
    );
    assert_eq!(again.root_run_id, fresh.root_run_id);
    assert_eq!(again.parent_run_id, None);
    // A new chat starts a new session, in the folder the interrupted run made: that folder is
    // not left behind where no review lists it.
    assert_eq!(again.session_id, None);
    assert_eq!(again.workspace_path.as_deref(), Some("/tmp/own"));
    assert_eq!(again.branch.as_deref(), Some("dots/own"));
    assert_eq!(again.base_commit.as_deref(), Some("abc"));

    let again = resends_of(&store, &dot.id, &reply.id).await;
    assert_eq!(again.len(), 1);
    let again = &again[0];
    assert_eq!(
        again.payload,
        Some(json!({ "message": "and then?", "typed": true, "resent": "restart" }))
    );
    assert_eq!(again.root_run_id, first.root_run_id);
    assert_eq!(again.parent_run_id.as_deref(), Some(first.id.as_str()));
    // A reply continues its topic from the parent, as `chat` does.
    assert_eq!(again.session_id.as_deref(), Some("topic-session"));
    assert_eq!(again.workspace_path.as_deref(), Some("/tmp/topic"));
    assert_eq!(again.branch.as_deref(), Some("dots/topic"));
    assert_eq!(again.base_commit.as_deref(), Some("def"));
}

#[tokio::test]
async fn other_triggers_are_not_resent() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    for trigger in [
        TriggerKind::Manual,
        TriggerKind::Schedule,
        TriggerKind::Webhook,
        TriggerKind::FollowUp,
        TriggerKind::Watcher,
        TriggerKind::Api,
    ] {
        running(&store, NewRun::new(&dot.id, trigger)).await;
    }
    assert_eq!(store.recover_interrupted().await.unwrap().len(), 6);
    assert!(store
        .list_runs_by_status(RunStatus::Queued)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_message_with_an_approved_call_is_not_resent() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    let run = running(&store, message(&dot.id, "send the mail")).await;
    let call = store
        .create_approval(&run.id, "Bash", &json!({"command": "send-mail"}))
        .await
        .unwrap();
    store.decide_approval(&call.id, true, None).await.unwrap();
    // A denied call ran nothing: alone it does not stop a resend.
    let other = running(&store, message(&dot.id, "maybe")).await;
    let denied = store
        .create_approval(&other.id, "Bash", &json!({"command": "rm"}))
        .await
        .unwrap();
    store
        .decide_approval(&denied.id, false, None)
        .await
        .unwrap();

    store.recover_interrupted().await.unwrap();
    let failed = store.get_run(&run.id).await.unwrap();
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("interrupted"));
    assert!(resends_of(&store, &dot.id, &run.id).await.is_empty());
    assert_eq!(resends_of(&store, &dot.id, &other.id).await.len(), 1);
}

#[tokio::test]
async fn resending_stops_after_three_tries() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    let mut current = running(&store, message(&dot.id, "again")).await;
    for _ in 0..3 {
        store.recover_interrupted().await.unwrap();
        let next = resends_of(&store, &dot.id, &current.id).await;
        assert_eq!(next.len(), 1);
        assert!(store.claim_run(&next[0].id).await.unwrap());
        current = next[0].clone();
    }
    // The third resend was interrupted too: no fourth.
    store.recover_interrupted().await.unwrap();
    assert!(resends_of(&store, &dot.id, &current.id).await.is_empty());
    let last = store.get_run(&current.id).await.unwrap();
    assert_eq!(last.status, RunStatus::Failed);
    assert_eq!(
        last.error.as_deref(),
        Some("interrupted; stopped resending after 3 tries")
    );
}

#[tokio::test]
async fn recovering_twice_resends_once() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    let run = running(&store, message(&dot.id, "once")).await;
    store.recover_interrupted().await.unwrap();
    assert!(store.recover_interrupted().await.unwrap().is_empty());
    assert_eq!(resends_of(&store, &dot.id, &run.id).await.len(), 1);
    assert_eq!(
        store
            .list_runs_by_status(RunStatus::Queued)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_message_that_called_a_tool_is_not_resent() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    // A call no one was asked about may have changed something: it could be done twice.
    let run = running(&store, message(&dot.id, "write it")).await;
    store
        .append_event(&run.id, "tool_call", &json!({"tool": "Bash"}))
        .await
        .unwrap();
    // Text alone changed nothing.
    let talk = running(&store, message(&dot.id, "say it")).await;
    store
        .append_event(&talk.id, "text", &json!({"text": "Sure."}))
        .await
        .unwrap();

    store.recover_interrupted().await.unwrap();
    let failed = store.get_run(&run.id).await.unwrap();
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("interrupted"));
    assert!(resends_of(&store, &dot.id, &run.id).await.is_empty());
    assert_eq!(resends_of(&store, &dot.id, &talk.id).await.len(), 1);
}

#[tokio::test]
async fn a_step_chat_is_not_resent() {
    let (dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    let run = running(&store, message(&dot.id, "for the step")).await;
    // Named a step without building a workflow: the step row itself does not matter here.
    let mut conn = SqliteConnectOptions::new()
        .filename(dir.path().join("dots.db"))
        .foreign_keys(false)
        .connect()
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET workflow_step_run_id = 'STEP' WHERE id = ?")
        .bind(&run.id)
        .execute(&mut conn)
        .await
        .unwrap();

    store.recover_interrupted().await.unwrap();
    assert_eq!(
        store.get_run(&run.id).await.unwrap().status,
        RunStatus::Failed
    );
    assert!(resends_of(&store, &dot.id, &run.id).await.is_empty());
}

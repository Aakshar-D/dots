mod common;

use common::{spec, temp_store};
use dots_core::model::{ApprovalStatus, NewRun, RunStatus, TriggerKind};
use dots_core::Error;
use serde_json::json;

#[tokio::test]
async fn create_run_defaults_and_payload() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let mut n = NewRun::new(&dot.id, TriggerKind::Webhook);
    n.payload = Some(json!({"k": [1, 2]}));
    let run = store.create_run(&n).await.unwrap();
    assert_eq!(run.root_run_id, run.id);
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.trigger, TriggerKind::Webhook);
    assert_eq!(run.payload, Some(json!({"k": [1, 2]})));
    assert!(run.started_at.is_none());
    assert_eq!(store.get_run(&run.id).await.unwrap(), run);
}

#[tokio::test]
async fn child_run_keeps_root_and_parent() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let parent = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let mut n = NewRun::new(&dot.id, TriggerKind::Resume);
    n.parent_run_id = Some(parent.id.clone());
    n.root_run_id = Some(parent.root_run_id.clone());
    n.session_id = Some("sess".into());
    let child = store.create_run(&n).await.unwrap();
    assert_eq!(child.root_run_id, parent.id);
    assert_eq!(child.parent_run_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.session_id.as_deref(), Some("sess"));
}

#[tokio::test]
async fn dispatchable_skips_dots_with_a_running_run_and_claim_is_exclusive() {
    let (_d, store) = temp_store().await;
    let a = store.create_dot(&spec("a")).await.unwrap();
    let b = store.create_dot(&spec("b")).await.unwrap();
    let a1 = store
        .create_run(&NewRun::new(&a.id, TriggerKind::Manual))
        .await
        .unwrap();
    let _a2 = store
        .create_run(&NewRun::new(&a.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store.claim_run(&a1.id).await.unwrap());
    assert!(!store.claim_run(&a1.id).await.unwrap());
    let b1 = store
        .create_run(&NewRun::new(&b.id, TriggerKind::Manual))
        .await
        .unwrap();
    let ids: Vec<String> = store
        .dispatchable_runs()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![b1.id]);
    let running = store.get_run(&a1.id).await.unwrap();
    assert_eq!(running.status, RunStatus::Running);
    assert!(running.started_at.is_some());
}

#[tokio::test]
async fn finish_workspace_session_usage() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.claim_run(&run.id).await.unwrap();
    store
        .set_workspace(&run.id, "C:/ws", Some("dots/a/x"), Some("abc123"))
        .await
        .unwrap();
    store.set_session(&run.id, "sess-1").await.unwrap();
    store.add_usage(&run.id, 10, 5).await.unwrap();
    store.add_usage(&run.id, 1, 1).await.unwrap();
    let done = store
        .finish_run(&run.id, RunStatus::Succeeded, Some("did it"), None)
        .await
        .unwrap();
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.summary.as_deref(), Some("did it"));
    assert_eq!(done.workspace_path.as_deref(), Some("C:/ws"));
    assert_eq!(done.branch.as_deref(), Some("dots/a/x"));
    assert_eq!(done.base_commit.as_deref(), Some("abc123"));
    assert_eq!(done.session_id.as_deref(), Some("sess-1"));
    assert_eq!((done.tokens_in, done.tokens_out), (11, 6));
    assert!(done.ended_at.is_some());
}

#[tokio::test]
async fn events_get_sequential_seq() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    for i in 0..3i64 {
        let e = store
            .append_event(&run.id, "assistant_text", &json!({"i": i}))
            .await
            .unwrap();
        assert_eq!(e.seq, i + 1);
    }
    let after_one = store.list_events(&run.id, 1).await.unwrap();
    assert_eq!(after_one.len(), 2);
    assert_eq!(after_one[0].seq, 2);
    assert_eq!(after_one[1].data, json!({"i": 2}));
}

#[tokio::test]
async fn recover_marks_running_as_interrupted() {
    let (_d, store) = temp_store().await;
    let a = store.create_dot(&spec("a")).await.unwrap();
    let b = store.create_dot(&spec("b")).await.unwrap();
    let running = store
        .create_run(&NewRun::new(&a.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.claim_run(&running.id).await.unwrap();
    let waiting = store
        .create_run(&NewRun::new(&b.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.claim_run(&waiting.id).await.unwrap();
    store
        .finish_run(&waiting.id, RunStatus::AwaitingApproval, None, None)
        .await
        .unwrap();
    let appr = store
        .create_approval(&waiting.id, "Bash", &json!({"command": "git push"}))
        .await
        .unwrap();

    let ids = store.recover_interrupted().await.unwrap();
    assert_eq!(ids, vec![running.id.clone()]);
    let r = store.get_run(&running.id).await.unwrap();
    assert_eq!(r.status, RunStatus::Failed);
    assert_eq!(r.error.as_deref(), Some("interrupted"));
    assert_eq!(
        store.get_run(&waiting.id).await.unwrap().status,
        RunStatus::AwaitingApproval
    );
    assert_eq!(
        store.get_approval(&appr.id).await.unwrap().status,
        ApprovalStatus::Pending
    );
}

#[tokio::test]
async fn approvals_decide_once() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let a = store
        .create_approval(&run.id, "Bash", &json!({"command": "git push"}))
        .await
        .unwrap();
    assert_eq!(a.status, ApprovalStatus::Pending);
    assert_eq!(a.input_hash.len(), 64);
    assert!(store.has_pending_for_run(&run.id).await.unwrap());
    assert_eq!(store.list_pending_approvals().await.unwrap().len(), 1);
    let d = store
        .decide_approval(&a.id, true, Some("ok"))
        .await
        .unwrap();
    assert_eq!(d.status, ApprovalStatus::Approved);
    assert_eq!(d.note.as_deref(), Some("ok"));
    assert!(d.decided_at.is_some());
    assert!(matches!(
        store.decide_approval(&a.id, false, None).await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        store.decide_approval("missing", true, None).await,
        Err(Error::NotFound(_))
    ));
    assert!(!store.has_pending_for_run(&run.id).await.unwrap());
}

#[tokio::test]
async fn parked_decisions_until_resolved_and_expiry() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let parked = store
        .create_approval(&run.id, "Bash", &json!({"command": "a"}))
        .await
        .unwrap();
    let live = store
        .create_approval(&run.id, "Bash", &json!({"command": "b"}))
        .await
        .unwrap();
    let pending = store
        .create_approval(&run.id, "Bash", &json!({"command": "c"}))
        .await
        .unwrap();
    store.mark_parked(&parked.id).await.unwrap();
    store
        .decide_approval(&parked.id, false, Some("use a branch"))
        .await
        .unwrap();
    store.decide_approval(&live.id, true, None).await.unwrap();
    let un = store.unresolved_parked_decisions(&run.id).await.unwrap();
    assert_eq!(un.len(), 1);
    assert_eq!(un[0].id, parked.id);
    assert!(un[0].parked);
    store.mark_resolved(&parked.id).await.unwrap();
    assert!(store
        .unresolved_parked_decisions(&run.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.expire_pending_for_run(&run.id).await.unwrap(), 1);
    assert_eq!(
        store.get_approval(&pending.id).await.unwrap().status,
        ApprovalStatus::Expired
    );
}

#[tokio::test]
async fn grants_are_single_use() {
    let (_d, store) = temp_store().await;
    assert!(!store.take_grant("root", "Bash", "h1").await.unwrap());
    store.create_grant("root", "Bash", "h1").await.unwrap();
    assert!(!store.take_grant("root", "Bash", "h2").await.unwrap());
    assert!(!store.take_grant("other", "Bash", "h1").await.unwrap());
    assert!(store.take_grant("root", "Bash", "h1").await.unwrap());
    assert!(!store.take_grant("root", "Bash", "h1").await.unwrap());
}

#[tokio::test]
async fn has_queued_run_and_list_runs() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    assert!(!store.has_queued_run(&dot.id).await.unwrap());
    let r1 = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    assert!(store.has_queued_run(&dot.id).await.unwrap());
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    let r2 = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let listed: Vec<String> = store
        .list_runs(Some(&dot.id), 10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(listed, vec![r2.id, r1.id]);
    assert_eq!(store.list_runs(None, 1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn cancel_if_inactive_only_touches_queued_or_awaiting() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("c")).await.unwrap();
    let queued = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let r = store.cancel_if_inactive(&queued.id).await.unwrap().unwrap();
    assert_eq!(r.status, RunStatus::Cancelled);
    assert!(r.ended_at.is_some());
    // Already terminal: nothing to do.
    assert!(store
        .cancel_if_inactive(&queued.id)
        .await
        .unwrap()
        .is_none());

    let running = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store.claim_run(&running.id).await.unwrap());
    assert!(store
        .cancel_if_inactive(&running.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_run(&running.id).await.unwrap().status,
        RunStatus::Running
    );

    store
        .finish_run(&running.id, RunStatus::AwaitingApproval, None, None)
        .await
        .unwrap();
    let r = store
        .cancel_if_inactive(&running.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.status, RunStatus::Cancelled);
}

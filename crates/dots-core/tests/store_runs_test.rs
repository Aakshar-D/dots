mod common;

use common::{spec, temp_store};
use dots_core::model::{ApplyState, Approval, ApprovalStatus, NewRun, Run, RunStatus, TriggerKind};
use dots_core::policy::{Action, Preset, Rule};
use dots_core::store::Store;
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
    let orphan = store
        .create_approval(&running.id, "Bash", &json!({"command": "git push"}))
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
    // The interrupted run can never act on its pending approval, so it is expired.
    let orphan = store.get_approval(&orphan.id).await.unwrap();
    assert_eq!(orphan.status, ApprovalStatus::Expired);
    assert!(orphan.decided_at.is_some());
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

#[tokio::test]
async fn the_lineage_index_exists() {
    let (dir, _store) = temp_store().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    let rows =
        sqlx::query("SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'runs_root'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
}

// ---- Approvals v2: calls that Dots performs itself ----

/// A parent run that parked with `n` approved calls, resumed: the calls are queued for Dots to
/// perform and the resume child exists. Returns (parent, child, approvals in the order asked).
async fn resumed_with_queued(store: &Store, name: &str, n: usize) -> (Run, Run, Vec<Approval>) {
    let dot = store.create_dot(&spec(name)).await.unwrap();
    let parent = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for i in 0..n {
        let a = store
            .create_approval_with(
                &parent.id,
                "Bash",
                &json!({"command": format!("step {i}")}),
                true,
            )
            .await
            .unwrap();
        store
            .decide_approval_with_parked(&a.id, true, None, true)
            .await
            .unwrap();
        ids.push(a.id);
    }
    store
        .set_status(&parent.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Resume);
    new.parent_run_id = Some(parent.id.clone());
    new.root_run_id = Some(parent.root_run_id.clone());
    let child = store
        .resume_parked(&parent.id, &[], &ids, &ids, Some(&new))
        .await
        .unwrap()
        .expect("child run");
    let mut approvals = Vec::new();
    for id in &ids {
        approvals.push(store.get_approval(id).await.unwrap());
    }
    (parent, child, approvals)
}

async fn apply_state(store: &Store, id: &str) -> Option<ApplyState> {
    store.get_approval(id).await.unwrap().apply_state
}

#[tokio::test]
async fn an_approval_records_whether_dots_performs_it() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let input = json!({"command": "git push"});
    let plain = store
        .create_approval(&run.id, "Bash", &input)
        .await
        .unwrap();
    assert!(!plain.self_apply);
    assert_eq!(plain.apply_state, None);
    assert_eq!(plain.applied_at, None);
    let own = store
        .create_approval_with(&run.id, "Bash", &input, true)
        .await
        .unwrap();
    assert!(own.self_apply);
    assert_eq!(own.apply_state, None, "nothing is queued before the resume");
    assert!(store.queued_applies(&run.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn resume_parked_queues_the_calls_in_the_order_they_were_asked() {
    let (_d, store) = temp_store().await;
    // Many calls made as fast as the store allows: several share a millisecond, so neither
    // `created_at` nor the ids order them. The order of insertion does.
    let (parent, child, approvals) = resumed_with_queued(&store, "a", 40).await;
    assert_eq!(child.parent_run_id.as_deref(), Some(parent.id.as_str()));
    let commands: Vec<String> = approvals
        .iter()
        .map(|a| a.input["command"].as_str().unwrap().to_string())
        .collect();
    let asked: Vec<String> = (0..40).map(|i| format!("step {i}")).collect();
    assert_eq!(commands, asked);
    for a in &approvals {
        assert_eq!(a.apply_state, Some(ApplyState::Queued));
        assert_eq!(a.status, ApprovalStatus::Approved);
        assert!(a.resolved);
        assert_eq!(a.applied_at, None);
    }
    let queued = store.queued_applies(&parent.id).await.unwrap();
    assert_eq!(queued, approvals);
    // Queued calls belong to the parent, not to the run that performs them.
    assert!(store.queued_applies(&child.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_queued_call_is_claimed_by_exactly_one_caller() {
    let (_d, store) = temp_store().await;
    let (_parent, _child, approvals) = resumed_with_queued(&store, "a", 1).await;
    let id = approvals[0].id.clone();
    let mut claims = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let id = id.clone();
        claims.push(tokio::spawn(async move {
            store.claim_apply(&id).await.unwrap()
        }));
    }
    let mut won = 0;
    for claim in claims {
        if claim.await.unwrap() {
            won += 1;
        }
    }
    assert_eq!(won, 1);
    assert_eq!(apply_state(&store, &id).await, Some(ApplyState::Applying));
    assert!(!store.claim_apply(&id).await.unwrap());
}

#[tokio::test]
async fn an_outcome_is_settled_once_and_only_for_a_claimed_call() {
    let (_d, store) = temp_store().await;
    let (_parent, _child, approvals) = resumed_with_queued(&store, "a", 2).await;
    let (first, second) = (&approvals[0].id, &approvals[1].id);
    // Not claimed: nothing to settle.
    assert!(!store
        .settle_apply(first, ApplyState::Applied)
        .await
        .unwrap());
    assert_eq!(apply_state(&store, first).await, Some(ApplyState::Queued));

    assert!(store.claim_apply(first).await.unwrap());
    assert!(store
        .settle_apply(first, ApplyState::Applied)
        .await
        .unwrap());
    let done = store.get_approval(first).await.unwrap();
    assert_eq!(done.apply_state, Some(ApplyState::Applied));
    assert!(done.applied_at.is_some());
    // A second outcome does not replace the first.
    assert!(!store.settle_apply(first, ApplyState::Failed).await.unwrap());
    assert_eq!(apply_state(&store, first).await, Some(ApplyState::Applied));
    // A settled call cannot be claimed again.
    assert!(!store.claim_apply(first).await.unwrap());

    assert!(store.claim_apply(second).await.unwrap());
    assert!(store
        .settle_apply(second, ApplyState::Failed)
        .await
        .unwrap());
    let failed = store.get_approval(second).await.unwrap();
    assert_eq!(failed.apply_state, Some(ApplyState::Failed));
    assert!(failed.applied_at.is_some());

    // Only the two outcomes of a performed call can be settled.
    for state in [
        ApplyState::Queued,
        ApplyState::Applying,
        ApplyState::Unknown,
        ApplyState::Skipped,
    ] {
        let err = store.settle_apply(second, state).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{state:?}: {err:?}");
    }
}

#[tokio::test]
async fn skip_and_close_end_what_was_not_performed() {
    let (_d, store) = temp_store().await;
    let (parent, _child, approvals) = resumed_with_queued(&store, "a", 4).await;
    let [done, claimed, waiting, given_up] = [0, 1, 2, 3].map(|i| approvals[i].id.clone());
    assert!(store.claim_apply(&done).await.unwrap());
    assert!(store
        .settle_apply(&done, ApplyState::Applied)
        .await
        .unwrap());
    assert!(store.claim_apply(&claimed).await.unwrap());
    assert!(store.skip_apply(&given_up).await.unwrap());
    // Skipping is for a queued call only.
    assert!(!store.skip_apply(&given_up).await.unwrap());
    assert!(!store.skip_apply(&claimed).await.unwrap());
    assert!(!store.skip_apply(&done).await.unwrap());

    let closed = store.close_applies(&parent.id).await.unwrap();
    let closed: Vec<(&str, Option<ApplyState>)> = closed
        .iter()
        .map(|a| (a.id.as_str(), a.apply_state))
        .collect();
    assert_eq!(
        closed,
        vec![
            (claimed.as_str(), Some(ApplyState::Unknown)),
            (waiting.as_str(), Some(ApplyState::Skipped)),
        ]
    );
    // What had an end keeps it.
    assert_eq!(apply_state(&store, &done).await, Some(ApplyState::Applied));
    assert_eq!(
        apply_state(&store, &given_up).await,
        Some(ApplyState::Skipped)
    );
    assert_eq!(store.get_approval(&claimed).await.unwrap().applied_at, None);
    // Nothing is left open, so a second close changes nothing.
    assert!(store.close_applies(&parent.id).await.unwrap().is_empty());
    for id in [&claimed, &waiting, &given_up] {
        assert!(!store.claim_apply(id).await.unwrap(), "{id} is closed");
    }
}

#[tokio::test]
async fn recover_closes_the_calls_of_an_interrupted_resume_run() {
    let (_d, store) = temp_store().await;
    // A resume run that was performing its parent's calls when the process ended.
    let (_p1, interrupted, calls) = resumed_with_queued(&store, "a", 3).await;
    store.claim_run(&interrupted.id).await.unwrap();
    assert!(store.claim_apply(&calls[0].id).await.unwrap());
    assert!(store
        .settle_apply(&calls[0].id, ApplyState::Applied)
        .await
        .unwrap());
    assert!(store.claim_apply(&calls[1].id).await.unwrap());
    // A resume run that had not started: its calls are still to be performed.
    let (_p2, waiting, later) = resumed_with_queued(&store, "b", 1).await;
    // A plain approval of another dot's run, never performed by Dots.
    let other = store.create_dot(&spec("c")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&other.id, TriggerKind::Manual))
        .await
        .unwrap();
    let plain = store
        .create_approval(&run.id, "Bash", &json!({"command": "x"}))
        .await
        .unwrap();

    let ids = store.recover_interrupted().await.unwrap();
    assert_eq!(ids, vec![interrupted.id.clone()]);
    assert_eq!(
        apply_state(&store, &calls[0].id).await,
        Some(ApplyState::Applied)
    );
    assert_eq!(
        apply_state(&store, &calls[1].id).await,
        Some(ApplyState::Unknown)
    );
    assert_eq!(
        apply_state(&store, &calls[2].id).await,
        Some(ApplyState::Skipped)
    );
    assert_eq!(
        apply_state(&store, &later[0].id).await,
        Some(ApplyState::Queued)
    );
    assert_eq!(
        store.get_run(&waiting.id).await.unwrap().status,
        RunStatus::Queued
    );
    assert_eq!(apply_state(&store, &plain.id).await, None);
}

#[tokio::test]
async fn recover_leaves_no_call_claimed() {
    let (_d, store) = temp_store().await;
    // Claimed, but its resume run already ended (the runner had no chance to close it).
    let (_parent, child, calls) = resumed_with_queued(&store, "a", 1).await;
    store.claim_run(&child.id).await.unwrap();
    assert!(store.claim_apply(&calls[0].id).await.unwrap());
    store
        .finish_run(&child.id, RunStatus::Failed, None, Some("boom"))
        .await
        .unwrap();
    assert!(store.recover_interrupted().await.unwrap().is_empty());
    assert_eq!(
        apply_state(&store, &calls[0].id).await,
        Some(ApplyState::Unknown)
    );
}

#[tokio::test]
async fn approvals_made_before_the_migration_are_not_self_applied() {
    let (dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    // A row written without the new columns, as before migration 0004: they take their defaults.
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO approvals (id, run_id, tool, input, input_hash, status, created_at) \
         VALUES ('old', ?, 'Bash', '{}', 'h', 'pending', '2026-10-01T00:00:00.000Z')",
    )
    .bind(&run.id)
    .execute(&pool)
    .await
    .unwrap();
    let old = store.get_approval("old").await.unwrap();
    assert!(!old.self_apply);
    assert_eq!(old.apply_state, None);
    assert_eq!(old.applied_at, None);
}

// ---- Approvals v2: a rule saved with an approval ----

#[tokio::test]
async fn approving_with_a_rule_writes_the_decision_and_the_policy_together() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let push = json!({"command": "git push"});
    let first = store.create_approval(&run.id, "Bash", &push).await.unwrap();
    let second = store.create_approval(&run.id, "Bash", &push).await.unwrap();
    let before = dot.spec.policy.clone();
    assert_eq!(before.preset, Preset::Sandboxed);
    let rule = Rule::new("Bash(git push)", Action::Allow);

    let approved = store
        .approve_with_rule(&first.id, Some("ok"), true, &rule)
        .await
        .unwrap();
    assert_eq!(approved.status, ApprovalStatus::Approved);
    assert_eq!(approved.note.as_deref(), Some("ok"));
    assert!(approved.parked);
    let saved = store.get_dot(&dot.id).await.unwrap();
    let mut expected = before.clone();
    expected.rules.push(rule.clone());
    expected.preset = Preset::Custom;
    assert_eq!(saved.spec.policy, expected);

    // The same rule with another approval: the decision is recorded, the dot is not touched.
    let again = store
        .approve_with_rule(&second.id, None, false, &rule)
        .await
        .unwrap();
    assert_eq!(again.status, ApprovalStatus::Approved);
    assert!(!again.parked);
    let unchanged = store.get_dot(&dot.id).await.unwrap();
    assert_eq!(unchanged, saved);

    // An approval that is no longer pending: a conflict, and neither row changes.
    let other = Rule::new("Bash(git status)", Action::Allow);
    let err = store
        .approve_with_rule(&first.id, Some("again"), false, &other)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), saved);
    assert_eq!(store.get_approval(&first.id).await.unwrap(), approved);
    let err = store
        .approve_with_rule("no-such-approval", None, true, &other)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), saved);
}

#[tokio::test]
async fn a_rule_the_policy_already_holds_keeps_the_preset() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let a = store
        .create_approval(&run.id, "Read", &json!({"file_path": "a.txt"}))
        .await
        .unwrap();
    // The sandboxed preset already allows `Read`.
    let held = Rule::new("Read", Action::Allow);
    assert!(dot.spec.policy.rules.contains(&held));
    store
        .approve_with_rule(&a.id, None, true, &held)
        .await
        .unwrap();
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);
}

#[tokio::test]
async fn pending_approvals_of_a_run_come_in_the_order_they_were_asked() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let other = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let mut asked = Vec::new();
    for i in 0..30 {
        let a = store
            .create_approval(&run.id, "Bash", &json!({"command": format!("step {i}")}))
            .await
            .unwrap();
        asked.push(a.id);
    }
    store
        .create_approval(&other.id, "Bash", &json!({"command": "elsewhere"}))
        .await
        .unwrap();
    store.decide_approval(&asked[3], true, None).await.unwrap();
    store.decide_approval(&asked[7], false, None).await.unwrap();
    let pending: Vec<String> = store
        .pending_for_run(&run.id)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    let expected: Vec<String> = asked
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 3 && *i != 7)
        .map(|(_, id)| id.clone())
        .collect();
    assert_eq!(pending, expected);
    // The Inbox's list is in the same order, with the other run's call where it was asked.
    let listed: Vec<String> = store
        .list_pending_approvals()
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.run_id == run.id)
        .map(|a| a.id)
        .collect();
    assert_eq!(listed, expected);
    assert_eq!(store.list_pending_approvals().await.unwrap().len(), 29);
}

#[tokio::test]
async fn recover_skips_the_calls_a_resume_run_left_queued_when_it_ended() {
    let (_d, store) = temp_store().await;
    // Resume runs whose final status was written, but whose calls were never closed (the
    // process ended in between, or the close failed): nothing will perform those calls.
    let (_p1, succeeded, left1) = resumed_with_queued(&store, "a", 2).await;
    store.claim_run(&succeeded.id).await.unwrap();
    assert!(store.claim_apply(&left1[0].id).await.unwrap());
    assert!(store
        .settle_apply(&left1[0].id, ApplyState::Applied)
        .await
        .unwrap());
    store
        .finish_run(&succeeded.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    let (_p2, parked, left2) = resumed_with_queued(&store, "b", 1).await;
    store.claim_run(&parked.id).await.unwrap();
    store
        .set_status(&parked.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    // A resume run that has not started yet: its calls are still to be performed.
    let (_p3, waiting, later) = resumed_with_queued(&store, "c", 2).await;

    assert!(store.recover_interrupted().await.unwrap().is_empty());
    assert_eq!(
        apply_state(&store, &left1[0].id).await,
        Some(ApplyState::Applied)
    );
    assert_eq!(
        apply_state(&store, &left1[1].id).await,
        Some(ApplyState::Skipped)
    );
    assert_eq!(
        apply_state(&store, &left2[0].id).await,
        Some(ApplyState::Skipped)
    );
    for call in &later {
        assert_eq!(
            apply_state(&store, &call.id).await,
            Some(ApplyState::Queued)
        );
    }
    assert_eq!(
        store.get_run(&waiting.id).await.unwrap().status,
        RunStatus::Queued
    );
}

#[tokio::test]
async fn resume_parked_refuses_to_queue_a_call_dots_is_not_to_perform() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let parent = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let ask = |self_apply: bool| {
        let store = store.clone();
        let run = parent.id.clone();
        async move {
            store
                .create_approval_with(&run, "Bash", &json!({"command": "x"}), self_apply)
                .await
                .unwrap()
        }
    };
    let denied = ask(true).await;
    store
        .decide_approval_with_parked(&denied.id, false, None, true)
        .await
        .unwrap();
    let granted = ask(false).await;
    store
        .decide_approval_with_parked(&granted.id, true, None, true)
        .await
        .unwrap();
    let own = ask(true).await;
    store
        .decide_approval_with_parked(&own.id, true, None, true)
        .await
        .unwrap();
    // An approved call of another run.
    let other_run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let foreign = store
        .create_approval_with(&other_run.id, "Bash", &json!({"command": "y"}), true)
        .await
        .unwrap();
    store
        .decide_approval_with_parked(&foreign.id, true, None, true)
        .await
        .unwrap();
    store
        .set_status(&parent.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();

    let all = vec![
        denied.id.clone(),
        granted.id.clone(),
        own.id.clone(),
        foreign.id.clone(),
    ];
    for bad in [&denied.id, &granted.id, &foreign.id] {
        let mut new = NewRun::new(&dot.id, TriggerKind::Resume);
        new.parent_run_id = Some(parent.id.clone());
        new.root_run_id = Some(parent.root_run_id.clone());
        let err = store
            .resume_parked(
                &parent.id,
                &[("Bash".into(), "k".into())],
                &all,
                &[own.id.clone(), bad.clone()],
                Some(&new),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{bad}: {err:?}");
        // Nothing was written: the parent still waits, no child, nothing resolved or queued.
        assert_eq!(
            store.get_run(&parent.id).await.unwrap().status,
            RunStatus::AwaitingApproval
        );
        assert_eq!(
            store.list_thread(&parent.root_run_id).await.unwrap().len(),
            1
        );
        for id in &all {
            let a = store.get_approval(id).await.unwrap();
            assert!(!a.resolved, "{id} resolved");
            assert_eq!(a.apply_state, None, "{id} queued");
        }
    }
    // The call Dots is to perform is queued alone.
    let mut new = NewRun::new(&dot.id, TriggerKind::Resume);
    new.parent_run_id = Some(parent.id.clone());
    new.root_run_id = Some(parent.root_run_id.clone());
    let child = store
        .resume_parked(
            &parent.id,
            &[],
            &all,
            std::slice::from_ref(&own.id),
            Some(&new),
        )
        .await
        .unwrap();
    assert!(child.is_some());
    assert_eq!(apply_state(&store, &own.id).await, Some(ApplyState::Queued));
}

/// A failed run of `trigger` for `dot`, retrying `retry_of`; with a `tool_call` event when
/// `called` is set.
async fn failed_run(
    store: &Store,
    dot: &str,
    trigger: TriggerKind,
    retry_of: Option<&str>,
    called: bool,
) -> Run {
    let mut new = NewRun::new(dot, trigger);
    new.retry_of = retry_of.map(str::to_string);
    let run = store.create_run(&new).await.unwrap();
    if called {
        let call = json!({"kind": "tool_call", "id": "c1", "tool": "Bash", "input": {}});
        store
            .append_event(&run.id, "tool_call", &call)
            .await
            .unwrap();
    }
    store
        .finish_run(&run.id, RunStatus::Failed, None, Some("down"))
        .await
        .unwrap()
}

/// A failed run is a run to review whatever its trigger and whether it called a tool, until
/// it is marked reviewed. The scheduler marks the failed runs it judges into a retry or into
/// "Needs attention" (`ScheduleChange::reviewed_run`), so a failed scheduled run that is still
/// listed is one it never judged, such as a run that a later slot overran.
#[tokio::test]
async fn a_failed_run_is_for_review_whatever_its_trigger() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let quiet = failed_run(&store, &dot.id, TriggerKind::Schedule, None, false).await;
    let retry = failed_run(
        &store,
        &dot.id,
        TriggerKind::Schedule,
        Some(&quiet.id),
        false,
    )
    .await;
    let called = failed_run(&store, &dot.id, TriggerKind::Schedule, None, true).await;
    let manual = failed_run(&store, &dot.id, TriggerKind::Manual, None, false).await;
    let hook = failed_run(&store, &dot.id, TriggerKind::Webhook, None, false).await;

    let mut ids: Vec<String> = store
        .unreviewed_thread_ends()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    ids.sort();
    let mut want = vec![quiet.id, retry.id, called.id, manual.id, hook.id];
    want.sort();
    assert_eq!(ids, want);
}

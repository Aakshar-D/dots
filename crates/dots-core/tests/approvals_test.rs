mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::approvals::{grant_key, parked_message, ApprovalHub, DecideEffect};
use dots_core::engine::{PermissionGate, PermissionOutcome};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::model::{ApprovalStatus, NewRun, Run, TriggerKind};
use dots_core::policy::{Policy, Preset};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn setup(
    preset: Preset,
    wait_secs: u64,
) -> (
    tempfile::TempDir,
    Store,
    Arc<ApprovalHub>,
    Run,
    dots_core::events::Bus,
) {
    let (dir, store) = temp_store().await;
    let mut s = spec("gate");
    s.policy = Policy::preset(preset);
    s.approval_wait_secs = wait_secs;
    let dot = store.create_dot(&s).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    (dir, store, hub, run, bus)
}

#[tokio::test]
async fn allow_and_deny_follow_policy_without_approvals() {
    let (_d, store, hub, run, _bus) = setup(Preset::ReadOnly, 5).await;
    let read = hub
        .check(&run.id, "Read", json!({"file_path": "a"}))
        .await
        .unwrap();
    assert_eq!(
        read,
        PermissionOutcome::Allow {
            input: json!({"file_path": "a"})
        }
    );
    let bash = hub
        .check(&run.id, "Bash", json!({"command": "ls"}))
        .await
        .unwrap();
    assert!(matches!(bash, PermissionOutcome::Deny { ref message } if message.contains("'gate'")));
    assert!(store.approvals_for_run(&run.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn ask_approved_within_window_is_live() {
    let (_d, store, hub, run, bus) = setup(Preset::Sandboxed, 5).await;
    let mut events = bus.subscribe();
    let h = hub.clone();
    let rid = run.id.clone();
    let check =
        tokio::spawn(async move { h.check(&rid, "Bash", json!({"command": "git push"})).await });
    let approval = loop {
        if let RuntimeEvent::ApprovalRequested { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    let effect = hub.decide(&approval.id, true, None).await.unwrap();
    assert!(matches!(effect, DecideEffect::Live(_)));
    let outcome = check.await.unwrap().unwrap();
    assert_eq!(
        outcome,
        PermissionOutcome::Allow {
            input: json!({"command": "git push"})
        }
    );
    let stored = store.get_approval(&approval.id).await.unwrap();
    assert_eq!(stored.status, ApprovalStatus::Approved);
    assert!(!stored.parked);
}

#[tokio::test]
async fn ask_denied_with_note_returns_note() {
    let (_d, _store, hub, run, bus) = setup(Preset::Sandboxed, 5).await;
    let mut events = bus.subscribe();
    let h = hub.clone();
    let rid = run.id.clone();
    let check =
        tokio::spawn(async move { h.check(&rid, "Bash", json!({"command": "git push"})).await });
    let approval = loop {
        if let RuntimeEvent::ApprovalRequested { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    hub.decide(&approval.id, false, Some("open a PR instead".into()))
        .await
        .unwrap();
    let outcome = check.await.unwrap().unwrap();
    assert_eq!(
        outcome,
        PermissionOutcome::Deny {
            message: "Denied by the reviewer: open a PR instead".into()
        }
    );
}

#[tokio::test]
async fn decision_after_window_is_parked() {
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    let outcome = hub
        .check(&run.id, "Bash", json!({"command": "git push"}))
        .await
        .unwrap();
    let pending = store.approvals_for_run(&run.id).await.unwrap();
    assert_eq!(pending.len(), 1);
    let a = &pending[0];
    assert_eq!(
        outcome,
        PermissionOutcome::Deny {
            message: parked_message(&a.id)
        }
    );
    assert_eq!(a.status, ApprovalStatus::Pending);
    assert!(a.parked);
    let effect = hub.decide(&a.id, true, None).await.unwrap();
    assert!(matches!(effect, DecideEffect::Parked(ref p) if p.status == ApprovalStatus::Approved));
}

#[tokio::test]
async fn deciding_twice_conflicts() {
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    hub.check(&run.id, "Bash", json!({"command": "git push"}))
        .await
        .unwrap();
    let id = store.approvals_for_run(&run.id).await.unwrap()[0]
        .id
        .clone();
    hub.decide(&id, false, None).await.unwrap();
    assert!(matches!(
        hub.decide(&id, true, None).await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        hub.decide("missing", true, None).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn grant_allows_exactly_once_and_ignores_description() {
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    let first = json!({"command": "git push", "description": "Push branch"});
    let retry = json!({"command": "git push", "description": "Push the work"});
    assert_eq!(grant_key("Bash", &first), grant_key("Bash", &retry));
    store
        .create_grant(&run.root_run_id, "Bash", &grant_key("Bash", &first))
        .await
        .unwrap();
    let allowed = hub.check(&run.id, "Bash", retry.clone()).await.unwrap();
    assert_eq!(
        allowed,
        PermissionOutcome::Allow {
            input: retry.clone()
        }
    );
    let again = hub.check(&run.id, "Bash", retry).await.unwrap();
    assert!(matches!(again, PermissionOutcome::Deny { .. }));
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn waiter_dropped_before_decision_is_parked_on_decide() {
    let (_d, store, hub, run, bus) = setup(Preset::Sandboxed, 30).await;
    let mut events = bus.subscribe();
    let h = hub.clone();
    let rid = run.id.clone();
    let check =
        tokio::spawn(async move { h.check(&rid, "Bash", json!({"command": "git push"})).await });
    let approval = loop {
        if let RuntimeEvent::ApprovalRequested { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    check.abort(); // e.g. the CLI's HTTP request went away
    tokio::time::sleep(Duration::from_millis(50)).await;
    let effect = hub.decide(&approval.id, true, None).await.unwrap();
    assert!(matches!(effect, DecideEffect::Parked(_)));
    assert!(store.get_approval(&approval.id).await.unwrap().parked);
}

#[tokio::test]
async fn policy_resolution_uses_the_run_workspace() {
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    store
        .set_workspace(&run.id, "C:/ws/run1", None, None)
        .await
        .unwrap();
    let inside = hub
        .check(
            &run.id,
            "Write",
            json!({"file_path": "C:/ws/run1/src/a.rs"}),
        )
        .await
        .unwrap();
    assert_eq!(
        inside,
        PermissionOutcome::Allow {
            input: json!({"file_path": "C:/ws/run1/src/a.rs"})
        }
    );
    let outside = hub
        .check(
            &run.id,
            "Write",
            json!({"file_path": "C:/Users/u/.gitconfig"}),
        )
        .await
        .unwrap();
    let approvals = store.approvals_for_run(&run.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert!(approvals[0].parked);
    assert_eq!(
        outside,
        PermissionOutcome::Deny {
            message: parked_message(&approvals[0].id)
        }
    );
}

#[test]
fn grant_key_command_shortcut_is_limited_to_shell_tools() {
    let staging = json!({"command": "deploy", "env": "staging"});
    let prod = json!({"command": "deploy", "env": "prod"});
    assert_ne!(
        grant_key("mcp__x__run", &staging),
        grant_key("mcp__x__run", &prod)
    );
    for tool in ["Bash", "PowerShell"] {
        let a = json!({"command": "git push", "description": "one"});
        let b = json!({"command": "git push", "description": "two"});
        assert_eq!(grant_key(tool, &a), grant_key(tool, &b));
    }
}

#[tokio::test]
async fn aborted_check_removes_its_waiter() {
    let (_d, _store, hub, run, bus) = setup(Preset::Sandboxed, 30).await;
    let mut events = bus.subscribe();
    let h = hub.clone();
    let rid = run.id.clone();
    let check =
        tokio::spawn(async move { h.check(&rid, "Bash", json!({"command": "git push"})).await });
    loop {
        if let RuntimeEvent::ApprovalRequested { .. } = events.recv().await.unwrap() {
            break;
        }
    }
    assert_eq!(hub.waiter_count(), 1);
    check.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(hub.waiter_count(), 0);
}

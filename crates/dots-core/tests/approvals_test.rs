mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::approvals::{grant_key, parked_message, ApprovalHub, DecideEffect};
use dots_core::engine::{PermissionGate, PermissionOutcome};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::model::{
    ApplyState, ApprovalStatus, EngineKind, NewRun, Run, RunStatus, TriggerKind,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
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

// ---- final review: I2 ----

#[tokio::test]
async fn decide_with_parked_writes_decision_and_flag_in_one_update() {
    let (_d, store, _hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    let a = store
        .create_approval(&run.id, "Bash", &json!({"command": "git push"}))
        .await
        .unwrap();
    let decided = store
        .decide_approval_with_parked(&a.id, true, Some("ok"), true)
        .await
        .unwrap();
    assert_eq!(decided.status, ApprovalStatus::Approved);
    assert_eq!(decided.note.as_deref(), Some("ok"));
    assert!(decided.decided_at.is_some());
    assert!(decided.parked);
    let again = store
        .decide_approval_with_parked(&a.id, false, None, false)
        .await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
    let stored = store.get_approval(&a.id).await.unwrap();
    assert_eq!(stored.status, ApprovalStatus::Approved);
    assert!(stored.parked);

    let b = store
        .create_approval(&run.id, "Bash", &json!({"command": "git push -f"}))
        .await
        .unwrap();
    let live = store
        .decide_approval_with_parked(&b.id, false, None, false)
        .await
        .unwrap();
    assert_eq!(live.status, ApprovalStatus::Denied);
    assert!(!live.parked);
}

#[tokio::test]
async fn decision_without_waiter_is_parked_before_it_is_announced() {
    let (_d, store, hub, run, bus) = setup(Preset::Sandboxed, 0).await;
    let a = store
        .create_approval(&run.id, "Bash", &json!({"command": "git push"}))
        .await
        .unwrap();
    let mut events = bus.subscribe();
    let effect = hub.decide(&a.id, true, None).await.unwrap();
    let DecideEffect::Parked(decided) = effect else {
        panic!("{effect:?}")
    };
    assert!(decided.parked);
    let announced = loop {
        if let RuntimeEvent::ApprovalDecided { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    assert!(announced.parked, "{announced:?}");
    assert_eq!(announced.status, ApprovalStatus::Approved);
}

// ---- Approvals v2: calls that Dots performs itself ----

#[tokio::test]
async fn an_approval_says_whether_dots_will_perform_its_call() {
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    let ask = |n: u32| json!({"command": format!("git push {n}")});
    // No engine performs calls itself: a late decision becomes a grant, as before.
    hub.check(&run.id, "Bash", ask(1)).await.unwrap();
    // The dot of `setup` is a Claude dot. An engine of another kind changes nothing for it.
    hub.set_self_apply([EngineKind::Local]);
    hub.check(&run.id, "Bash", ask(2)).await.unwrap();
    hub.set_self_apply([EngineKind::Claude, EngineKind::Local]);
    hub.check(&run.id, "Bash", ask(3)).await.unwrap();
    // Taking the engine out again does not touch the approval made meanwhile.
    hub.set_self_apply(Vec::<EngineKind>::new());
    hub.check(&run.id, "Bash", ask(4)).await.unwrap();

    let mut flags: Vec<(String, bool)> = store
        .approvals_for_run(&run.id)
        .await
        .unwrap()
        .into_iter()
        .map(|a| {
            (
                a.input["command"].as_str().unwrap().to_string(),
                a.self_apply,
            )
        })
        .collect();
    flags.sort();
    assert_eq!(
        flags,
        vec![
            ("git push 1".to_string(), false),
            ("git push 2".to_string(), false),
            ("git push 3".to_string(), true),
            ("git push 4".to_string(), false),
        ]
    );
}

#[tokio::test]
async fn a_rule_is_saved_with_an_approval_and_refused_with_a_denial() {
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
    let rule = Rule::new("Bash(git push)", Action::Allow);
    // Refused before anything is touched: the engine still waits and can still be answered.
    let err = hub
        .decide_with_rule(&approval.id, false, None, Some(rule.clone()))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert_eq!(hub.waiter_count(), 1);
    assert_eq!(
        store.get_approval(&approval.id).await.unwrap().status,
        ApprovalStatus::Pending
    );

    let effect = hub
        .decide_with_rule(&approval.id, true, None, Some(rule.clone()))
        .await
        .unwrap();
    assert!(matches!(effect, DecideEffect::Live(_)));
    assert!(matches!(
        check.await.unwrap().unwrap(),
        PermissionOutcome::Allow { .. }
    ));
    let policy = store.get_dot(&run.dot_id).await.unwrap().spec.policy;
    assert_eq!(policy.rules.last(), Some(&rule));
    assert_eq!(policy.preset, Preset::Custom);
    // The rule holds for the next call at once: no question, no approval row.
    let again = hub
        .check(&run.id, "Bash", json!({"command": "git push"}))
        .await
        .unwrap();
    assert!(matches!(again, PermissionOutcome::Allow { .. }));
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);
}

/// An approved, parked approval of `run` whose call Dots is to perform.
async fn approved_call(store: &Store, run: &Run, command: &str) -> String {
    let a = store
        .create_approval_with(&run.id, "Bash", &json!({ "command": command }), true)
        .await
        .unwrap();
    store
        .decide_approval_with_parked(&a.id, true, None, true)
        .await
        .unwrap();
    a.id
}

#[tokio::test]
async fn claiming_settling_and_skipping_a_call_are_each_announced_once() {
    let (_d, store, hub, run, bus) = setup(Preset::Sandboxed, 0).await;
    let first = approved_call(&store, &run, "one").await;
    let second = approved_call(&store, &run, "two").await;
    store
        .set_status(&run.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    let ids = vec![first.clone(), second.clone()];
    store
        .resume_parked(&run.id, &[], &ids, &ids, None)
        .await
        .unwrap();
    let mut events = bus.subscribe();
    // What was announced since the last look: the hub sends before it returns.
    let mut updates = move || {
        let mut seen = Vec::new();
        while let Ok(ev) = events.try_recv() {
            match ev {
                RuntimeEvent::ApprovalUpdated { approval } => {
                    seen.push((approval.id, approval.apply_state))
                }
                other => panic!("unexpected event {other:?}"),
            }
        }
        seen
    };

    assert!(hub.claim_apply(&first).await.unwrap());
    assert_eq!(updates(), vec![(first.clone(), Some(ApplyState::Applying))]);
    // A claim that is lost changes nothing and announces nothing.
    assert!(!hub.claim_apply(&first).await.unwrap());
    assert!(!hub.claim_apply("no-such-approval").await.unwrap());
    assert!(updates().is_empty());

    hub.settle_apply(&first, ApplyState::Applied).await.unwrap();
    assert_eq!(updates(), vec![(first.clone(), Some(ApplyState::Applied))]);
    assert!(store
        .get_approval(&first)
        .await
        .unwrap()
        .applied_at
        .is_some());
    // An outcome is written once, and only for a call that was claimed.
    for id in [&first, &second] {
        let err = hub.settle_apply(id, ApplyState::Failed).await.unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    }
    assert!(updates().is_empty());

    assert!(hub.skip_apply(&second).await.unwrap());
    assert_eq!(updates(), vec![(second.clone(), Some(ApplyState::Skipped))]);
    assert!(!hub.skip_apply(&second).await.unwrap());
    assert!(!hub.claim_apply(&second).await.unwrap());
    assert!(updates().is_empty());
}

/// A Claude-engine retry of an approved MCP call takes its one-shot grant only under the
/// server's accepted fingerprint it was approved with (MCP/skills spec section 4).
#[tokio::test]
async fn an_mcp_grant_allows_only_under_the_fingerprint_it_was_approved_with() {
    use dots_core::policy::McpTrust;
    let (_d, store, hub, run, _bus) = setup(Preset::Sandboxed, 0).await;
    let tool = "mcp__files__write";
    let input = json!({"path": "a.txt"});
    let writes = McpTrust {
        held: false,
        read_only: false,
    };
    let check = |fingerprint: &str| {
        hub.check_mcp(
            &run.id,
            tool,
            input.clone(),
            writes,
            Some(fingerprint.to_string()),
        )
    };
    store
        .create_mcp_grant(
            &run.root_run_id,
            tool,
            &grant_key(tool, &input),
            Some("fp1"),
        )
        .await
        .unwrap();
    assert_eq!(
        check("fp1").await.unwrap(),
        PermissionOutcome::Allow {
            input: input.clone()
        }
    );
    // Used up: the next call asks, and asking parks at once.
    assert!(matches!(
        check("fp1").await.unwrap(),
        PermissionOutcome::Deny { .. }
    ));
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);

    store
        .create_mcp_grant(
            &run.root_run_id,
            tool,
            &grant_key(tool, &input),
            Some("fp1"),
        )
        .await
        .unwrap();
    assert_eq!(
        check("fp2").await.unwrap(),
        PermissionOutcome::Deny {
            message: "mcp__files__write: not performed, because the tool changed since it was \
                      approved."
                .into()
        }
    );
    // That grant is used up too, and no approval was asked for.
    assert!(!store
        .take_grant(&run.root_run_id, tool, &grant_key(tool, &input))
        .await
        .unwrap());
    assert_eq!(store.approvals_for_run(&run.id).await.unwrap().len(), 1);
}

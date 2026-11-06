//! The person's goals (Intents): the runtime lists, adds, changes, completes and deletes them
//! with plain sentences for what it refuses, publishes `intents_updated`, keeps each goal in
//! the account and workspace it was added in, and a deleted dot leaves its goals unlinked.

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::intents::{IntentFields, BAD_DATE, GONE, NO_DOT, OTHER_WORKSPACE, TEXT_RULE};
use dots_core::{Config, Error, Runtime};
use tempfile::TempDir;

const PERSONAL: &str = "personal";
const DEFAULT: &str = "default";

struct Rt {
    _dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { _dir: dir, rt }
}

fn refusal(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) | Error::NotAllowed(m) => m,
        other => other.to_string(),
    }
}

fn goal(text: &str) -> IntentFields {
    IntentFields {
        text: text.into(),
        due_date: None,
        urgent: false,
        dot_id: None,
    }
}

/// Whether an `intents_updated` arrived.
fn heard(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> bool {
    let mut any = false;
    while let Ok(e) = events.try_recv() {
        any |= matches!(e, RuntimeEvent::IntentsUpdated { .. });
    }
    any
}

#[tokio::test]
async fn the_person_adds_changes_and_deletes_goals() {
    let t = start().await;
    let dot = t.rt.create_dot(spec("planner"), None, None).await.unwrap();
    let mut events = t.rt.subscribe();

    let added =
        t.rt.intent_create(
            PERSONAL,
            DEFAULT,
            IntentFields {
                text: "  Ship the spring report  ".into(),
                due_date: Some("2026-11-12".into()),
                urgent: true,
                dot_id: Some(dot.id.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(added.text, "Ship the spring report");
    assert_eq!(added.due_date.as_deref(), Some("2026-11-12"));
    assert!(added.urgent);
    assert_eq!(added.dot_id.as_deref(), Some(dot.id.as_str()));
    assert_eq!(added.org_id, None);
    assert_eq!(added.workspace_id, DEFAULT);
    assert_eq!(added.done_at, None);
    assert!(heard(&mut events));

    let changed =
        t.rt.intent_update(&added.id, goal("Ship the autumn report"))
            .await
            .unwrap();
    assert_eq!(changed.text, "Ship the autumn report");
    assert_eq!(changed.due_date, None);
    assert!(!changed.urgent);
    assert_eq!(changed.dot_id, None);
    assert_eq!(changed.created_at, added.created_at);
    assert!(heard(&mut events));

    let view = t.rt.intents_list(PERSONAL, DEFAULT).await.unwrap();
    assert_eq!(view.intents, vec![changed.clone()]);
    assert_eq!(view.refusal, None);

    t.rt.intent_delete(&added.id).await.unwrap();
    assert!(heard(&mut events));
    assert!(t
        .rt
        .intents_list(PERSONAL, DEFAULT)
        .await
        .unwrap()
        .intents
        .is_empty());
    assert_eq!(
        refusal(t.rt.intent_delete(&added.id).await.unwrap_err()),
        GONE
    );
    assert_eq!(
        refusal(
            t.rt.intent_update(&added.id, goal("again"))
                .await
                .unwrap_err()
        ),
        GONE
    );
}

#[tokio::test]
async fn a_goal_is_refused_with_a_plain_sentence() {
    let t = start().await;
    let long = "x".repeat(501);
    for (fields, sentence) in [
        (goal(""), TEXT_RULE),
        (goal("   "), TEXT_RULE),
        (goal(&long), TEXT_RULE),
        (
            IntentFields {
                due_date: Some("next friday".into()),
                ..goal("Book the venue")
            },
            BAD_DATE,
        ),
        (
            IntentFields {
                due_date: Some("2026-02-30".into()),
                ..goal("Book the venue")
            },
            BAD_DATE,
        ),
        (
            IntentFields {
                dot_id: Some("01NOSUCHDOT".into()),
                ..goal("Book the venue")
            },
            NO_DOT,
        ),
    ] {
        assert_eq!(
            refusal(
                t.rt.intent_create(PERSONAL, DEFAULT, fields)
                    .await
                    .unwrap_err()
            ),
            sentence
        );
    }
    // 500 characters is the most; a blank date is no date.
    let most = "y".repeat(500);
    let ok =
        t.rt.intent_create(
            PERSONAL,
            DEFAULT,
            IntentFields {
                due_date: Some("  ".into()),
                ..goal(&most)
            },
        )
        .await
        .unwrap();
    assert_eq!(ok.text.chars().count(), 500);
    assert_eq!(ok.due_date, None);
    assert_eq!(
        refusal(t.rt.intent_update(&ok.id, goal("")).await.unwrap_err()),
        TEXT_RULE
    );
}

#[tokio::test]
async fn a_goal_is_marked_done_and_undone() {
    let t = start().await;
    let added =
        t.rt.intent_create(PERSONAL, DEFAULT, goal("Renew the passport"))
            .await
            .unwrap();
    let mut events = t.rt.subscribe();
    let done = t.rt.intent_set_done(&added.id, true).await.unwrap();
    assert!(done.done_at.is_some());
    assert!(heard(&mut events));
    // Done again keeps the first time.
    let again = t.rt.intent_set_done(&added.id, true).await.unwrap();
    assert_eq!(again.done_at, done.done_at);
    let undone = t.rt.intent_set_done(&added.id, false).await.unwrap();
    assert_eq!(undone.done_at, None);
    assert!(heard(&mut events));
    assert_eq!(
        refusal(t.rt.intent_set_done("01NOSUCH", true).await.unwrap_err()),
        GONE
    );
}

#[tokio::test]
async fn a_deleted_dot_leaves_its_goals_unlinked() {
    let t = start().await;
    let dot = t.rt.create_dot(spec("helper"), None, None).await.unwrap();
    let linked =
        t.rt.intent_create(
            PERSONAL,
            DEFAULT,
            IntentFields {
                dot_id: Some(dot.id.clone()),
                ..goal("Draft the newsletter")
            },
        )
        .await
        .unwrap();
    let mut events = t.rt.subscribe();
    t.rt.delete_dot(&dot.id).await.unwrap();
    assert!(heard(&mut events));
    let view = t.rt.intents_list(PERSONAL, DEFAULT).await.unwrap();
    assert_eq!(view.intents.len(), 1);
    assert_eq!(view.intents[0].id, linked.id);
    assert_eq!(view.intents[0].dot_id, None);
    assert_eq!(view.intents[0].text, "Draft the newsletter");
}

#[tokio::test]
async fn goals_stay_in_the_workspace_they_were_added_in() {
    let t = start().await;
    let home = t.rt.personal_workspace_add("Home").await.unwrap();
    let work =
        t.rt.intent_create(PERSONAL, DEFAULT, goal("File the expenses"))
            .await
            .unwrap();
    let chores =
        t.rt.intent_create(PERSONAL, &home.id, goal("Fix the gate"))
            .await
            .unwrap();
    let ids = |v: Vec<dots_core::model::Intent>| v.into_iter().map(|i| i.id).collect::<Vec<_>>();
    assert_eq!(
        ids(t.rt.intents_list(PERSONAL, DEFAULT).await.unwrap().intents),
        vec![work.id.clone()]
    );
    assert_eq!(
        ids(t.rt.intents_list(PERSONAL, &home.id).await.unwrap().intents),
        vec![chores.id.clone()]
    );

    // A dot of another workspace can't be linked.
    let other =
        t.rt.create_dot(spec("default-dot"), None, None)
            .await
            .unwrap();
    assert_eq!(
        refusal(
            t.rt.intent_create(
                PERSONAL,
                &home.id,
                IntentFields {
                    dot_id: Some(other.id.clone()),
                    ..goal("Paint the fence")
                },
            )
            .await
            .unwrap_err()
        ),
        OTHER_WORKSPACE
    );
    assert_eq!(
        refusal(
            t.rt.intent_update(
                &chores.id,
                IntentFields {
                    dot_id: Some(other.id.clone()),
                    ..goal("Fix the gate")
                },
            )
            .await
            .unwrap_err()
        ),
        OTHER_WORKSPACE
    );

    // An unknown workspace is refused.
    assert!(t
        .rt
        .intent_create(PERSONAL, "nowhere", goal("Lost"))
        .await
        .is_err());

    // A deleted workspace's goals go to Default, as its dots do.
    t.rt.personal_workspace_delete(&home.id).await.unwrap();
    let mut all = ids(t.rt.intents_list(PERSONAL, DEFAULT).await.unwrap().intents);
    all.sort();
    let mut want = vec![work.id, chores.id];
    want.sort();
    assert_eq!(all, want);
}

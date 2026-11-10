//! The context meter and the compact and start-fresh rules (context rules spec): the parser on
//! the CLI output pinned in `fixtures/context_turn.jsonl` and `fixtures/compact.jsonl`, the
//! decisions, the seed, and the store.

mod common;

use common::{fake_run, spec, temp_store};
use dots_core::context_rules::{
    applies, asks_fresh, at_or_above, cut_summary, decide, handoff_seed, percent, window_for,
    ContextRules, EventKind, Request, SeedSection, Step, Turn, SEED_HEADING, SUMMARY_MAX,
    WINDOW_1M, WINDOW_DEFAULT,
};
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::EngineEvent;
use dots_core::model::{EngineKind, NewRun, RunStatus, TriggerKind};
use dots_core::settings::Settings;
use dots_core::store::NewContextEvent;
use serde_json::json;

const TURN: &str = include_str!("fixtures/context_turn.jsonl");
const COMPACT: &str = include_str!("fixtures/compact.jsonl");

fn events(lines: &str) -> Vec<EngineEvent> {
    lines.lines().flat_map(parse_line).collect()
}

fn contexts(events: &[EngineEvent]) -> Vec<(Option<u64>, Option<u64>)> {
    events
        .iter()
        .filter_map(|e| match e {
            EngineEvent::Context { fill, window } => Some((*fill, *window)),
            _ => None,
        })
        .collect()
}

// ---- parser ----

#[test]
fn a_real_turn_gives_its_fill_and_the_results_window() {
    let all = events(TURN);
    // The assistant line: 2 + 4,767 + 19,541; then the result's `modelUsage` window.
    assert_eq!(
        contexts(&all),
        vec![(Some(24_310), None), (None, Some(1_000_000))]
    );
    // The rest is as before: the session, the text, the usage and the end.
    assert!(matches!(all[0], EngineEvent::SessionStarted { .. }));
    assert!(all
        .iter()
        .any(|e| matches!(e, EngineEvent::Finished { .. })));
}

#[test]
fn a_real_compaction_gives_its_boundary_and_no_turn() {
    let all = events(COMPACT);
    let compacted: Vec<_> = all
        .iter()
        .filter(|e| matches!(e, EngineEvent::Compacted { .. }))
        .collect();
    assert_eq!(
        compacted,
        vec![&EngineEvent::Compacted {
            trigger: "manual".into(),
            before: 24_315,
            after: 3_520,
        }]
    );
    // `/compact` makes no turn of its own: no fill.
    assert!(contexts(&all).iter().all(|(fill, _)| fill.is_none()));
    // Its result is a success with an empty text.
    assert!(all
        .iter()
        .any(|e| matches!(e, EngineEvent::Finished { summary } if summary.is_empty())));
}

#[test]
fn a_helpers_turn_is_not_the_sessions_fill() {
    let helper = json!({"type": "assistant", "parent_tool_use_id": "toolu_task",
        "message": {"content": [{"type": "text", "text": "x"}],
        "usage": {"input_tokens": 5, "cache_read_input_tokens": 90_000}}});
    assert!(contexts(&parse_line(&helper.to_string())).is_empty());
    let own = json!({"type": "assistant", "parent_tool_use_id": null,
        "message": {"content": [], "usage": {"input_tokens": 5,
        "cache_creation_input_tokens": 10, "cache_read_input_tokens": 90_000}}});
    assert_eq!(
        contexts(&parse_line(&own.to_string())),
        vec![(Some(90_015), None)]
    );
    // A line without usage gives no fill.
    let bare = json!({"type": "assistant", "message": {"content": []}});
    assert!(contexts(&parse_line(&bare.to_string())).is_empty());
}

#[test]
fn a_1m_model_names_its_window_at_init() {
    let init = json!({"type": "system", "subtype": "init", "session_id": "s",
        "model": "claude-opus-5-5[1m]"});
    assert_eq!(
        contexts(&parse_line(&init.to_string())),
        vec![(None, Some(WINDOW_1M))]
    );
    let plain = json!({"type": "system", "subtype": "init", "session_id": "s",
        "model": "claude-opus-5-5"});
    assert!(contexts(&parse_line(&plain.to_string())).is_empty());
}

#[test]
fn the_clis_own_compaction_is_auto() {
    let line = json!({"type": "system", "subtype": "compact_boundary",
        "compact_metadata": {"trigger": "auto", "pre_tokens": 150_000, "post_tokens": 20_000}});
    assert_eq!(
        parse_line(&line.to_string()),
        vec![EngineEvent::Compacted {
            trigger: "auto".into(),
            before: 150_000,
            after: 20_000
        }]
    );
}

#[test]
fn a_context_event_is_named_and_round_trips() {
    let e = EngineEvent::Context {
        fill: Some(5),
        window: None,
    };
    assert_eq!(e.kind(), "context");
    let back: EngineEvent = serde_json::from_value(serde_json::to_value(&e).unwrap()).unwrap();
    assert_eq!(back, e);
    let c = EngineEvent::Compacted {
        trigger: "auto".into(),
        before: 2,
        after: 1,
    };
    assert_eq!(c.kind(), "compacted");
}

// ---- rules ----

#[test]
fn the_rules_default_to_on_60_and_80_and_keep_their_bounds() {
    let r = ContextRules::default();
    assert_eq!((r.enabled, r.compact_at, r.clear_at), (true, 60, 80));
    assert!(r.validate().is_ok());
    let bad = |compact_at, clear_at| {
        ContextRules {
            enabled: true,
            compact_at,
            clear_at,
        }
        .validate()
        .is_err()
    };
    assert!(bad(9, 80));
    assert!(bad(60, 96));
    assert!(bad(70, 70));
    assert!(bad(80, 60));
    assert!(!bad(10, 95));
    assert_eq!(Settings::default().context_rules, ContextRules::default());
    let mut s = Settings::default();
    s.context_rules.clear_at = 50;
    assert!(s.validate().is_err());
    let mut d = spec("Scout");
    d.context_rules = Some(ContextRules {
        enabled: true,
        compact_at: 90,
        clear_at: 85,
    });
    assert!(d.validate().is_err());
}

#[test]
fn the_window_and_the_share() {
    assert_eq!(window_for(Some("sonnet"), None), WINDOW_DEFAULT);
    assert_eq!(window_for(Some("opus[1m]"), None), WINDOW_1M);
    assert_eq!(window_for(Some("sonnet"), Some(1_000_000)), 1_000_000);
    assert_eq!(window_for(None, Some(0)), WINDOW_DEFAULT);
    assert_eq!(percent(92_000, 200_000), 46);
    assert!(at_or_above(120_000, 200_000, 60));
    assert!(!at_or_above(119_999, 200_000, 60));
}

fn turn(chat: bool, fill: Option<u64>, request: Option<Request>) -> Turn {
    Turn {
        chat,
        fill,
        window: 200_000,
        request,
    }
}

#[test]
fn a_background_turn_compacts_at_60_and_starts_fresh_at_80() {
    let r = ContextRules::default();
    assert_eq!(decide(&r, &turn(false, Some(100_000), None)), Step::Nothing);
    assert_eq!(decide(&r, &turn(false, Some(120_000), None)), Step::Compact);
    assert_eq!(decide(&r, &turn(false, Some(159_999), None)), Step::Compact);
    assert_eq!(decide(&r, &turn(false, Some(160_000), None)), Step::Fresh);
    assert_eq!(decide(&r, &turn(false, None, None)), Step::Nothing);
}

#[test]
fn a_chat_turn_is_compacted_and_starts_fresh_only_when_asked() {
    let r = ContextRules::default();
    assert_eq!(decide(&r, &turn(true, Some(170_000), None)), Step::Compact);
    assert_eq!(
        decide(&r, &turn(true, Some(10_000), Some(Request::Fresh))),
        Step::Fresh
    );
    assert_eq!(
        decide(&r, &turn(true, Some(10_000), Some(Request::Compact))),
        Step::Compact
    );
}

#[test]
fn rules_off_do_nothing_but_a_request_still_counts() {
    let off = ContextRules {
        enabled: false,
        ..ContextRules::default()
    };
    assert_eq!(
        decide(&off, &turn(false, Some(190_000), None)),
        Step::Nothing
    );
    assert_eq!(
        decide(&off, &turn(false, None, Some(Request::Fresh))),
        Step::Fresh
    );
}

#[test]
fn the_topic_asks_until_keep_going_or_a_request() {
    let r = ContextRules::default();
    assert!(asks_fresh(&r, Some(164_000), 200_000, false, None));
    assert!(!asks_fresh(&r, Some(150_000), 200_000, false, None));
    assert!(!asks_fresh(&r, Some(164_000), 200_000, true, None));
    assert!(!asks_fresh(
        &r,
        Some(164_000),
        200_000,
        false,
        Some(Request::Fresh)
    ));
    let off = ContextRules {
        enabled: false,
        ..r
    };
    assert!(!asks_fresh(&off, Some(190_000), 200_000, false, None));
}

#[test]
fn the_rules_look_at_a_claude_turn_of_a_session_that_is_not_a_resume() {
    let chat = fake_run(TriggerKind::Chat, None, Some("s"));
    assert!(applies(&chat, EngineKind::Claude));
    assert!(!applies(&chat, EngineKind::Local));
    assert!(!applies(
        &fake_run(TriggerKind::Resume, None, Some("s")),
        EngineKind::Claude
    ));
    assert!(!applies(
        &fake_run(TriggerKind::Chat, None, None),
        EngineKind::Claude
    ));
    assert!(applies(
        &fake_run(TriggerKind::FollowUp, None, Some("s")),
        EngineKind::Claude
    ));
}

// ---- seed ----

#[test]
fn the_seed_has_the_note_the_last_messages_and_extra_sections() {
    let mut thread = Vec::new();
    for i in 0..8 {
        let mut r = fake_run(
            TriggerKind::Chat,
            Some(json!({"message": format!("question {i}")})),
            Some("s"),
        );
        r.summary = Some(format!("answer {i}"));
        thread.push(r);
    }
    let extra = [SeedSection {
        title: "Project map".into(),
        body: "src/ holds the code.".into(),
    }];
    let seed = handoff_seed(Some("We are fixing the parser."), &thread, &extra);
    assert!(seed.starts_with(SEED_HEADING), "{seed}");
    assert!(seed.contains("### Hand-off note\n\nWe are fixing the parser."));
    // The last six messages only.
    assert!(!seed.contains("question 1"), "{seed}");
    assert!(seed.contains("- The person: question 2"));
    assert!(seed.contains("  You: answer 7"));
    assert!(seed.contains("### Project map\n\nsrc/ holds the code."));
    // No note: it says so.
    let bare = handoff_seed(None, &[], &[]);
    assert!(bare.contains("(No hand-off note could be written.)"));
    assert!(!bare.contains("### Last messages"));
}

#[test]
fn a_long_summary_is_cut() {
    let long = "a".repeat(SUMMARY_MAX + 10);
    let cut = cut_summary(&long).unwrap();
    assert_eq!(cut.chars().count(), SUMMARY_MAX + 1);
    assert!(cut.ends_with('…'));
    assert_eq!(cut_summary("  "), None);
}

// ---- store ----

#[tokio::test]
async fn the_store_keeps_fills_requests_and_records() {
    let (_dir, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("Scout"), None, "default")
        .await
        .unwrap();
    let first = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    store.set_session(&first.id, "S1").await.unwrap();
    assert_eq!(store.session_fill("S1").await.unwrap(), None);
    assert!(store
        .set_run_context(&first.id, Some(90_000), None)
        .await
        .unwrap());
    // The same again changes nothing; a window alone keeps the fill.
    assert!(!store
        .set_run_context(&first.id, Some(90_000), None)
        .await
        .unwrap());
    assert!(store
        .set_run_context(&first.id, None, Some(1_000_000))
        .await
        .unwrap());
    let run = store.get_run(&first.id).await.unwrap();
    assert_eq!(
        (run.context_fill, run.context_window),
        (Some(90_000), Some(1_000_000))
    );
    let mut next = NewRun::new(&dot.id, TriggerKind::Chat);
    next.parent_run_id = Some(first.id.clone());
    next.root_run_id = Some(first.id.clone());
    next.session_id = Some("S1".into());
    let second = store.create_run(&next).await.unwrap();
    store
        .finish_run(&first.id, RunStatus::Succeeded, Some("ok"), None)
        .await
        .unwrap();
    store
        .set_run_context(&second.id, Some(120_000), None)
        .await
        .unwrap();
    let last = store.session_fill("S1").await.unwrap().unwrap();
    assert_eq!(
        (last.run_id.as_str(), last.fill),
        (second.id.as_str(), 120_000)
    );
    assert_eq!(
        store.thread_fill(&first.id).await.unwrap().unwrap().fill,
        120_000
    );

    // The topic's request is taken once.
    let root = &first.id;
    assert_eq!(store.thread_state(root).await.unwrap().request, None);
    store
        .set_thread_request(root, Some(Request::Fresh))
        .await
        .unwrap();
    store.set_keep_going(root, true).await.unwrap();
    let state = store.thread_state(root).await.unwrap();
    assert_eq!(
        (state.keep_going, state.request),
        (true, Some(Request::Fresh))
    );
    assert_eq!(
        store.take_thread_request(root).await.unwrap(),
        Some(Request::Fresh)
    );
    assert_eq!(store.take_thread_request(root).await.unwrap(), None);
    store.clear_keep_going(root).await.unwrap();
    assert!(!store.thread_state(root).await.unwrap().keep_going);

    // Records, and the dot's counts.
    store
        .add_context_event(&NewContextEvent {
            run_id: &second.id,
            dot_id: &dot.id,
            root_run_id: root,
            kind: EventKind::Compact,
            manual: false,
            fill_before: 124_000,
            fill_after: Some(36_000),
            from_session: None,
        })
        .await
        .unwrap();
    let clear = store
        .add_context_event(&NewContextEvent {
            run_id: &second.id,
            dot_id: &dot.id,
            root_run_id: root,
            kind: EventKind::Clear,
            manual: true,
            fill_before: 168_000,
            fill_after: None,
            from_session: Some("S1"),
        })
        .await
        .unwrap();
    assert_eq!(clear.trigger, "manual");
    assert_eq!(clear.from_session.as_deref(), Some("S1"));
    store.set_clear_after(&second.id, 30_000).await.unwrap();
    store.set_clear_after(&second.id, 40_000).await.unwrap();
    let list = store.list_context_events(&second.id).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].fill_after, Some(30_000));
    assert_eq!(store.context_counts(&dot.id).await.unwrap(), (1, 1));
}

#[tokio::test]
async fn a_dots_own_rules_are_stored_and_none_takes_settings() {
    let (_dir, store) = temp_store().await;
    let mut s = spec("Scout");
    let dot = store.create_dot(&s, None, "default").await.unwrap();
    assert_eq!(dot.spec.context_rules, None);
    s.context_rules = Some(ContextRules {
        enabled: false,
        compact_at: 40,
        clear_at: 70,
    });
    let dot = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(dot.spec.context_rules, s.context_rules);
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.context_rules,
        s.context_rules
    );
}

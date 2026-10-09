use std::path::Path;

use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::EngineEvent;

fn events(name: &str) -> Vec<EngineEvent> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(parse_line)
        .collect()
}

fn terminal_count(ev: &[EngineEvent]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, EngineEvent::Finished { .. } | EngineEvent::Failed { .. }))
        .count()
}

#[test]
fn simple_fixture_starts_a_session_and_finishes() {
    let ev = events("simple.jsonl");
    assert!(
        matches!(ev.first(), Some(EngineEvent::SessionStarted { .. })),
        "{ev:#?}"
    );
    assert_eq!(terminal_count(&ev), 1);
    assert!(
        matches!(ev.last(), Some(EngineEvent::Finished { summary }) if summary.contains("pong"))
    );
    assert!(ev
        .iter()
        .any(|e| matches!(e, EngineEvent::Usage { tokens_out, .. } if *tokens_out > 0)));
}

#[test]
fn tool_use_fixture_has_call_and_result() {
    let ev = events("tool_use.jsonl");
    assert!(
        ev.iter()
            .any(|e| matches!(e, EngineEvent::ToolCall { tool, .. } if tool == "Bash")),
        "{ev:#?}"
    );
    assert!(ev
        .iter()
        .any(|e| matches!(e, EngineEvent::ToolResult { .. })));
    assert_eq!(terminal_count(&ev), 1);
    assert!(matches!(ev.last(), Some(EngineEvent::Finished { .. })));
}

#[test]
fn max_turns_fixture_fails_with_subtype() {
    let ev = events("max_turns.jsonl");
    assert_eq!(terminal_count(&ev), 1);
    assert!(
        matches!(ev.last(), Some(EngineEvent::Failed { error }) if error.starts_with("error_max_turns")),
        "{:#?}",
        ev.last()
    );
}

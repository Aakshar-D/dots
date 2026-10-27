//! The `claude` CLI's limit readings and run cost (usage and limits spec sections 3 and 5): a
//! `rate_limit_event` line becomes one `Limits` event with a reading per window, and the result
//! line's `total_cost_usd` rides on `Usage`. The fixture is a line CLI 2.1.310 printed.

use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::EngineEvent;
use dots_core::store::LimitReading;

const RATE_LIMIT: &str = include_str!("fixtures/rate_limit_event.json");

fn strip(r: &LimitReading) -> (String, f64, String, String) {
    (
        r.window.clone(),
        r.utilization,
        r.resets_at.clone(),
        r.status.clone(),
    )
}

#[test]
fn the_rate_limit_line_becomes_a_limits_event() {
    let events = parse_line(RATE_LIMIT);
    let [EngineEvent::Limits {
        status,
        windows,
        resets_at,
    }] = events.as_slice()
    else {
        panic!("one Limits event, got {events:?}");
    };
    assert_eq!(status, "allowed");
    // The line's own reset, of the window that limits now (final review I2).
    assert_eq!(resets_at.as_deref(), Some("2026-10-26T17:40:00.000Z"));
    let got: Vec<_> = windows.iter().map(strip).collect();
    assert_eq!(
        got,
        [
            (
                "five_hour".to_string(),
                0.05,
                "2026-10-26T17:40:00.000Z".to_string(),
                "allowed".to_string()
            ),
            (
                "seven_day".to_string(),
                0.16,
                "2026-10-28T04:00:00.000Z".to_string(),
                "allowed".to_string()
            ),
        ]
    );
    assert!(windows.iter().all(|w| !w.read_at.is_empty()));
}

#[test]
fn a_rate_limit_line_with_only_the_limiting_window_reads_that_one() {
    let line = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1793036400,"rateLimitType":"five_hour","utilization":1.02}}"#;
    let events = parse_line(line);
    let [EngineEvent::Limits {
        status, windows, ..
    }] = events.as_slice()
    else {
        panic!("one Limits event, got {events:?}");
    };
    assert_eq!(status, "rejected");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].window, "five_hour");
    assert_eq!(windows[0].utilization, 1.02);
    assert_eq!(windows[0].status, "rejected");
}

#[test]
fn a_rate_limit_line_with_a_status_only_has_no_readings() {
    let line = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#;
    assert_eq!(
        parse_line(line),
        vec![EngineEvent::Limits {
            status: "allowed".into(),
            windows: vec![],
            resets_at: None,
        }]
    );
}

#[test]
fn a_malformed_rate_limit_line_is_ignored() {
    assert!(parse_line(r#"{"type":"rate_limit_event","rate_limit_info":"x"}"#).is_empty());
    assert!(parse_line(r#"{"type":"rate_limit_event"}"#).is_empty());
}

#[test]
fn the_result_line_carries_the_cost() {
    let with = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":4},"total_cost_usd":1.32}"#;
    assert_eq!(
        parse_line(with)[0],
        EngineEvent::Usage {
            tokens_in: 3,
            tokens_out: 4,
            cost_usd: Some(1.32),
        }
    );
    let without = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":3,"output_tokens":4}}"#;
    assert_eq!(
        parse_line(without)[0],
        EngineEvent::Usage {
            tokens_in: 3,
            tokens_out: 4,
            cost_usd: None,
        }
    );
}

mod common;

use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, TimeDelta, Utc, Weekday};
use common::tz::{at, rfc, TestTz};
use dots_core::schedule::{
    due_slot, newest_slot_in, next_slot, slots_after, upcoming, Rule, Schedule, Until,
};
use dots_core::{Error, Result};
use serde_json::{json, Value};

fn hm(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s, "%H:%M").unwrap()
}

fn local(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn once(s: &str) -> Rule {
    Rule::Once { at: local(s) }
}

fn every(minutes: u32, anchor: &str) -> Rule {
    Rule::Every {
        minutes,
        anchor: Some(utc(anchor)),
    }
}

fn daily(time: &str) -> Rule {
    Rule::Daily { time: hm(time) }
}

fn weekly(days: &[Weekday], time: &str) -> Rule {
    Rule::Weekly {
        days: days.to_vec(),
        time: hm(time),
    }
}

fn cron(expr: &str) -> Rule {
    Rule::Cron { expr: expr.into() }
}

fn on(rule: Rule, until: Option<Until>) -> Schedule {
    Schedule {
        rule,
        catch_up: true,
        until,
    }
}

/// The first `n` slots of `rule` strictly after `after`, in `TestTz`.
fn first(rule: &Rule, after: &str, n: usize) -> Vec<String> {
    rfc(slots_after(rule, &at(after), &TestTz).unwrap().take(n))
}

fn newest(rule: &Rule, after: &str, upto: &str) -> Option<String> {
    newest_slot_in(rule, &at(after), &at(upto), &TestTz)
        .unwrap()
        .map(|t| t.to_rfc3339())
}

fn invalid(r: Result<()>) -> String {
    match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

// --- JSON ---

#[test]
fn a_schedule_is_json_with_a_kind_tag_as_the_spec_writes_it() {
    let v = json!({"rule": {"kind": "daily", "time": "09:00"}, "catch_up": true, "until": {"count": 10}});
    let s: Schedule = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(s, on(daily("09:00"), Some(Until::Count(10))));
    assert_eq!(serde_json::to_value(&s).unwrap(), v);
}

#[test]
fn every_kind_and_bound_round_trips() {
    let rules = [
        json!({"kind": "once", "at": "2026-10-20T09:00"}),
        json!({"kind": "every", "minutes": 90, "anchor": "2026-10-14T08:00:00Z"}),
        json!({"kind": "daily", "time": "23:59"}),
        json!({"kind": "weekly", "days": ["mon", "fri", "sun"], "time": "07:30"}),
        json!({"kind": "cron", "expr": "0 0 9 * * *"}),
    ];
    for rule in rules {
        let v = json!({"rule": rule, "catch_up": false, "until": null});
        let s: Schedule = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap(), v);
    }
    let v = json!({"rule": {"kind": "daily", "time": "09:00"}, "catch_up": true, "until": {"date": "2026-10-20T09:00"}});
    let s: Schedule = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(s.until, Some(Until::Date(local("2026-10-20T09:00"))));
    assert_eq!(serde_json::to_value(&s).unwrap(), v);
    let s: Schedule = serde_json::from_value(
        json!({"rule": {"kind": "weekly", "days": ["sat"], "time": "10:00"}}),
    )
    .unwrap();
    assert_eq!(s.rule, weekly(&[Weekday::Sat], "10:00"));
}

#[test]
fn catch_up_defaults_to_true_and_a_missing_anchor_to_none() {
    let s: Schedule =
        serde_json::from_value(json!({"rule": {"kind": "every", "minutes": 30}})).unwrap();
    assert_eq!(
        s,
        on(
            Rule::Every {
                minutes: 30,
                anchor: None
            },
            None
        )
    );
    assert_eq!(
        serde_json::to_value(&s).unwrap()["rule"]["anchor"],
        Value::Null
    );
}

#[test]
fn times_dates_and_days_that_do_not_parse_are_refused() {
    let bad = [
        json!({"kind": "daily", "time": "25:00"}),
        json!({"kind": "daily", "time": "09:00:30"}),
        json!({"kind": "daily", "time": "9am"}),
        json!({"kind": "once", "at": "2026-10-20 09:00"}),
        json!({"kind": "once", "at": "2026-02-30T09:00"}),
        json!({"kind": "weekly", "days": ["mon", "funday"], "time": "09:00"}),
        json!({"kind": "monthly"}),
    ];
    for rule in bad {
        let r = serde_json::from_value::<Schedule>(json!({ "rule": rule }));
        assert!(r.is_err(), "{rule} should not parse");
    }
    let e = serde_json::from_value::<Schedule>(json!({"rule": {"kind": "daily", "time": "25:00"}}))
        .unwrap_err();
    assert!(e.to_string().contains("expected HH:MM"), "{e}");
    let e = serde_json::from_value::<Schedule>(
        json!({"rule": {"kind": "daily", "time": "09:00"}, "until": {"date": "tomorrow"}}),
    )
    .unwrap_err();
    assert!(e.to_string().contains("expected YYYY-MM-DDTHH:MM"), "{e}");
}

// --- Validation ---

#[test]
fn validate_refuses_each_structural_mistake() {
    let anchor = "2026-10-14T08:00:00Z";
    assert!(invalid(on(every(0, anchor), None).validate()).contains("1 to 10080"));
    assert!(invalid(on(every(10081, anchor), None).validate()).contains("1 to 10080"));
    assert!(invalid(on(weekly(&[], "09:00"), None).validate()).contains("at least one day"));
    assert!(invalid(on(cron("* * *"), None).validate()).contains("cron"));
    assert!(invalid(on(cron("0 0 25 * * *"), None).validate()).contains("cron"));
    assert!(invalid(on(daily("09:00"), Some(Until::Count(0))).validate()).contains("at least 1"));
    let once_until = on(
        once("2026-10-20T09:00"),
        Some(Until::Date(local("2026-10-21T09:00"))),
    );
    assert!(invalid(once_until.validate()).contains("once"));
}

#[test]
fn validate_accepts_each_kind_within_bounds() {
    let anchor = "2026-10-14T08:00:00Z";
    for s in [
        on(once("2020-01-01T00:00"), None),
        on(every(1, anchor), Some(Until::Count(1))),
        on(every(10080, anchor), None),
        on(
            Rule::Every {
                minutes: 5,
                anchor: None,
            },
            None,
        ),
        on(daily("00:00"), Some(Until::Date(local("2026-10-20T09:00")))),
        on(weekly(&[Weekday::Mon], "09:00"), None),
        on(cron("0 0 9 * * Mon-Fri"), None),
        on(cron("0 0 9 1 1 * 2030"), None),
    ] {
        s.validate().unwrap_or_else(|e| panic!("{s:?}: {e}"));
    }
}

#[test]
fn a_schedule_that_would_never_run_is_refused() {
    let now = at("2026-10-14T10:00:00+02:00");
    let never = |s: Schedule| {
        let m = invalid(s.check_runs_after(&now, &TestTz));
        assert!(m.contains("this schedule would never run"), "{m}");
    };
    never(on(once("2026-10-14T09:00"), None));
    never(on(once("2026-10-14T10:00"), None));
    never(on(
        daily("09:00"),
        Some(Until::Date(local("2026-10-15T08:59"))),
    ));
    never(on(
        Rule::Every {
            minutes: 60,
            anchor: None,
        },
        Some(Until::Date(local("2026-10-14T10:30"))),
    ));
    never(on(cron("0 0 9 1 1 * 2020"), None));

    let runs = |s: Schedule| s.check_runs_after(&now, &TestTz).unwrap();
    runs(on(once("2026-10-14T10:01"), None));
    runs(on(
        daily("09:00"),
        Some(Until::Date(local("2026-10-15T09:00"))),
    ));
    runs(on(
        Rule::Every {
            minutes: 60,
            anchor: None,
        },
        None,
    ));
    runs(on(daily("09:00"), Some(Until::Count(1))));
}

#[test]
fn the_core_fills_the_anchor_and_keeps_it_while_minutes_stay() {
    let now = utc("2026-10-14T08:00:00.123Z");
    let old = utc("2026-10-01T06:30:00Z");
    let bare = |minutes| {
        on(
            Rule::Every {
                minutes,
                anchor: None,
            },
            None,
        )
    };
    let anchor_of = |s: &Schedule| match s.rule {
        Rule::Every { anchor, .. } => anchor,
        _ => panic!("not every"),
    };
    let previous = on(
        Rule::Every {
            minutes: 30,
            anchor: Some(old),
        },
        None,
    );

    let mut s = bare(30);
    s.fill_anchor(None, now);
    assert_eq!(anchor_of(&s), Some(now), "a new schedule starts now");

    let mut s = bare(30);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(anchor_of(&s), Some(old), "same minutes keep the phase");

    let mut s = bare(45);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(anchor_of(&s), Some(now), "new minutes restart the phase");

    let mut s = previous.clone();
    if let Rule::Every { minutes, .. } = &mut s.rule {
        *minutes = 45;
    }
    s.fill_anchor(Some(&previous), now);
    assert_eq!(
        anchor_of(&s),
        Some(now),
        "the old anchor sent back with new minutes"
    );

    let mut s = on(every(30, "2026-10-10T00:00:00Z"), None);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(anchor_of(&s), Some(utc("2026-10-10T00:00:00Z")));

    let mut s = on(daily("09:00"), None);
    s.fill_anchor(Some(&previous), now);
    assert_eq!(s, on(daily("09:00"), None));
}

// --- Slots ---

#[test]
fn once_is_one_slot_strictly_after_after() {
    let r = once("2026-10-20T09:00");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 3),
        ["2026-10-20T09:00:00+02:00"]
    );
    assert!(first(&r, "2026-10-20T09:00:00+02:00", 3).is_empty());
    assert_eq!(
        first(&once("2026-03-29T02:30"), "2026-03-28T00:00:00+01:00", 3),
        ["2026-03-29T03:30:00+02:00"]
    );
}

#[test]
fn every_counts_elapsed_time_from_its_anchor() {
    let r = every(15, "2026-10-14T07:07:00Z");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 2),
        ["2026-10-14T10:07:00+02:00", "2026-10-14T10:22:00+02:00"]
    );
    // Whole k, so also before the anchor.
    assert_eq!(
        first(&r, "2026-10-14T08:50:00+02:00", 2),
        ["2026-10-14T08:52:00+02:00", "2026-10-14T09:07:00+02:00"]
    );
    assert_eq!(
        first(&r, "2026-10-14T10:07:00+02:00", 1),
        ["2026-10-14T10:22:00+02:00"]
    );
}

#[test]
fn every_does_not_follow_the_wall_clock_across_the_gap_and_the_overlap() {
    let spring = every(60, "2026-03-28T23:00:00Z");
    assert_eq!(
        first(&spring, "2026-03-29T00:00:00+01:00", 3),
        [
            "2026-03-29T01:00:00+01:00",
            "2026-03-29T03:00:00+02:00",
            "2026-03-29T04:00:00+02:00"
        ]
    );
    let autumn = every(60, "2026-10-24T22:00:00Z");
    assert_eq!(
        first(&autumn, "2026-10-25T00:30:00+02:00", 4),
        [
            "2026-10-25T01:00:00+02:00",
            "2026-10-25T02:00:00+02:00",
            "2026-10-25T02:00:00+01:00",
            "2026-10-25T03:00:00+01:00"
        ]
    );
}

#[test]
fn every_without_an_anchor_has_no_slots() {
    let r = Rule::Every {
        minutes: 5,
        anchor: None,
    };
    assert!(matches!(
        slots_after(&r, &at("2026-10-14T10:00:00+02:00"), &TestTz),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn daily_is_the_wall_clock_time_on_each_day() {
    let r = daily("09:00");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 2),
        ["2026-10-15T09:00:00+02:00", "2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(
        first(&r, "2026-10-15T09:00:00+02:00", 1),
        ["2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(
        first(&r, "2026-10-14T08:59:59+02:00", 1),
        ["2026-10-14T09:00:00+02:00"]
    );
}

#[test]
fn a_time_in_the_gap_moves_forward_by_the_gap() {
    assert_eq!(
        first(&daily("02:30"), "2026-03-28T12:00:00+01:00", 2),
        ["2026-03-29T03:30:00+02:00", "2026-03-30T02:30:00+02:00"]
    );
    assert_eq!(
        first(&daily("02:00"), "2026-03-28T12:00:00+01:00", 1),
        ["2026-03-29T03:00:00+02:00"]
    );
}

#[test]
fn a_time_in_the_overlap_fires_once_at_the_earlier_instant() {
    let r = daily("02:30");
    assert_eq!(
        first(&r, "2026-10-24T12:00:00+02:00", 2),
        ["2026-10-25T02:30:00+02:00", "2026-10-26T02:30:00+01:00"]
    );
    assert_eq!(
        first(&r, "2026-10-25T02:30:00+02:00", 1),
        ["2026-10-26T02:30:00+01:00"]
    );
}

#[test]
fn weekly_fires_on_its_days_only() {
    let r = weekly(&[Weekday::Fri, Weekday::Mon], "09:00");
    assert_eq!(
        first(&r, "2026-10-14T10:00:00+02:00", 4),
        [
            "2026-10-16T09:00:00+02:00",
            "2026-10-19T09:00:00+02:00",
            "2026-10-23T09:00:00+02:00",
            "2026-10-26T09:00:00+01:00"
        ]
    );
}

#[test]
fn cron_is_what_the_cron_crate_computes() {
    let r = cron("0 0 9 * * Mon-Fri");
    assert_eq!(
        first(&r, "2026-10-16T10:00:00+02:00", 2),
        ["2026-10-19T09:00:00+02:00", "2026-10-20T09:00:00+02:00"]
    );
    assert!(matches!(
        slots_after(&cron("nonsense"), &at("2026-10-14T10:00:00+02:00"), &TestTz),
        Err(Error::Invalid(_))
    ));
}

/// Records what the `cron` crate does across DST (spec §3); Dots does not change it, except
/// that a slot earlier than the one before it is dropped, so slots stay in order.
#[test]
fn cron_across_the_gap_and_the_overlap() {
    // Gap: a time that does not exist is skipped, not moved.
    assert_eq!(
        first(&cron("0 0 * * * *"), "2026-03-29T00:30:00+01:00", 3),
        [
            "2026-03-29T01:00:00+01:00",
            "2026-03-29T03:00:00+02:00",
            "2026-03-29T04:00:00+02:00"
        ]
    );
    assert_eq!(
        first(&cron("0 30 2 * * *"), "2026-03-28T12:00:00+01:00", 2),
        ["2026-03-30T02:30:00+02:00", "2026-03-31T02:30:00+02:00"]
    );
    // Overlap: a time that occurs twice fires twice.
    assert_eq!(
        first(&cron("0 0 * * * *"), "2026-10-25T00:30:00+02:00", 4),
        [
            "2026-10-25T01:00:00+02:00",
            "2026-10-25T02:00:00+02:00",
            "2026-10-25T02:00:00+01:00",
            "2026-10-25T03:00:00+01:00"
        ]
    );
    assert_eq!(
        first(&cron("0 30 2 * * *"), "2026-10-24T12:00:00+02:00", 3),
        [
            "2026-10-25T02:30:00+02:00",
            "2026-10-25T02:30:00+01:00",
            "2026-10-26T02:30:00+01:00"
        ]
    );
    // Every minute: after the second 02:00 the crate goes back to the first pass of the hour
    // (02:01+02:00); those slots are dropped, so the first pass ends at 02:00.
    assert_eq!(
        first(&cron("0 * * * * *"), "2026-10-25T01:58:00+02:00", 5),
        [
            "2026-10-25T01:59:00+02:00",
            "2026-10-25T02:00:00+02:00",
            "2026-10-25T02:00:00+01:00",
            "2026-10-25T02:01:00+01:00",
            "2026-10-25T02:02:00+01:00"
        ]
    );
}

// --- Newest slot in a window ---

#[test]
fn newest_slot_in_is_the_newest_in_after_exclusive_to_upto_inclusive() {
    let d = daily("09:00");
    assert_eq!(
        newest(&d, "2026-10-10T12:00:00+02:00", "2026-10-14T10:00:00+02:00").as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(&d, "2026-10-10T12:00:00+02:00", "2026-10-14T09:00:00+02:00").as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(&d, "2026-10-14T09:00:00+02:00", "2026-10-14T10:00:00+02:00"),
        None
    );
    assert_eq!(
        newest(&d, "2026-10-14T09:00:00+02:00", "2026-10-15T09:00:00+02:00").as_deref(),
        Some("2026-10-15T09:00:00+02:00")
    );

    let e = every(60, "2026-10-14T08:00:00Z");
    assert_eq!(
        newest(&e, "2026-10-14T10:30:00+02:00", "2026-10-14T14:15:00+02:00").as_deref(),
        Some("2026-10-14T14:00:00+02:00")
    );
    assert_eq!(
        newest(&e, "2026-10-14T14:00:00+02:00", "2026-10-14T14:15:00+02:00"),
        None
    );

    let w = weekly(&[Weekday::Mon, Weekday::Fri], "09:00");
    assert_eq!(
        newest(&w, "2026-10-14T10:00:00+02:00", "2026-10-21T10:00:00+02:00").as_deref(),
        Some("2026-10-19T09:00:00+02:00")
    );

    let o = once("2026-10-20T09:00");
    assert_eq!(
        newest(&o, "2026-10-14T10:00:00+02:00", "2026-10-21T10:00:00+02:00").as_deref(),
        Some("2026-10-20T09:00:00+02:00")
    );
    assert_eq!(
        newest(&o, "2026-10-20T09:00:00+02:00", "2026-10-21T10:00:00+02:00"),
        None
    );
    assert_eq!(
        newest(&o, "2026-10-14T10:00:00+02:00", "2026-10-20T08:59:00+02:00"),
        None
    );

    let c = cron("0 0 9 * * *");
    assert_eq!(
        newest(&c, "2026-10-10T12:00:00+02:00", "2026-10-14T09:00:00+02:00").as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(&c, "2026-10-14T09:00:00+02:00", "2026-10-14T10:00:00+02:00"),
        None
    );
}

#[test]
fn newest_slot_in_applies_the_dst_rules() {
    assert_eq!(
        newest(
            &daily("02:30"),
            "2026-10-25T00:00:00+02:00",
            "2026-10-25T04:00:00+01:00"
        )
        .as_deref(),
        Some("2026-10-25T02:30:00+02:00")
    );
    assert_eq!(
        newest(
            &daily("02:30"),
            "2026-10-25T02:30:00+02:00",
            "2026-10-25T04:00:00+01:00"
        ),
        None,
        "the repeated 02:30 is not a second slot"
    );
    assert_eq!(
        newest(
            &daily("02:30"),
            "2026-03-29T00:00:00+01:00",
            "2026-03-29T12:00:00+02:00"
        )
        .as_deref(),
        Some("2026-03-29T03:30:00+02:00")
    );
}

#[test]
fn newest_slot_in_a_long_window_does_not_walk_every_slot() {
    let started = std::time::Instant::now();
    assert_eq!(
        newest(
            &cron("* * * * * *"),
            "2020-01-01T00:00:00+01:00",
            "2026-10-14T09:00:00+02:00"
        )
        .as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert_eq!(
        newest(
            &every(1, "2020-01-01T00:00:30Z"),
            "2020-01-01T00:00:00+01:00",
            "2026-10-14T09:00:00+02:00"
        )
        .as_deref(),
        Some("2026-10-14T08:59:30+02:00")
    );
    assert_eq!(
        newest(
            &daily("09:00"),
            "2020-01-01T00:00:00+01:00",
            "2026-10-14T09:00:00+02:00"
        )
        .as_deref(),
        Some("2026-10-14T09:00:00+02:00")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

// --- Bounds ---

#[test]
fn until_date_is_inclusive_and_resolved_like_a_slot() {
    let s = on(daily("09:00"), Some(Until::Date(local("2026-10-16T09:00"))));
    let next = |after: &str| {
        next_slot(&s, &at(after), &TestTz)
            .unwrap()
            .map(|t| t.to_rfc3339())
    };
    assert_eq!(
        next("2026-10-15T10:00:00+02:00").as_deref(),
        Some("2026-10-16T09:00:00+02:00")
    );
    assert_eq!(next("2026-10-16T09:00:00+02:00"), None);
    assert_eq!(
        due_slot(
            &s,
            &at("2026-10-14T10:00:00+02:00"),
            &at("2026-10-18T10:00:00+02:00"),
            &TestTz
        )
        .unwrap()
        .map(|t| t.to_rfc3339())
        .as_deref(),
        Some("2026-10-16T09:00:00+02:00")
    );

    let overlap = on(daily("09:00"), Some(Until::Date(local("2026-10-25T02:30"))));
    assert_eq!(
        overlap.until_instant(&TestTz).unwrap().to_rfc3339(),
        "2026-10-25T02:30:00+01:00",
        "inclusive: the later of the two instants"
    );
    let gap = on(daily("09:00"), Some(Until::Date(local("2026-03-29T02:30"))));
    assert_eq!(
        gap.until_instant(&TestTz).unwrap().to_rfc3339(),
        "2026-03-29T03:30:00+02:00"
    );
    assert_eq!(on(daily("09:00"), None).until_instant(&TestTz), None);
    assert_eq!(
        on(daily("09:00"), Some(Until::Count(3))).until_instant(&TestTz),
        None
    );
}

#[test]
fn next_slot_without_a_bound_is_the_first_slot() {
    let s = on(once("2026-10-20T09:00"), None);
    let next = |after: &str| next_slot(&s, &at(after), &TestTz).unwrap();
    assert_eq!(
        next("2026-10-14T10:00:00+02:00")
            .map(|t| t.to_rfc3339())
            .as_deref(),
        Some("2026-10-20T09:00:00+02:00")
    );
    assert_eq!(next("2026-10-20T09:00:00+02:00"), None);
}

#[test]
fn upcoming_honours_count_and_date_from_a_fresh_start() {
    let after = at("2026-10-14T10:00:00+02:00");
    let take = |s: &Schedule| rfc(upcoming(s, &after, &TestTz).unwrap().take(5));
    assert_eq!(
        take(&on(daily("09:00"), Some(Until::Count(2)))),
        ["2026-10-15T09:00:00+02:00", "2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(
        take(&on(
            daily("09:00"),
            Some(Until::Date(local("2026-10-17T08:00")))
        )),
        ["2026-10-15T09:00:00+02:00", "2026-10-16T09:00:00+02:00"]
    );
    assert_eq!(take(&on(daily("09:00"), None)).len(), 5);
    assert!(take(&on(once("2026-10-14T09:00"), None)).is_empty());
}

// --- The PC's zone ---

#[test]
fn slots_work_in_the_local_zone() {
    let now = Utc::now();
    for rule in [daily("09:00"), cron("0 0 9 * * *")] {
        let next = slots_after(&rule, &now, &Local).unwrap().next().unwrap();
        assert!(next > now && next <= now + TimeDelta::hours(26), "{next}");
        let prev = newest_slot_in(&rule, &(now - TimeDelta::days(2)), &now, &Local)
            .unwrap()
            .unwrap();
        assert!(prev <= now && prev > now - TimeDelta::hours(26), "{prev}");
    }
}

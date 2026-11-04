mod common;

use chrono::{NaiveTime, Weekday};
use common::tz::at;
use dots_core::model::WatchHours;
use dots_core::watchers::hours::{check_hours, is_inside, next_opening};
use dots_core::Error;
use serde_json::json;

fn hm(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s, "%H:%M").unwrap()
}

fn hours(days: &[Weekday], start: &str, end: &str) -> WatchHours {
    WatchHours {
        days: days.to_vec(),
        start: hm(start),
        end: hm(end),
    }
}

const WEEK: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

// 2026-11-02 is a Monday; winter time, so +01:00.

#[test]
fn a_same_day_range_includes_its_start_and_excludes_its_end() {
    let h = hours(&WEEK, "09:00", "18:00");
    assert!(!is_inside(&h, &at("2026-11-02T08:59:00+01:00")));
    assert!(is_inside(&h, &at("2026-11-02T09:00:00+01:00")));
    assert!(is_inside(&h, &at("2026-11-02T17:59:00+01:00")));
    assert!(!is_inside(&h, &at("2026-11-02T18:00:00+01:00")));
    assert!(!is_inside(&h, &at("2026-11-07T10:00:00+01:00")));
}

#[test]
fn an_overnight_range_belongs_to_the_day_it_starts() {
    let h = hours(&[Weekday::Fri], "22:00", "06:00");
    assert!(!is_inside(&h, &at("2026-11-06T21:59:00+01:00")));
    assert!(is_inside(&h, &at("2026-11-06T22:00:00+01:00")));
    assert!(is_inside(&h, &at("2026-11-06T23:00:00+01:00")));
    assert!(is_inside(&h, &at("2026-11-07T03:00:00+01:00")));
    assert!(!is_inside(&h, &at("2026-11-07T06:00:00+01:00")));
    assert!(!is_inside(&h, &at("2026-11-08T03:00:00+01:00")));
    // Friday's early morning belongs to Thursday, which is not allowed.
    assert!(!is_inside(&h, &at("2026-11-06T03:00:00+01:00")));
}

#[test]
fn next_opening_is_the_first_allowed_start_strictly_after() {
    let h = hours(&WEEK, "09:00", "18:00");
    let next = |s: &str| next_opening(&h, &at(s)).to_rfc3339();
    assert_eq!(
        next("2026-11-06T18:30:00+01:00"),
        "2026-11-09T09:00:00+01:00"
    );
    assert_eq!(
        next("2026-11-09T08:00:00+01:00"),
        "2026-11-09T09:00:00+01:00"
    );
    assert_eq!(
        next("2026-11-09T09:00:00+01:00"),
        "2026-11-10T09:00:00+01:00"
    );
}

#[test]
fn next_opening_moves_forward_over_a_summer_time_gap() {
    // 2026-03-29 is a Sunday; 02:00 to 03:00 does not exist.
    let h = hours(&[Weekday::Sun], "02:30", "09:00");
    let t = next_opening(&h, &at("2026-03-28T12:00:00+01:00"));
    assert_eq!(t.to_rfc3339(), "2026-03-29T03:30:00+02:00");
    assert_eq!(
        next_opening(&h, &at("2026-03-29T03:30:00+02:00")).to_rfc3339(),
        "2026-04-05T02:30:00+02:00"
    );
}

#[test]
fn hours_without_days_are_refused() {
    let err = check_hours(&hours(&[], "09:00", "18:00")).unwrap_err();
    assert!(matches!(&err, Error::Invalid(m) if m == "Watch hours need at least one day."));
}

#[test]
fn hours_with_equal_start_and_end_are_refused() {
    let err = check_hours(&hours(&WEEK, "09:00", "09:00")).unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "Watch hours need a start and an end that differ.")
    );
}

#[test]
fn a_repeated_day_is_refused() {
    let err = check_hours(&hours(&[Weekday::Mon, Weekday::Mon], "09:00", "18:00")).unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "Watch hours name the same day more than once.")
    );
}

#[test]
fn valid_hours_come_back_with_their_days_in_week_order() {
    let h = hours(
        &[Weekday::Fri, Weekday::Mon, Weekday::Wed],
        "09:00",
        "18:00",
    );
    let checked = check_hours(&h).unwrap();
    assert_eq!(checked.days, vec![Weekday::Mon, Weekday::Wed, Weekday::Fri]);
    assert_eq!((checked.start, checked.end), (h.start, h.end));
}

#[test]
fn hours_round_trip_through_json() {
    let h = hours(&[Weekday::Mon, Weekday::Fri], "09:00", "18:00");
    let v = serde_json::to_value(&h).unwrap();
    assert_eq!(
        v,
        json!({"days": ["mon", "fri"], "start": "09:00", "end": "18:00"})
    );
    assert_eq!(serde_json::from_value::<WatchHours>(v).unwrap(), h);
}

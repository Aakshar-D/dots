//! A hand-written time zone for schedule tests, so they do not depend on the PC's zone.
//!
//! `TestTz` is UTC+1, with summer time (UTC+2) from 2026-03-29 01:00 UTC to 2026-10-25
//! 01:00 UTC: local clocks jump from 02:00 to 03:00 on 29 March 2026 (a one-hour gap) and
//! fall back from 03:00 to 02:00 on 25 October 2026 (a one-hour overlap). Other years have
//! no summer time.

use chrono::{DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestTz;

fn utc(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap()
}

fn winter() -> FixedOffset {
    FixedOffset::east_opt(3600).unwrap()
}

fn summer() -> FixedOffset {
    FixedOffset::east_opt(7200).unwrap()
}

impl TimeZone for TestTz {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        TestTz
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        // The larger offset first: it gives the earlier instant.
        let fits: Vec<FixedOffset> = [summer(), winter()]
            .into_iter()
            .filter(|o| {
                let at = *local - TimeDelta::seconds(o.local_minus_utc() as i64);
                self.offset_from_utc_datetime(&at) == *o
            })
            .collect();
        match fits[..] {
            [o] => LocalResult::Single(o),
            [earliest, latest] => LocalResult::Ambiguous(earliest, latest),
            _ => LocalResult::None,
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, at: &NaiveDateTime) -> FixedOffset {
        if *at >= utc("2026-03-29T01:00") && *at < utc("2026-10-25T01:00") {
            summer()
        } else {
            winter()
        }
    }
}

/// An instant written in RFC 3339 with its offset, e.g. `2026-10-25T02:30:00+02:00`.
pub fn at(s: &str) -> DateTime<TestTz> {
    DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&TestTz)
}

/// Instants as RFC 3339 with their `TestTz` offset, for readable assertions.
pub fn rfc(times: impl IntoIterator<Item = DateTime<TestTz>>) -> Vec<String> {
    times.into_iter().map(|t| t.to_rfc3339()).collect()
}

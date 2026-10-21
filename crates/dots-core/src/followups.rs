//! Follow-ups (check-ins spec sections 3 and 6): runs a dot schedules for itself. A follow-up
//! continues the thread of the run that scheduled it, with the dot's note, once it is due.
//! This module holds the limits, checked when the tool is called, and the follow-up loop.

use chrono::{DateTime, TimeDelta, Utc};

use crate::checkins::When;
use crate::{Error, Result};

/// A follow-up is due at least this long after it is scheduled.
pub const MIN_AHEAD: TimeDelta = TimeDelta::minutes(5);
/// A follow-up is due at most this long after it is scheduled.
pub const MAX_AHEAD: TimeDelta = TimeDelta::days(30);
/// The most pending follow-ups a dot may have.
pub const MAX_PENDING: usize = 3;
/// A follow-up that fires later than this after it was due says how late it is.
pub const LATE_AFTER: TimeDelta = TimeDelta::minutes(15);
/// How long after each failed start the next try waits. The start after the last of them is
/// the last: when it fails too, the follow-up is skipped.
pub const RETRY_DELAYS: [TimeDelta; 3] = [
    TimeDelta::minutes(1),
    TimeDelta::minutes(5),
    TimeDelta::minutes(30),
];

pub const TOO_SOON: &str = "A follow-up must be at least 5 minutes away.";
pub const TOO_FAR: &str = "A follow-up must be at most 30 days away.";
pub const TOO_MANY: &str =
    "This dot already has 3 pending follow-ups; that is the most it may have.";

/// When a follow-up asked for at `now` is due, or the limit it breaks (section 3, in order:
/// `MIN_AHEAD`, then `MAX_AHEAD`; `MAX_PENDING` is checked when it is stored).
pub fn due_at(when: When, now: DateTime<Utc>) -> std::result::Result<DateTime<Utc>, &'static str> {
    let ahead = match when {
        When::At(t) => t - now,
        When::In(d) => d,
    };
    if ahead < MIN_AHEAD {
        return Err(TOO_SOON);
    }
    if ahead > MAX_AHEAD {
        return Err(TOO_FAR);
    }
    Ok(now + ahead)
}

/// A time the store wrote (`util::timestamp`).
pub fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| Error::Invalid(format!("stored time {s:?}: {e}")))
}

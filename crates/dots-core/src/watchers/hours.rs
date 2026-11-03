//! Watch hours (watch hours spec): the check a watcher's hours pass before they are saved, and
//! the maths of when they are open. Times are the wall clock of the time zone given, which is
//! this computer's local time in production.

use chrono::{DateTime, Datelike, NaiveDate, TimeDelta, TimeZone, Utc, Weekday};

use crate::model::WatchHours;
use crate::schedule::resolve;
use crate::{Error, Result};

/// The hours as they will be saved: days in week order. Refused when no day is named, a day
/// is named twice, or the range starts and ends at the same time.
pub fn check_hours(h: &WatchHours) -> Result<WatchHours> {
    if h.days.is_empty() {
        return Err(Error::Invalid("Watch hours need at least one day.".into()));
    }
    if h.start == h.end {
        return Err(Error::Invalid(
            "Watch hours need a start and an end that differ.".into(),
        ));
    }
    let mut days = h.days.clone();
    days.sort_by_key(Weekday::num_days_from_monday);
    if days.windows(2).any(|w| w[0] == w[1]) {
        return Err(Error::Invalid(
            "Watch hours name the same day more than once.".into(),
        ));
    }
    Ok(WatchHours {
        days,
        start: h.start,
        end: h.end,
    })
}

/// Whether `at` is inside the hours. An overnight range (start after end) belongs to the day
/// it starts: the early hours are inside when the day before is an allowed day.
pub fn is_inside<Tz: TimeZone>(h: &WatchHours, at: &DateTime<Tz>) -> bool {
    let local = at.naive_local();
    let (day, time) = (local.date(), local.time());
    let allowed = |d: NaiveDate| h.days.contains(&d.weekday());
    if h.start < h.end {
        allowed(day) && time >= h.start && time < h.end
    } else {
        (time >= h.start && allowed(day)) || (time < h.end && day.pred_opt().is_some_and(allowed))
    }
}

/// The first `start` of an allowed day strictly after `after`, as an instant in `after`'s
/// zone (a start inside a summer-time gap moves forward by the gap).
pub fn next_opening<Tz: TimeZone + 'static>(h: &WatchHours, after: &DateTime<Tz>) -> DateTime<Tz> {
    let tz = after.timezone();
    let first = after.naive_local().date() - TimeDelta::days(1);
    // Eight days cover a full week plus the margin for offset changes.
    (0..9)
        .map(|n| first + TimeDelta::days(n))
        .filter(|d| h.days.contains(&d.weekday()))
        .filter_map(|d| resolve(&tz, d.and_time(h.start)))
        .find(|t| t > after)
        .unwrap_or_else(|| after.clone())
}

/// The time zone a watcher's hours are read in: `chrono::Local` in the app, a fixed zone in
/// tests. Any chrono time zone is one.
pub trait Zone: Send + Sync {
    /// Whether the instant `at` is inside the hours, on this zone's wall clock.
    fn is_inside(&self, hours: &WatchHours, at: DateTime<Utc>) -> bool;
    /// The hours' first opening strictly after `after`.
    fn next_opening(&self, hours: &WatchHours, after: DateTime<Utc>) -> DateTime<Utc>;
    /// The instant in RFC 3339 with this zone's offset, to the second.
    fn rfc3339(&self, at: DateTime<Utc>) -> String;
}

impl<Tz> Zone for Tz
where
    Tz: TimeZone + Send + Sync + 'static,
    Tz::Offset: std::fmt::Display,
{
    fn is_inside(&self, hours: &WatchHours, at: DateTime<Utc>) -> bool {
        is_inside(hours, &at.with_timezone(self))
    }

    fn next_opening(&self, hours: &WatchHours, after: DateTime<Utc>) -> DateTime<Utc> {
        next_opening(hours, &after.with_timezone(self)).with_timezone(&Utc)
    }

    fn rfc3339(&self, at: DateTime<Utc>) -> String {
        at.with_timezone(self)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
    }
}

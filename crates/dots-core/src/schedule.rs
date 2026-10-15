//! A dot's schedule (spec "Scheduler v2" §2) and its slots (§3): the instants at which it is
//! due. Pure: every function takes the time and the time zone as arguments, so the scheduler
//! passes `Local` and tests pass a zone of their own.

use chrono::{
    DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeDelta,
    TimeZone, Utc, Weekday,
};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub rule: Rule,
    /// One run for the most recent missed slot when Dots comes back. Default true.
    #[serde(default = "yes")]
    pub catch_up: bool,
    /// None: no bound. Not allowed with `Rule::Once`.
    #[serde(default)]
    pub until: Option<Until>,
}

fn yes() -> bool {
    true
}

/// JSON: `{"kind": "daily", "time": "09:00"}`. Times are `HH:MM` and date-times
/// `YYYY-MM-DDTHH:MM`, both local and without an offset; `anchor` is RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rule {
    /// Once, at a local date and time.
    Once {
        #[serde(with = "local_date_time")]
        at: NaiveDateTime,
    },
    /// Every `minutes` (1 to 10080) of elapsed time, phase counted from `anchor`. The core sets
    /// `anchor` (`Schedule::fill_anchor`); the UI never sends it.
    Every {
        minutes: u32,
        #[serde(default)]
        anchor: Option<DateTime<Utc>>,
    },
    /// Every day at a local time.
    Daily {
        #[serde(with = "local_time")]
        time: NaiveTime,
    },
    /// On the given weekdays (at least one; `"mon"` to `"sun"`) at a local time.
    Weekly {
        #[serde(with = "weekdays")]
        days: Vec<Weekday>,
        #[serde(with = "local_time")]
        time: NaiveTime,
    },
    /// A 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
    Cron { expr: String },
}

/// JSON: `{"count": 10}` or `{"date": "2026-10-20T09:00"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Until {
    /// Stop after this many occurrences (at least 1).
    Count(u32),
    /// Stop after this local date and time, inclusive.
    Date(#[serde(with = "local_date_time")] NaiveDateTime),
}

/// Slots in order, each strictly later than the one before.
pub type Slots<Tz> = Box<dyn Iterator<Item = DateTime<Tz>>>;

pub const MAX_EVERY_MINUTES: u32 = 7 * 24 * 60;

fn invalid<T>(m: &str) -> Result<T> {
    Err(Error::Invalid(m.to_string()))
}

impl Schedule {
    /// A cron schedule as Dots had them before Scheduler v2: no catch-up, no bound. Migration
    /// 0005 converts a stored cron string to this.
    pub fn cron(expr: &str) -> Schedule {
        Schedule {
            rule: Rule::Cron {
                expr: expr.to_string(),
            },
            catch_up: false,
            until: None,
        }
    }

    /// The structural rules of spec §2. Times and dates that do not parse are refused earlier,
    /// when the JSON is read.
    pub fn validate(&self) -> Result<()> {
        match &self.rule {
            Rule::Every { minutes, .. } if !(1..=MAX_EVERY_MINUTES).contains(minutes) => {
                return invalid("every: minutes must be 1 to 10080 (7 days)");
            }
            Rule::Weekly { days, .. } if days.is_empty() => {
                return invalid("weekly: pick at least one day");
            }
            Rule::Cron { expr } => {
                crate::scheduler::parse_cron(expr)?;
            }
            _ => {}
        }
        match (&self.rule, &self.until) {
            (Rule::Once { .. }, Some(_)) => invalid("once: a stop (until) is not allowed"),
            (_, Some(Until::Count(0))) => invalid("until: count must be at least 1"),
            _ => Ok(()),
        }
    }

    /// Refuses a schedule with no slot after `now` within its `until` date: a past `Once`, an
    /// `Until::Date` before the first slot, a cron expression whose years are over. Used when a
    /// dot is created or its schedule changes, not when it is loaded. An `Every` without an
    /// anchor is checked as if it were saved now.
    pub fn check_runs_after<A: TimeZone, Tz: TimeZone + 'static>(
        &self,
        now: &DateTime<A>,
        tz: &Tz,
    ) -> Result<()> {
        let mut schedule = self.clone();
        schedule.fill_anchor(None, now.with_timezone(&Utc));
        match next_slot(&schedule, now, tz)? {
            Some(_) => Ok(()),
            None => invalid("this schedule would never run"),
        }
    }

    /// Sets `Every.anchor` as the core does on save: a schedule without one keeps the previous
    /// schedule's anchor while `minutes` stay the same, and starts at `now` otherwise. An
    /// anchor sent back unchanged with new minutes starts at `now` too. Other rules are left
    /// as they are.
    pub fn fill_anchor(&mut self, previous: Option<&Schedule>, now: DateTime<Utc>) {
        let Rule::Every { minutes, anchor } = &mut self.rule else {
            return;
        };
        let before = match previous.map(|p| &p.rule) {
            Some(Rule::Every {
                minutes: m,
                anchor: a,
            }) => Some((*m, *a)),
            _ => None,
        };
        *anchor = match (*anchor, before) {
            (None, Some((m, Some(a)))) if m == *minutes => Some(a),
            (None, _) => Some(now),
            (Some(a), Some((m, Some(b)))) if a == b && m != *minutes => Some(now),
            (Some(a), _) => Some(a),
        };
    }

    /// What the core does to a schedule that is saved (spec §2): `fill_anchor` from
    /// `previous`, the stored schedule (`None` on create), then `check_runs_after(now)` when the
    /// schedule is new or changed. A schedule saved unchanged is not checked again, so a dot
    /// whose `Once` has passed can still be edited.
    pub fn prepare_save<Tz: TimeZone + 'static>(
        &mut self,
        previous: Option<&Schedule>,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> Result<()> {
        self.fill_anchor(previous, now);
        if previous != Some(self) {
            self.check_runs_after(&now, tz)?;
        }
        Ok(())
    }

    /// The instant of `Until::Date` in `tz`: a time that occurs twice is its later instant (the
    /// bound is inclusive), a time in a gap moves forward like a slot.
    pub fn until_instant<Tz: TimeZone>(&self, tz: &Tz) -> Option<DateTime<Tz>> {
        match self.until {
            Some(Until::Date(date)) => match tz.from_local_datetime(&date) {
                LocalResult::Ambiguous(_, latest) => Some(latest),
                _ => resolve(tz, date),
            },
            _ => None,
        }
    }
}

/// The slots of `rule` strictly after `after`, in `tz`, in order (spec §3). `Invalid` for a
/// cron expression that does not parse and for an `Every` without an anchor.
pub fn slots_after<A: TimeZone, Tz: TimeZone + 'static>(
    rule: &Rule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Slots<Tz>> {
    let after = after.with_timezone(tz);
    Ok(match rule {
        Rule::Once { at } => Box::new(resolve(tz, *at).filter(|t| *t > after).into_iter()),
        Rule::Every { minutes, anchor } => {
            let (anchor, step) = every_parts(*minutes, *anchor)?;
            let first = slot_index_upto(anchor, step, &after.with_timezone(&Utc)) + 1;
            let tz = tz.clone();
            Box::new(
                (first..)
                    .map_while(move |k| every_slot(anchor, step, k))
                    .map(move |t| t.with_timezone(&tz)),
            )
        }
        Rule::Daily { time } => wall_clock_slots(tz, after, *time, None),
        Rule::Weekly { days, time } => wall_clock_slots(tz, after, *time, Some(days.clone())),
        Rule::Cron { expr } => {
            let tz = tz.clone();
            let start = after.naive_local() - CRON_MARGIN;
            Box::new(
                crate::scheduler::parse_cron(expr)?
                    .after_owned(start.and_utc())
                    .filter_map(move |wall| cron_resolve(&tz, wall.naive_utc()))
                    .filter(move |t| *t > after),
            )
        }
    })
}

/// The newest slot of `rule` with `after < slot <= upto`, in `tz`. It does not walk the
/// window from its start, so a long window (Dots closed for months) is cheap.
pub fn newest_slot_in<A: TimeZone, B: TimeZone, Tz: TimeZone + 'static>(
    rule: &Rule,
    after: &DateTime<A>,
    upto: &DateTime<B>,
    tz: &Tz,
) -> Result<Option<DateTime<Tz>>> {
    let after = after.with_timezone(tz);
    let upto = upto.with_timezone(tz);
    if upto <= after {
        return Ok(None);
    }
    let newest = match rule {
        Rule::Once { at } => resolve(tz, *at).filter(|t| *t <= upto),
        Rule::Every { minutes, anchor } => {
            let (anchor, step) = every_parts(*minutes, *anchor)?;
            let k = slot_index_upto(anchor, step, &upto.with_timezone(&Utc));
            every_slot(anchor, step, k).map(|t| t.with_timezone(tz))
        }
        Rule::Daily { time } => newest_wall_clock(tz, &after, &upto, *time, None),
        Rule::Weekly { days, time } => {
            newest_wall_clock(tz, &after, &upto, *time, Some(days.as_slice()))
        }
        Rule::Cron { expr } => {
            // Walk the wall clock backwards from past `upto`; the first slot at or before it
            // is the newest, the same set `slots_after` walks forward.
            let start = upto.naive_local() + CRON_MARGIN + TimeDelta::nanoseconds(1);
            let mut back = crate::scheduler::parse_cron(expr)?.after_owned(start.and_utc());
            std::iter::from_fn(|| back.next_back())
                .filter_map(|wall| cron_resolve(tz, wall.naive_utc()))
                .find(|t| *t <= upto)
        }
    };
    Ok(newest.filter(|t| *t > after))
}

/// The first slot of `schedule` after `after`, or `None` when none is left within its
/// `Until::Date`. `Until::Count` is the scheduler's (it knows how many slots fired).
pub fn next_slot<A: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Option<DateTime<Tz>>> {
    Ok(upcoming_within_date(schedule, after, tz)?.next())
}

/// `newest_slot_in` for a schedule: the newest slot in `after < slot <= upto` that is within
/// its `Until::Date`.
pub fn due_slot<A: TimeZone, B: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    upto: &DateTime<B>,
    tz: &Tz,
) -> Result<Option<DateTime<Tz>>> {
    let upto = upto.with_timezone(tz);
    let upto = match schedule.until_instant(tz) {
        Some(until) if until < upto => until,
        _ => upto,
    };
    newest_slot_in(&schedule.rule, after, &upto, tz)
}

/// The slots of a schedule that starts after `after` (a preview): within its `Until::Date`
/// and at most `Until::Count` of them.
pub fn upcoming<A: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Slots<Tz>> {
    let slots = upcoming_within_date(schedule, after, tz)?;
    Ok(match schedule.until {
        Some(Until::Count(n)) => Box::new(slots.take(n as usize)),
        _ => slots,
    })
}

fn upcoming_within_date<A: TimeZone, Tz: TimeZone + 'static>(
    schedule: &Schedule,
    after: &DateTime<A>,
    tz: &Tz,
) -> Result<Slots<Tz>> {
    let slots = slots_after(&schedule.rule, after, tz)?;
    Ok(match schedule.until_instant(tz) {
        Some(until) => Box::new(slots.take_while(move |t| *t <= until)),
        None => slots,
    })
}

/// A local date and time as an instant in `tz`, with the DST rules of spec §3: a time that
/// occurs twice is its earlier instant; a time in a gap moves forward by the length of the gap
/// (02:30 on a night when clocks go from 02:00 to 03:00 becomes 03:30).
fn resolve<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => Some(t),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => {
            // Read the time with the offset from before the gap.
            let before = tz
                .from_local_datetime(&(local - TimeDelta::days(1)))
                .earliest()?;
            let offset = TimeDelta::seconds(before.offset().fix().local_minus_utc() as i64);
            Some(tz.from_utc_datetime(&(local - offset)))
        }
    }
}

/// How far from a wall-clock time the cron walk starts, so that an offset change of a few
/// hours cannot hide a slot.
const CRON_MARGIN: TimeDelta = TimeDelta::hours(3);

/// A wall-clock time matched by a cron expression as an instant in `tz`: a time that occurs
/// twice is its earlier instant, a time in a gap does not exist and is skipped (spec §3).
fn cron_resolve<Tz: TimeZone>(tz: &Tz, wall: NaiveDateTime) -> Option<DateTime<Tz>> {
    match tz.from_local_datetime(&wall) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t),
        LocalResult::None => None,
    }
}

fn every_parts(minutes: u32, anchor: Option<DateTime<Utc>>) -> Result<(DateTime<Utc>, TimeDelta)> {
    let Some(anchor) = anchor else {
        return invalid("every: the schedule has no anchor yet");
    };
    if minutes == 0 {
        return invalid("every: minutes must be 1 to 10080 (7 days)");
    }
    Ok((anchor, TimeDelta::minutes(minutes as i64)))
}

fn every_slot(anchor: DateTime<Utc>, step: TimeDelta, k: i64) -> Option<DateTime<Utc>> {
    let ms = step.num_milliseconds().checked_mul(k)?;
    anchor.checked_add_signed(TimeDelta::try_milliseconds(ms)?)
}

/// The largest `k` with `anchor + k * step <= t`.
fn slot_index_upto(anchor: DateTime<Utc>, step: TimeDelta, t: &DateTime<Utc>) -> i64 {
    let mut k = (*t - anchor)
        .num_milliseconds()
        .div_euclid(step.num_milliseconds());
    // Milliseconds drop the nanoseconds: settle the last step exactly.
    while every_slot(anchor, step, k).is_some_and(|s| s > *t) {
        k -= 1;
    }
    while every_slot(anchor, step, k + 1).is_some_and(|s| s <= *t) {
        k += 1;
    }
    k
}

fn on_day(days: Option<&[Weekday]>, date: NaiveDate) -> bool {
    days.is_none_or(|d| d.contains(&date.weekday()))
}

fn wall_clock_slots<Tz: TimeZone + 'static>(
    tz: &Tz,
    after: DateTime<Tz>,
    time: NaiveTime,
    days: Option<Vec<Weekday>>,
) -> Slots<Tz> {
    if days.as_ref().is_some_and(|d| d.is_empty()) {
        return Box::new(std::iter::empty());
    }
    // From the day before: a time in a gap can move past midnight.
    let start = after
        .naive_local()
        .date()
        .pred_opt()
        .unwrap_or(NaiveDate::MIN);
    let tz = tz.clone();
    Box::new(
        start
            .iter_days()
            .filter(move |d| on_day(days.as_deref(), *d))
            .filter_map(move |d| resolve(&tz, d.and_time(time)))
            .filter(move |t| *t > after),
    )
}

fn newest_wall_clock<Tz: TimeZone>(
    tz: &Tz,
    after: &DateTime<Tz>,
    upto: &DateTime<Tz>,
    time: NaiveTime,
    days: Option<&[Weekday]>,
) -> Option<DateTime<Tz>> {
    // From the day after `upto` back to the day before `after`, to cover a moved time.
    let first = after.naive_local().date().pred_opt()?;
    let last = upto.naive_local().date().succ_opt()?;
    std::iter::successors(Some(last), |d| d.pred_opt())
        .take_while(|d| *d >= first)
        .filter(|d| on_day(days, *d))
        .filter_map(|d| resolve(tz, d.and_time(time)))
        .find(|t| t <= upto)
}

mod local_time {
    use chrono::NaiveTime;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&t.format("%H:%M"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let s = String::deserialize(d)?;
        NaiveTime::parse_from_str(&s, "%H:%M")
            .map_err(|_| D::Error::custom(format!("time {s:?}: expected HH:MM")))
    }
}

mod local_date_time {
    use chrono::NaiveDateTime;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&t.format("%Y-%m-%dT%H:%M"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
        let s = String::deserialize(d)?;
        NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M")
            .map_err(|_| D::Error::custom(format!("date {s:?}: expected YYYY-MM-DDTHH:MM")))
    }
}

mod weekdays {
    use chrono::Weekday;
    use serde::de::Error as _;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};

    const NAMES: [(&str, Weekday); 7] = [
        ("mon", Weekday::Mon),
        ("tue", Weekday::Tue),
        ("wed", Weekday::Wed),
        ("thu", Weekday::Thu),
        ("fri", Weekday::Fri),
        ("sat", Weekday::Sat),
        ("sun", Weekday::Sun),
    ];

    pub fn serialize<S: Serializer>(days: &[Weekday], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(days.len()))?;
        for day in days {
            seq.serialize_element(NAMES[day.num_days_from_monday() as usize].0)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Weekday>, D::Error> {
        Vec::<String>::deserialize(d)?
            .into_iter()
            .map(|s| {
                NAMES
                    .iter()
                    .find(|(name, _)| *name == s)
                    .map(|(_, day)| *day)
                    .ok_or_else(|| D::Error::custom(format!("day {s:?}: expected mon to sun")))
            })
            .collect()
    }
}

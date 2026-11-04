//! Send later (send later spec): the pure parts of sending a scheduled message, and what a
//! chat run carries besides its text. The runtime does the sending (`Runtime::send_scheduled`)
//! and runs the sender loop (`Runtime::spawn_send_later`).

use chrono::{DateTime, TimeDelta, Utc};

use crate::model::{Run, ScheduledStatus, ScheduledWait, SentLater};
use crate::util::timestamp;
use crate::{Error, Result};

/// Sent more than this after its due time, a message is late.
pub const LATE_AFTER: TimeDelta = TimeDelta::seconds(60);
/// How soon a message may be scheduled, at the least.
pub const MIN_AHEAD: TimeDelta = TimeDelta::minutes(1);
/// How far ahead a message may be scheduled, at the most.
pub const MAX_AHEAD: TimeDelta = TimeDelta::days(365);
/// The sender's longest sleep: it retries waiting messages and catches up after the PC slept.
pub const SEND_EVERY: TimeDelta = TimeDelta::seconds(30);

pub const TOO_SOON: &str = "Pick a time at least a minute from now.";
pub const TOO_FAR: &str = "Pick a time within the next year.";
pub const BEING_SENT: &str = "This message is being sent now.";
pub const ALREADY_SENT: &str = "This message has already been sent.";
pub const CANCELLED: &str = "This message was cancelled.";

/// What a chat run carries besides its text (send later spec section 4.3). It is where the
/// files of a message will travel too (attachments): `Runtime::chat_extra_for` is the one place
/// a scheduled message's extras are built.
#[derive(Default, Clone, Debug)]
pub struct ChatExtra {
    /// The note of a scheduled message, written into the run's payload as `send_later`.
    pub send_later: Option<SentLater>,
    /// The staged files the message carries (attachments spec section 4.3), by id.
    pub attachment_ids: Vec<String>,
    /// The scheduled message whose held files these are: they may be held by it.
    pub scheduled_id: Option<String>,
    /// Retry: the message whose files move to the new one, keeping their names.
    pub carry_from: Option<String>,
}

/// Where a due message goes, decided at send time.
#[derive(Debug, Clone, PartialEq)]
pub enum SendTarget {
    /// A reply after the topic's end.
    Reply { parent_run_id: String },
    /// A new topic; `fallback` when the message had a topic that cannot be continued.
    New { fallback: bool },
    /// Not now: tried again on the next pass.
    Wait(ScheduledWait),
}

/// Checks a due time for scheduling at `now`: RFC 3339, from a minute to a year ahead. Returns
/// it as stored (UTC, milliseconds).
pub fn check_due(due_at: &str, now: DateTime<Utc>) -> Result<String> {
    let due = DateTime::parse_from_rfc3339(due_at)
        .map_err(|_| Error::Invalid("due_at is not a time".into()))?
        .with_timezone(&Utc);
    if due < now + MIN_AHEAD {
        return Err(Error::Invalid(TOO_SOON.into()));
    }
    if due > now + MAX_AHEAD {
        return Err(Error::Invalid(TOO_FAR.into()));
    }
    Ok(timestamp(due))
}

/// The end of a thread for the chat path: its last run that is not a message dismissed as not
/// delivered (delivery tracking spec section 3.9).
pub fn thread_end(thread: &[Run]) -> Option<&Run> {
    thread
        .iter()
        .rfind(|r| r.dismissed_at.is_none() || r.delivered_at.is_some())
}

/// Whether a message due at `due_at` and sent at `sent_at` went late. An unreadable `due_at`
/// is not late.
pub fn is_late(due_at: &str, sent_at: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(due_at)
        .map(|due| sent_at - due.with_timezone(&Utc) > LATE_AFTER)
        .unwrap_or(false)
}

/// The refusal of Edit, Cancel or Send now for a message in `status`.
pub fn not_changeable(status: ScheduledStatus) -> Error {
    let text = match status {
        ScheduledStatus::Sending => BEING_SENT,
        ScheduledStatus::Sent => ALREADY_SENT,
        ScheduledStatus::Cancelled => CANCELLED,
        // Not reached: these can be changed. Kept as the race it would be.
        ScheduledStatus::Scheduled | ScheduledStatus::Failed => BEING_SENT,
    };
    Error::Conflict(text.into())
}

/// The first line of an error's text, for a failed message.
pub fn first_line(e: &Error) -> String {
    let text = match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m.clone(),
        other => other.to_string(),
    };
    text.lines().next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-04T09:00:00.000Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn invalid(r: Result<String>) -> String {
        match r {
            Err(Error::Invalid(m)) => m,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn check_due_refuses_past_near_and_far() {
        let soon = timestamp(t0() + TimeDelta::seconds(30));
        assert_eq!(invalid(check_due(&soon, t0())), TOO_SOON);
        let past = timestamp(t0() - TimeDelta::hours(1));
        assert_eq!(invalid(check_due(&past, t0())), TOO_SOON);
        let far = timestamp(t0() + TimeDelta::days(366));
        assert_eq!(invalid(check_due(&far, t0())), TOO_FAR);
        assert_eq!(invalid(check_due("soon", t0())), "due_at is not a time");
        assert_eq!(
            check_due("2026-11-04T11:00:00+01:00", t0()).unwrap(),
            "2026-11-04T10:00:00.000Z"
        );
        assert_eq!(
            check_due(&timestamp(t0() + TimeDelta::minutes(1)), t0()).unwrap(),
            "2026-11-04T09:01:00.000Z"
        );
    }

    #[test]
    fn late_is_more_than_a_minute() {
        let due = timestamp(t0());
        assert!(!is_late(&due, t0() + TimeDelta::seconds(30)));
        assert!(!is_late(&due, t0() + TimeDelta::seconds(60)));
        assert!(is_late(&due, t0() + TimeDelta::minutes(2)));
    }

    #[test]
    fn refusals_name_the_state() {
        let text = |s| match not_changeable(s) {
            Error::Conflict(m) => m,
            other => panic!("{other:?}"),
        };
        assert_eq!(text(ScheduledStatus::Sending), BEING_SENT);
        assert_eq!(text(ScheduledStatus::Sent), ALREADY_SENT);
        assert_eq!(text(ScheduledStatus::Cancelled), CANCELLED);
    }
}

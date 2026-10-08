use std::str::FromStr;

use chrono::{DateTime, Local};

use crate::{Error, Result};

/// Parses a 6-field (sec min hour day month weekday) or 7-field (+ year) cron expression.
pub fn parse_cron(expr: &str) -> Result<cron::Schedule> {
    let fields = expr.split_whitespace().count();
    if !(6..=7).contains(&fields) {
        return Err(Error::Invalid(format!(
            "cron {expr:?}: expected 6 fields (sec min hour day month weekday) or 7 with year"
        )));
    }
    cron::Schedule::from_str(expr).map_err(|e| Error::Invalid(format!("cron {expr:?}: {e}")))
}

/// Next fire time strictly after `after`, in local time.
pub fn next_fire(expr: &str, after: DateTime<Local>) -> Result<Option<DateTime<Local>>> {
    Ok(parse_cron(expr)?.after(&after).next())
}

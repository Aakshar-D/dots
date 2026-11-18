//! The person's goals (Intents): what a goal may hold, and the sentences for what is refused.
//! The runtime checks the account role and the linked dot; the store keeps the rows.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::model::Intent;
use crate::{Error, Result};

/// The longest goal, in characters after trimming.
pub const MAX_TEXT: usize = 500;

pub const TEXT_RULE: &str = "A goal needs between 1 and 500 characters.";
pub const BAD_DATE: &str = "A due date must be a real calendar date, like 2026-11-05.";
pub const NO_DOT: &str = "That sol no longer exists. Pick another one.";
pub const OTHER_WORKSPACE: &str = "That sol is in another workspace. Pick one from this workspace.";
pub const GONE: &str = "That goal no longer exists.";

/// What the person writes for a goal, on create and on every change: a change replaces all
/// four. `due_date` is `YYYY-MM-DD`; blank is no date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentFields {
    pub text: String,
    pub due_date: Option<String>,
    pub urgent: bool,
    pub dot_id: Option<String>,
}

/// `fields` as stored: the text trimmed, the date checked and written `YYYY-MM-DD`, a blank
/// date or dot as none. Refused with `TEXT_RULE` or `BAD_DATE`.
pub fn clean(fields: IntentFields) -> Result<IntentFields> {
    let text = fields.text.trim().to_string();
    let len = text.chars().count();
    if len == 0 || len > MAX_TEXT {
        return Err(Error::Invalid(TEXT_RULE.into()));
    }
    let due_date = match fields.due_date.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(d) => Some(
            NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .map_err(|_| Error::Invalid(BAD_DATE.into()))?
                .format("%Y-%m-%d")
                .to_string(),
        ),
    };
    let dot_id = fields
        .dot_id
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty());
    Ok(IntentFields {
        text,
        due_date,
        urgent: fields.urgent,
        dot_id,
    })
}

/// The goals of one account's workspace, and why this PC may not change them (`None` when it
/// may): the Intents screen shows them read-only then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentsView {
    pub intents: Vec<Intent>,
    pub refusal: Option<String>,
}

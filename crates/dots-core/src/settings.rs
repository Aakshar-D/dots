//! App settings, stored in the `settings` table as one JSON value per key (spec §3).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::check_endpoint_url;
use crate::store::Store;
use crate::{Error, Result};

/// The `claude` CLI version the stream-json parser and the permission flow were last checked
/// against. A different installed version shows a warning until it is marked as tested.
pub const TESTED_CLAUDE_VERSION: &str = "2.1.295";
pub const MAX_CONCURRENT_RUNS: usize = 16;
/// The longest global instructions, in bytes (context library spec section 2).
pub const MAX_GLOBAL_INSTRUCTIONS: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub port: u16,
    pub max_concurrent_runs: usize,
    /// `None` searches PATH.
    pub claude_path: Option<String>,
    pub tested_claude_version: String,
    pub local_endpoints: Vec<String>,
    pub autostart: bool,
    pub schedules_paused: bool,
    /// The text every dot's first prompt starts with, unless the dot switches it off.
    pub global_instructions: String,
    /// A dot's note shows a notification (check-ins spec section 10). A question always does.
    pub notify_on_notes: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 47321,
            max_concurrent_runs: 2,
            claude_path: None,
            tested_claude_version: TESTED_CLAUDE_VERSION.to_string(),
            local_endpoints: Vec::new(),
            autostart: true,
            schedules_paused: false,
            global_instructions: String::new(),
            notify_on_notes: true,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
        if self.port == 0 {
            return invalid("port must be between 1 and 65535");
        }
        if !(1..=MAX_CONCURRENT_RUNS).contains(&self.max_concurrent_runs) {
            return invalid("max_concurrent_runs must be between 1 and 16");
        }
        if let Some(path) = &self.claude_path {
            if path.trim().is_empty() {
                return invalid("claude_path must not be empty (leave it unset to search PATH)");
            }
            check_plain_text("claude_path", path)?;
        }
        if self.tested_claude_version.trim().is_empty() {
            return invalid("tested_claude_version must not be empty");
        }
        for url in &self.local_endpoints {
            check_endpoint_url(url)?;
        }
        if self.global_instructions.len() > MAX_GLOBAL_INSTRUCTIONS {
            return invalid("global_instructions must be at most 32 KB");
        }
        Ok(())
    }

    /// Whether moving from `self` (what the running app started with) to `next` needs a
    /// restart: the port, concurrency and `claude` path are fixed when the runtime starts.
    pub fn restart_required(&self, next: &Settings) -> bool {
        self.port != next.port
            || self.max_concurrent_runs != next.max_concurrent_runs
            || self.claude_path != next.claude_path
    }
}

/// Refuses a value with a character that would show it as something other than it is. The
/// user confirms `claude_path` by reading it in a native dialog (spec §12), and that only works
/// if every character is seen, in the order it is stored. Three rules, named in the error with
/// the character and its position (counted from 1):
///
/// - no control characters, and no line or paragraph separator: a tab or a line break splits
///   the value or pushes a part of it out of sight;
/// - no format characters: they have no width, and some turn the text around them (U+202E
///   shows `exe.edualc` as `claude.exe`);
/// - no other character that draws nothing (see `is_ignorable_or_blank`): a path with one reads
///   like the path without it.
///
/// Letters of any script, spaces and the look-alikes of a space pass: a name that reads like
/// another one is not hidden, and no list of characters can rule that out.
fn check_plain_text(field: &str, value: &str) -> Result<()> {
    for (i, c) in value.chars().enumerate() {
        let rule = if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
            "control characters such as a tab or a line break"
        } else if is_format_char(c) || is_ignorable_or_blank(c) {
            "invisible or text-direction characters"
        } else {
            continue;
        };
        return Err(Error::Invalid(format!(
            "{field} must not contain {rule} (found U+{:04X} at character {})",
            c as u32,
            i + 1
        )));
    }
    Ok(())
}

/// Unicode general category Cf (format), which holds the zero-width characters, the byte order
/// mark, the soft hyphen, the tag characters and every bidirectional control. `std` knows only
/// the control category, so the ranges are listed here (Unicode 15.0; the code points not
/// assigned yet inside U+2060..U+206F and U+E0000..U+E007F are included).
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0000}'..='\u{e007f}'
    )
}

/// The characters that draw nothing and are not in category Cf: the rest of Unicode's
/// `Default_Ignorable_Code_Point` set (the combining grapheme joiner, the variation selectors,
/// the Hangul and Mongolian fillers and free variation selectors, the Khmer inherent vowels, the
/// unassigned code points among the specials and tags) and the blank braille pattern U+2800,
/// which `trim` keeps. A path with one reads in the dialog like the path without it, so it
/// could name another program than the one the user sees.
fn is_ignorable_or_blank(c: char) -> bool {
    matches!(
        c,
        '\u{34f}'
            | '\u{115f}'..='\u{1160}'
            | '\u{17b4}'..='\u{17b5}'
            | '\u{180b}'..='\u{180d}'
            | '\u{180f}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{ffa0}'
            | '\u{fff0}'..='\u{fff8}'
            | '\u{e0080}'..='\u{e0fff}'
    )
}

/// Reads the settings of the database in `data_dir` (created if missing) and closes that
/// store again, so the app can build its start configuration before the runtime opens it.
pub async fn load(data_dir: &Path) -> Result<Settings> {
    let store = Store::open(&data_dir.join(crate::store::DB_FILE)).await?;
    let settings = store.get_settings().await;
    store.close().await;
    settings
}

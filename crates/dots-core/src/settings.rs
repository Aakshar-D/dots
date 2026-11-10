//! App settings, stored in the `settings` table as one JSON value per key (spec §3).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::auto_model::AutoModels;
use crate::model::{check_endpoint_url, MAX_QUIET_LIMIT_SECS, MIN_QUIET_LIMIT_SECS};
use crate::models::DEFAULT_CLAUDE_MODEL;
use crate::store::Store;
use crate::{Error, Result};

/// The `claude` CLI version the stream-json parser and the permission flow were last checked
/// against. A different installed version shows a warning until it is marked as tested.
pub const TESTED_CLAUDE_VERSION: &str = "2.1.295";
pub const MAX_CONCURRENT_RUNS: usize = 16;
/// The longest global instructions, in bytes (context library spec section 2).
pub const MAX_GLOBAL_INSTRUCTIONS: usize = 32 * 1024;
/// The most days `auto_archive_days` takes (ten years).
pub const MAX_AUTO_ARCHIVE_DAYS: u32 = 3650;
/// The words when a hold share is out of bounds (usage and limits spec section 4).
pub const SHARE_RULE: &str = "A share is 10 to 100%, in steps of 5.";

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
    /// The Microsoft Edge that dots browse with (browser spec section 6). `None` uses the one
    /// installed under Program Files.
    pub edge_path: Option<String>,
    /// Background Claude runs wait while the account's Claude usage is at or above a share
    /// (usage and limits spec section 4).
    pub hold_enabled: bool,
    /// The 5-hour window's share, in percent (10 to 100, in steps of 5).
    pub hold_share_five_hour: u8,
    /// The weekly window's share, in percent (10 to 100, in steps of 5).
    pub hold_share_weekly: u8,
    /// Dots installs a downloaded update by itself when idle (updates UI spec section 4.2).
    pub auto_install_updates: bool,
    /// The vault entry holding the GitHub token Dots publishes with (updates UI spec section
    /// 5.1); `None` when this Dots does not publish.
    pub publish_token: Option<String>,
    /// How long a run may go without output before the watchdog stops it, unless the dot sets
    /// its own (stuck-run watchdog spec section 4.3). 60 to 86400.
    pub quiet_limit_secs: u64,
    /// A topic idle for this many days is archived (topic tools spec section 3.3); 0 never.
    pub auto_archive_days: u32,
    /// The model a new Claude dot starts with (model switcher spec section 2.2). Existing dots
    /// keep their own.
    pub default_model: String,
    /// "Answer from my phone" (phone view spec section 2): a listener on this PC's private
    /// network addresses serves the Inbox to paired phones. Off by default; applies at once.
    pub phone_enabled: bool,
    /// The phone view's port, not the local server's.
    pub phone_port: u16,
    /// Every Claude run's final answer gets a quick automatic check before it reaches the
    /// person (result check spec section 5); a dot can turn it off for itself.
    pub check_results: bool,
    /// The Claude model the result check uses: the cheapest by default.
    pub checker_model: String,
    /// The most every dot together may spend in a local day, in estimated API USD (spend caps
    /// design); `None` is no cap.
    pub spend_daily_usd: Option<f64>,
    /// The most tokens (in + out) every dot together may use in a local day; `None` is no cap.
    pub spend_daily_tokens: Option<u64>,
    /// The most every dot together may spend in a local calendar month, in estimated API USD.
    pub spend_monthly_usd: Option<f64>,
    /// The model of each size Auto picks from (auto model spec section 3).
    pub auto_models: AutoModels,
    /// When a Claude session is compacted or started fresh, unless its dot has rules of its
    /// own (context rules spec section 4).
    pub context_rules: crate::context_rules::ContextRules,
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
            edge_path: None,
            hold_enabled: true,
            hold_share_five_hour: 60,
            hold_share_weekly: 70,
            auto_install_updates: false,
            publish_token: None,
            quiet_limit_secs: 600,
            auto_archive_days: 30,
            default_model: DEFAULT_CLAUDE_MODEL.to_string(),
            phone_enabled: false,
            phone_port: crate::phone::DEFAULT_PORT,
            check_results: true,
            checker_model: crate::result_check::DEFAULT_CHECKER_MODEL.to_string(),
            spend_daily_usd: None,
            spend_daily_tokens: None,
            spend_monthly_usd: None,
            auto_models: AutoModels::default(),
            context_rules: crate::context_rules::ContextRules::default(),
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
        // Dots runs this program too, so it is confirmed and shown as `claude_path` is.
        if let Some(path) = &self.edge_path {
            if path.trim().is_empty() {
                return invalid(
                    "edge_path must not be empty (leave it unset to use the installed Microsoft Edge)",
                );
            }
            check_plain_text("edge_path", path)?;
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
        let share_ok = |s: u8| (10..=100).contains(&s) && s.is_multiple_of(5);
        if !share_ok(self.hold_share_five_hour) || !share_ok(self.hold_share_weekly) {
            return invalid(SHARE_RULE);
        }
        if !(MIN_QUIET_LIMIT_SECS..=MAX_QUIET_LIMIT_SECS).contains(&self.quiet_limit_secs) {
            return invalid("quiet_limit_secs must be between 60 and 86400");
        }
        if self.auto_archive_days > MAX_AUTO_ARCHIVE_DAYS {
            return invalid("auto_archive_days must be between 0 and 3650");
        }
        if self.default_model.trim().is_empty() {
            return invalid("default_model must not be empty");
        }
        if self.phone_port == 0 {
            return invalid("phone_port must be between 1 and 65535");
        }
        if self.phone_port == self.port {
            return invalid("phone_port must differ from the local server's port");
        }
        if self.checker_model.trim().is_empty() {
            return invalid("checker_model must not be empty");
        }
        crate::spend::GlobalCaps::from(self).validate()?;
        self.auto_models.validate()?;
        self.context_rules.validate()?;
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
pub(crate) fn check_plain_text(field: &str, value: &str) -> Result<()> {
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

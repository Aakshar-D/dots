//! Workspace ids (accounts design §6.3): made from a name, never changed after.

use super::DEFAULT_WORKSPACE;

/// The longest a workspace id may be (`^[a-z0-9][a-z0-9-]{0,31}$`).
const MAX_ID: usize = 32;

/// The id for a new workspace called `name`: its lower-case letters and digits, words joined by
/// `-` (`Payments Team` is `payments-team`), `workspace` when the name has none. A taken id
/// (`taken`, which includes `default`) gets `-2`, `-3`, ... after it. Never over 32 characters:
/// the name is cut short to make room.
pub fn workspace_id_for(name: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = if base.is_empty() { "workspace" } else { &base };
    let is_taken = |id: &str| id == DEFAULT_WORKSPACE || taken.iter().any(|t| t == id);
    let fit = |room: usize| base[..base.len().min(room)].trim_end_matches('-');
    if !is_taken(fit(MAX_ID)) {
        return fit(MAX_ID).to_string();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            format!("{}{suffix}", fit(MAX_ID - suffix.len()))
        })
        .find(|id| !is_taken(id))
        .expect("an unused number exists")
}

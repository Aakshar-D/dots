//! Workspace ids (accounts design §6.3): made from a name, never changed after.

use super::DEFAULT_WORKSPACE;

/// The id for a new workspace called `name`: its lower-case letters and digits, words joined by
/// `-` (`Payments Team` is `payments-team`), `workspace` when the name has none. A taken id
/// (`taken`, which includes `default`) gets `-2`, `-3`, ... after it.
pub fn workspace_id_for(name: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_end_matches('-');
    let base = if base.is_empty() { "workspace" } else { base };
    let is_taken = |id: &str| id == DEFAULT_WORKSPACE || taken.iter().any(|t| t == id);
    if !is_taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !is_taken(id))
        .expect("an unused number exists")
}

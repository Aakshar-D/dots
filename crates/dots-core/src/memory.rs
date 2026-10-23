//! Memory for each dot (dot memory spec): what a dot remembers across runs, one fact per memory.
//! This module holds the rules of a memory's text, `Memories`, which the person's commands and
//! the dot's tools go through, and the event every change publishes.

use std::sync::LazyLock;

use regex::Regex;

use crate::events::{Bus, RuntimeEvent};
use crate::model::{Memory, MemorySource};
use crate::store::{Added, Store};
use crate::{Error, Result};

/// The most memories a dot keeps.
pub const MAX_PER_DOT: usize = 200;
/// The longest memory, in characters after trimming.
pub const MAX_TEXT: usize = 500;

/// The refusal of a text that is blank or too long.
pub const LENGTH_RULE: &str = "A memory is 1 to 500 characters.";
/// The refusal of a text the dot remembers already.
pub const DUPLICATE: &str = "This dot already remembers that.";
/// The refusal of a memory past `MAX_PER_DOT`.
pub const FULL: &str = "This dot remembers 200 things already: forget something first.";
/// The refusal of a text that matches a secret pattern (`looks_secret`).
pub const SECRET: &str =
    "That looks like a secret. Don't store secrets in memory; ask the person to keep it in the vault.";

/// The secret patterns of spec section 6. A key prefix counts only at the start of a word, so
/// `task-…` is not `sk-…`.
static SECRET_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"-----BEGIN",
        r"|(?:^|[^A-Za-z0-9_-])(?:sk-|sk_live_|ghp_|gho_|github_pat_|xox[bp]-|AKIA)[A-Za-z0-9_-]{16,}",
        r"|(?i:bearer) [A-Za-z0-9._~+/=-]{20,}",
        r"|(?i:password|passwd|secret|token)\s*[=:]\s*\S{8,}",
    ))
    .expect("the secret patterns compile")
});

/// Whether `text` matches a secret pattern: a key, a private key block, a bearer token, or a
/// password given with `=` or `:`.
pub fn looks_secret(text: &str) -> bool {
    SECRET_PATTERN.is_match(text)
}

/// The memory text `raw` as it is stored: trimmed, 1 to `MAX_TEXT` characters, no secret. `Err`
/// is the rule it breaks, in the spec's sentence.
pub fn check_text(raw: &str) -> std::result::Result<String, &'static str> {
    let text = raw.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT {
        return Err(LENGTH_RULE);
    }
    if looks_secret(text) {
        return Err(SECRET);
    }
    Ok(text.to_string())
}

/// What two texts are compared by for a duplicate: without spaces, tabs and line ends, ASCII
/// letters in lower case (the store's `key_of` computes the same in SQL).
pub fn same_key(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r'))
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The dots' memories, for the person's commands and the dot's tools. Every change publishes
/// `MemoriesUpdated`. Cheap to clone.
#[derive(Clone)]
pub struct Memories {
    store: Store,
    bus: Bus,
}

impl Memories {
    pub fn new(store: Store, bus: Bus) -> Self {
        Self { store, bus }
    }

    /// The dot's memories, pinned first, then newest. `NotFound` for an unknown dot.
    pub async fn list(&self, dot_id: &str) -> Result<Vec<Memory>> {
        self.store.get_dot(dot_id).await?;
        self.store.list_memories(dot_id).await
    }

    /// Saves a memory of the dot, written by `source` (a run's id with `Dot`). The text rules
    /// are `Invalid`, a duplicate and the limit `Conflict`, each with the spec's sentence.
    pub async fn save(
        &self,
        dot_id: &str,
        text: &str,
        source: MemorySource,
        run_id: Option<&str>,
    ) -> Result<Memory> {
        let text = check_text(text).map_err(|rule| Error::Invalid(rule.into()))?;
        self.store.get_dot(dot_id).await?;
        let added = self
            .store
            .add_memory(dot_id, &text, &same_key(&text), source, run_id, MAX_PER_DOT)
            .await?;
        match added {
            Added::Saved(memory) => {
                self.changed(dot_id);
                Ok(memory)
            }
            Added::Duplicate => Err(Error::Conflict(DUPLICATE.into())),
            Added::Full => Err(Error::Conflict(FULL.into())),
        }
    }

    /// The person's memory of the dot (the Memory tab's Add memory).
    pub async fn add(&self, dot_id: &str, text: &str) -> Result<Memory> {
        self.save(dot_id, text, MemorySource::Person, None).await
    }

    /// The person changes any memory: its text (the rules of `save`) and/or its pin.
    pub async fn update(
        &self,
        id: &str,
        text: Option<&str>,
        pinned: Option<bool>,
    ) -> Result<Memory> {
        let text = text
            .map(check_text)
            .transpose()
            .map_err(|rule| Error::Invalid(rule.into()))?;
        let keyed = text.as_deref().map(|t| (t, same_key(t)));
        let updated = self
            .store
            .update_memory(id, keyed.as_ref().map(|(t, k)| (*t, k.as_str())), pinned)
            .await?;
        let memory = updated.ok_or_else(|| Error::Conflict(DUPLICATE.into()))?;
        self.changed(&memory.dot_id);
        Ok(memory)
    }

    /// The person deletes any memory. `NotFound` for an unknown id.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let memory = self.store.delete_memory(id).await?;
        self.changed(&memory.dot_id);
        Ok(())
    }

    fn changed(&self, dot_id: &str) {
        let _ = self.bus.send(RuntimeEvent::MemoriesUpdated {
            dot_id: dot_id.to_string(),
        });
    }
}

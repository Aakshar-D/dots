//! Search (search spec 2026-11-06-search-design.md): one box that finds a word in what the
//! person and the dots said, in memory, goals, topic titles and the names of files added to a
//! chat. The store keeps a full-text index in step with those rows (migration 0037); this
//! module holds what a search asks and finds, and turns what the person typed into a query the
//! index cannot misread.

use serde::{Deserialize, Serialize};

// What a hit is, which says where it opens.
str_enum!(SearchKind {
    // What the person said in a chat.
    Chat => "chat",
    // What a dot said: a run's result or a reply it wrote along the way.
    Reply => "reply",
    // A note or question a dot posted.
    Note => "note",
    Memory => "memory",
    // A goal (Intents).
    Intent => "intent",
    // The title the person gave a topic.
    Topic => "topic",
    // The name of a file added to a chat.
    File => "file",
});

/// The most hits one search gives.
pub const MAX_LIMIT: u32 = 100;
/// The hits a search gives when it names no limit.
pub const DEFAULT_LIMIT: u32 = 30;
/// The most words of a search that count; the rest are left out.
pub const MAX_WORDS: usize = 12;
/// Where a matched word starts in a snippet.
pub const HIT_START: char = '\u{E000}';
/// Where a matched word ends in a snippet.
pub const HIT_END: char = '\u{E001}';

/// What the person searched for: `text` as typed, only hits of `kinds` (none is every kind),
/// only those of the dot `dot_id`, and at most `limit` (`DEFAULT_LIMIT` when none, never more
/// than `MAX_LIMIT`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    #[serde(default)]
    pub kinds: Vec<SearchKind>,
    #[serde(default)]
    pub dot_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

impl SearchQuery {
    /// The limit asked, kept between 1 and `MAX_LIMIT`.
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }
}

/// Where a hit opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchTarget {
    /// A chat topic on the home: its root run, and the run the hit is in (none for a title).
    Topic {
        dot_id: String,
        root_run_id: String,
        run_id: Option<String>,
    },
    /// A run that did not start in a chat (a schedule, a webhook, a workflow step), on its own
    /// page.
    Run { run_id: String },
    /// A dot's conversation on the home: a note of Dots' own, which no run wrote.
    Dot { dot_id: String },
    /// A memory, in its dot's Memory tab.
    Memory { dot_id: String, memory_id: String },
    /// A goal, on the Intents screen.
    Intent { intent_id: String },
}

/// One hit: what it is, its dot (a goal may have none), where it opens, when it was written,
/// and the words around the match with each matched word between `HIT_START` and `HIT_END`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub kind: SearchKind,
    pub dot_id: Option<String>,
    pub dot_name: Option<String>,
    pub target: SearchTarget,
    pub at: String,
    pub snippet: String,
}

/// `text` as a full-text query, or `None` when it has no word. Each word, a run of letters and
/// digits as the index splits them, is quoted, so nothing typed is read as query syntax
/// (quotes, `-`, `*`, `OR`, `NEAR`, parentheses or a column name); all words must match. The
/// last word also matches words it starts, unless the text ends after it with a space or a
/// mark, which says it is whole. At most `MAX_WORDS` words count.
pub fn fts_query(text: &str) -> Option<String> {
    let all: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let words = &all[..all.len().min(MAX_WORDS)];
    let last = words.len().checked_sub(1)?;
    // A word cut off after `MAX_WORDS` is not the one being typed.
    let whole = !text.ends_with(char::is_alphanumeric) || all.len() > MAX_WORDS;
    let parts: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == last && !whole {
                format!("\"{w}\"*")
            } else {
                format!("\"{w}\"")
            }
        })
        .collect();
    Some(parts.join(" "))
}

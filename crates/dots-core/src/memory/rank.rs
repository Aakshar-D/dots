//! Ranking memories against a text (memory spec section 7): the words a query is made of,
//! what the index is asked, the boosts for a pin, for recency and for a linked file, and the
//! count of matching words that stands in when the index cannot be used. No disk, no store.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::model::Memory;

/// The most words of a text that go into a query.
pub const MAX_TERMS: usize = 32;
/// A word this long or longer also matches the words it starts.
const PREFIX_FROM: usize = 4;
/// What a pinned memory's score is multiplied by.
pub const PIN_BOOST: f64 = 1.25;
/// What a memory written now gains over an old one, at most.
pub const RECENT_BOOST: f64 = 0.2;
/// The days after which half of `RECENT_BOOST` is left.
pub const RECENT_HALF_LIFE_DAYS: f64 = 30.0;
/// What the score of a memory linked to a file the text names is multiplied by.
pub const LINK_BOOST: f64 = 1.5;
/// The score of a memory that matches no word but is linked to a file the text names, when
/// no memory matches a word.
pub const LINK_ONLY: f64 = 1.0;
/// The same when some memory matches: this share of the best match's score.
pub const LINK_ONLY_SHARE: f64 = 0.75;

/// Words too common to tell memories apart.
const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has", "have",
    "how", "i", "if", "in", "into", "is", "it", "its", "just", "me", "my", "no", "not", "of", "on",
    "or", "our", "please", "so", "some", "than", "that", "the", "their", "them", "then", "there",
    "these", "they", "this", "to", "up", "us", "was", "we", "were", "what", "when", "which", "who",
    "why", "will", "with", "would", "you", "your",
];

/// The words of `text` that a query is made of: runs of letters and digits in lower case, two
/// characters or more, without the common ones, each once, the first `MAX_TERMS`.
pub fn query_terms(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !STOP_WORDS.contains(&w.as_str()))
        .filter(|w| seen.insert(w.clone()))
        .take(MAX_TERMS)
        .collect()
}

/// `terms` as a full-text query: any of them, each quoted so nothing in it is read as query
/// syntax, a longer one also matching the words it starts. `None` without terms.
pub fn fts_query(terms: &[String]) -> Option<String> {
    if terms.is_empty() {
        return None;
    }
    let parts: Vec<String> = terms
        .iter()
        .map(|t| {
            if t.chars().count() >= PREFIX_FROM {
                format!("\"{t}\"*")
            } else {
                format!("\"{t}\"")
            }
        })
        .collect();
    Some(parts.join(" OR "))
}

/// A path as two of them are compared: with `/`, without `./` in front or `/` behind, in
/// lower case (Windows ignores case, and a memory is no worse for matching too much).
fn norm(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p.trim_end_matches('/').to_lowercase()
}

/// Whether `memory` is linked to the project path `path`: it names that path, a folder above
/// it, or a file below it.
pub fn linked_to(memory: &Memory, path: &str) -> bool {
    let path = norm(path);
    if path.is_empty() {
        return false;
    }
    memory.files.iter().map(|f| norm(f)).any(|file| {
        file == path
            || path.starts_with(&format!("{file}/"))
            || file.starts_with(&format!("{path}/"))
    })
}

/// Whether `text` names one of the files `memory` is linked to: a path with a folder as it is
/// written, a single name as a word of its own (`docs`), or a path's last part when that has
/// a `.` in it and is five characters or more (`main.rs`).
pub fn names_file(text: &str, memory: &Memory) -> bool {
    if memory.files.is_empty() {
        return false;
    }
    let text = text.replace('\\', "/").to_lowercase();
    let words = words_of(&text);
    memory.files.iter().map(|f| norm(f)).any(|file| {
        let written = if file.contains('/') {
            text.contains(&file)
        } else {
            words.contains(&file)
        };
        let name = file.rsplit('/').next().unwrap_or_default();
        written || (name.chars().count() >= 5 && name.contains('.') && text.contains(name))
    })
}

/// `score` with the boosts of `memory`: for its pin, for how recently it was changed (as of
/// `now`), and for being linked to a file the text names.
pub fn boost(score: f64, memory: &Memory, now: DateTime<Utc>, linked: bool) -> f64 {
    let age_days = DateTime::parse_from_rfc3339(&memory.updated_at)
        .map(|t| (now - t.with_timezone(&Utc)).num_seconds().max(0) as f64 / 86_400.0)
        .unwrap_or(f64::MAX);
    let recent = RECENT_BOOST * 0.5f64.powf(age_days / RECENT_HALF_LIFE_DAYS);
    let mut out = score * (1.0 + recent);
    if memory.pinned {
        out *= PIN_BOOST;
    }
    if linked {
        out *= LINK_BOOST;
    }
    out
}

/// The words of a memory as the fallback and the near-duplicate check compare them.
fn words_of(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// How much of their words two texts share, 0 to 1: the words in both over the words in
/// either, the common words left out.
pub fn word_overlap(a: &str, b: &str) -> f64 {
    let keep = |text: &str| -> HashSet<String> {
        words_of(text)
            .into_iter()
            .filter(|w| !STOP_WORDS.contains(&w.as_str()))
            .collect()
    };
    let (a, b) = (keep(a), keep(b));
    let either = a.union(&b).count();
    if either == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / either as f64
}

/// The score of each memory that has a word of `terms`, when the index cannot be asked: how
/// many of the terms its text or its files have, a longer term also counting at a word's
/// start. A memory with none is left out.
pub fn fallback_scores(memories: &[Memory], terms: &[String]) -> Vec<(String, f64)> {
    memories
        .iter()
        .filter_map(|m| {
            let mut words = words_of(&m.text);
            for file in &m.files {
                words.extend(words_of(file));
            }
            let hits = terms
                .iter()
                .filter(|t| {
                    words.contains(*t)
                        || (t.chars().count() >= PREFIX_FROM
                            && words.iter().any(|w| w.starts_with(t.as_str())))
                })
                .count();
            (hits > 0).then(|| (m.id.clone(), hits as f64))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MemoryKind, MemoryScope, MemorySource};

    fn memory(text: &str, files: &[&str]) -> Memory {
        Memory {
            id: "M".into(),
            dot_id: String::new(),
            text: text.into(),
            pinned: false,
            source: MemorySource::Dot,
            run_id: None,
            created_at: "2026-11-01T00:00:00.000Z".into(),
            updated_at: "2026-11-01T00:00:00.000Z".into(),
            kind: MemoryKind::Semantic,
            scope: MemoryScope::Project,
            files: files.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[test]
    fn a_query_is_the_texts_own_words_once_each() {
        assert_eq!(
            query_terms("Please fix the Checkout page: the checkout TOTAL is wrong, a 5% tax!"),
            ["fix", "checkout", "page", "total", "wrong", "tax"]
        );
        assert!(query_terms("the of and a").is_empty());
        assert_eq!(query_terms(&"word1 word2 ".repeat(40)).len(), 2);
        let many: String = (0..100).map(|i| format!("w{i} ")).collect();
        assert_eq!(query_terms(&many).len(), MAX_TERMS);
        // Nothing typed is query syntax: every word is quoted.
        assert_eq!(
            fts_query(&query_terms("NEAR(\"x\") OR -tests* col:ab")).unwrap(),
            "\"near\"* OR \"tests\"* OR \"col\" OR \"ab\""
        );
        assert_eq!(fts_query(&[]), None);
    }

    #[test]
    fn a_memory_is_linked_to_its_files_the_folders_above_and_the_files_below() {
        let m = memory("x", &["src/graph", "ui/package.json"]);
        for path in [
            "src/graph",
            "src\\graph\\mod.rs",
            "./ui/package.json",
            "UI/Package.json",
            "src",
            "ui/",
        ] {
            assert!(linked_to(&m, path), "{path}");
        }
        for path in [
            "src/graphs",
            "ui/package.json.bak",
            "package.json",
            "",
            "docs",
        ] {
            assert!(!linked_to(&m, path), "{path}");
        }
    }

    #[test]
    fn a_text_names_a_file_by_its_path_or_its_name() {
        let m = memory("x", &["ui/src/api/types.ts", "docs"]);
        assert!(names_file("Change ui\\src\\api\\types.ts please", &m));
        assert!(names_file("the Types.ts file is stale", &m));
        assert!(!names_file("what types do we have", &m));
        assert!(names_file("update the docs", &m));
        assert!(!names_file("anything", &memory("x", &[])));
    }

    #[test]
    fn a_pin_a_recent_change_and_a_linked_file_raise_a_score() {
        let now = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let fresh = memory("x", &[]);
        let mut old = fresh.clone();
        old.updated_at = "2025-11-01T00:00:00.000Z".into();
        let mut pinned = old.clone();
        pinned.pinned = true;
        let (f, o, p) = (
            boost(1.0, &fresh, now, false),
            boost(1.0, &old, now, false),
            boost(1.0, &pinned, now, false),
        );
        assert!((f - 1.2).abs() < 1e-9, "{f}");
        assert!(o > 1.0 && o < 1.001, "{o}");
        assert!((p / o - PIN_BOOST).abs() < 1e-9);
        assert!((boost(1.0, &old, now, true) / o - LINK_BOOST).abs() < 1e-9);
        // A time that cannot be read is an old one, not an error.
        let mut odd = fresh.clone();
        odd.updated_at = "yesterday".into();
        assert_eq!(boost(2.0, &odd, now, false), 2.0);
    }

    #[test]
    fn the_fallback_counts_matching_words_and_overlap_compares_two_texts() {
        let list = [
            Memory {
                id: "A".into(),
                ..memory("Deploys go out on Fridays.", &[])
            },
            Memory {
                id: "B".into(),
                ..memory(
                    "The deployment script needs the VPN.",
                    &["tools/deploy.ps1"],
                )
            },
            Memory {
                id: "C".into(),
                ..memory("Unrelated.", &[])
            },
        ];
        let scores = fallback_scores(&list, &query_terms("deploy script fridays"));
        assert_eq!(scores, [("A".to_string(), 2.0), ("B".to_string(), 2.0)]);
        assert!(word_overlap("We deploy on Fridays.", "we deploy on fridays") > 0.99);
        assert!(word_overlap("We deploy on Fridays.", "Deploys happen on Mondays.") < 0.3);
        assert_eq!(word_overlap("the", "a"), 0.0);
    }
}

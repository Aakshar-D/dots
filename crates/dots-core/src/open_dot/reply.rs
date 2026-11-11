//! Reply reading: a model reply split into the words to say and at most one wish.
//!
//! The reply is plain speech. Its last non-blank line may be `DO {json}` (or that
//! line inside a code fence at the end), which carries the one thing Open Dot is
//! asked to do. The block is never spoken. Where a rule leaves a case open the
//! stricter reading wins: a doubtful block is `bad_block`, and doubtful text is
//! held back from speech.

use serde::Deserialize;

pub const MAX_SPOKEN_BYTES: usize = 600;

/// Most options a question may offer, and the longest one.
const MAX_OPTIONS: usize = 6;
const MAX_OPTION_BYTES: usize = 80;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "act", rename_all = "snake_case")]
pub enum Wish {
    /// What needs me.
    Needs,
    ReadQuestion {
        q: String,
    },
    Answer {
        q: String,
        text: String,
        #[serde(default)]
        unsure: bool,
    },
    Approve {
        a: String,
    },
    Deny {
        a: String,
    },
    Create {
        description: String,
    },
    Send {
        d: String,
        text: String,
        #[serde(default)]
        here: bool,
    },
    Run {
        d: String,
    },
    /// "d3", "settings", "inbox", "usage", "home".
    Open {
        target: String,
    },
    /// The model asks the person.
    Ask {
        question: String,
        options: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub words: String,
    pub wish: Option<Wish>,
}

/// Whole reply text to words + wish. Never fails: an unreadable `DO` line is dropped
/// and reported as `wish: None` with `bad_block = true`.
pub fn read_reply(text: &str) -> (Reply, bool) {
    let (before, json) = split_block(text);
    let words = spoken(before);
    let Some(json) = json else {
        return (Reply { words, wish: None }, false);
    };
    match serde_json::from_str::<Wish>(json) {
        Ok(wish) if wish.is_usable() => (
            Reply {
                words,
                wish: Some(wish),
            },
            false,
        ),
        _ => (Reply { words, wish: None }, true),
    }
}

impl Wish {
    /// Whether every value is usable. Blank ids, texts and options are not.
    fn is_usable(&self) -> bool {
        let filled = |s: &str| !s.trim().is_empty();
        match self {
            Wish::Needs => true,
            Wish::ReadQuestion { q } => filled(q),
            Wish::Answer { q, text, .. } => filled(q) && filled(text),
            Wish::Approve { a } | Wish::Deny { a } => filled(a),
            Wish::Create { description } => filled(description),
            Wish::Send { d, text, .. } => filled(d) && filled(text),
            Wish::Run { d } => filled(d),
            Wish::Open { target } => filled(target),
            Wish::Ask { question, options } => {
                filled(question)
                    && !options.is_empty()
                    && options.len() <= MAX_OPTIONS
                    && options
                        .iter()
                        .all(|o| filled(o) && o.len() <= MAX_OPTION_BYTES)
            }
        }
    }
}

const BLOCK_START: &str = "DO {";
const FENCE: &str = "```";

fn is_block_line(line: &str) -> bool {
    line.trim_start().starts_with(BLOCK_START)
}

/// The JSON of a block line: from its opening brace.
fn block_json(line: &str) -> &str {
    line.trim_start()[BLOCK_START.len() - 1..].trim()
}

/// An opening fence: backticks and at most a language word.
fn is_fence_open(line: &str) -> bool {
    line.trim().strip_prefix(FENCE).is_some_and(|lang| {
        lang.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    })
}

/// The reply text before the block, and the block's JSON, when the last non-blank
/// line is a block (bare, or alone in a closing code fence).
fn split_block(text: &str) -> (&str, Option<&str>) {
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if !line.trim().is_empty() {
            lines.push((at, line));
        }
        at += line.len();
    }
    let Some(&(last_at, last)) = lines.last() else {
        return (text, None);
    };
    if is_block_line(last) {
        return (&text[..last_at], Some(block_json(last)));
    }
    if last.trim() == FENCE && lines.len() >= 3 {
        let (_, inner) = lines[lines.len() - 2];
        let (open_at, open) = lines[lines.len() - 3];
        if is_block_line(inner) && is_fence_open(open) {
            return (&text[..open_at], Some(block_json(inner)));
        }
    }
    (text, None)
}

/// Byte offset just after the last sentence end that lies within `limit` bytes of
/// `s`. With `end_counts`, a mark at the very end of `s` is a sentence end; without
/// it a mark needs a following character, since it may be the dot of "3.14".
fn last_sentence_end(s: &str, limit: usize, end_counts: bool) -> Option<usize> {
    let mut found = None;
    for (i, ch) in s.char_indices() {
        let end = i + ch.len_utf8();
        if end > limit {
            break;
        }
        let next = s[end..].chars().next();
        if next.is_none() && !end_counts {
            continue;
        }
        let mark = matches!(
            ch,
            '.' | '!' | '?' | '\u{2026}' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}'
        );
        if mark && next.is_none_or(char::is_whitespace) {
            found = Some(end);
        }
    }
    found
}

/// The byte length the words are cut to: all of them when they fit, else up to the
/// last sentence end within the cap, else up to the cap on a char boundary.
fn cut_len(words: &str) -> usize {
    if words.len() <= MAX_SPOKEN_BYTES {
        return words.len();
    }
    let mut limit = MAX_SPOKEN_BYTES;
    while !words.is_char_boundary(limit) {
        limit -= 1;
    }
    last_sentence_end(words, limit, true).unwrap_or(limit)
}

fn spoken(before: &str) -> String {
    let words = before.trim();
    words[..cut_len(words)].trim_end().to_string()
}

/// Whether the text from just after an opening fence line can still turn out to be
/// the end block: one `DO {` line, then the closing fence, then nothing but blanks.
fn fence_may_be_block(rest: &str) -> bool {
    let mut lines = rest.split_inclusive('\n').filter(|l| !l.trim().is_empty());
    let Some(first) = lines.next() else {
        return true;
    };
    let c1 = first.trim();
    if !c1.starts_with(BLOCK_START) {
        return !first.ends_with('\n') && BLOCK_START.starts_with(c1);
    }
    if !first.ends_with('\n') {
        return true;
    }
    let Some(second) = lines.next() else {
        return true;
    };
    let c2 = second.trim();
    if !second.ends_with('\n') {
        return FENCE.starts_with(c2);
    }
    c2 == FENCE && lines.next().is_none()
}

/// Byte offset in `body` before which everything is certainly speech. From a line
/// that could be (the start of) an end block, nothing is safe yet.
fn safe_end(body: &str) -> usize {
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        let terminated = line.ends_with('\n');
        let t = line.trim_start();
        let rest = &body[at + line.len()..];
        let candidate = if !terminated && (BLOCK_START.starts_with(t) || FENCE.starts_with(t)) {
            true
        } else if t.starts_with(BLOCK_START) {
            !terminated || rest.trim().is_empty()
        } else if t.trim_end().starts_with(FENCE) {
            !terminated || fence_may_be_block(rest)
        } else {
            false
        };
        if candidate {
            return at;
        }
        at += line.len();
    }
    body.len()
}

/// Feeds streamed text; returns only text that is certainly speech.
///
/// What it releases, joined with `finish`, is the reply's words. Text is held back
/// from any line that may be the end block, and from the sentence being written,
/// so the cap's sentence cut never has to take back what was already said.
pub struct SpeechGate {
    text: String,
    /// Bytes of the (leading-trimmed) text already released.
    released: usize,
    /// The cap was reached: nothing more is released.
    capped: bool,
}

impl SpeechGate {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            released: 0,
            capped: false,
        }
    }

    /// Speech released by this delta ("" if held).
    pub fn push(&mut self, delta: &str) -> String {
        self.text.push_str(delta);
        if self.capped {
            return String::new();
        }
        let body = self.text.trim_start();
        let safe = &body[..safe_end(body)];
        let settled = safe.trim_end();
        let target = if settled.len() > MAX_SPOKEN_BYTES {
            self.capped = true;
            cut_len(settled)
        } else {
            last_sentence_end(safe, settled.len(), false).unwrap_or(settled.len())
        };
        if target <= self.released {
            return String::new();
        }
        let out = body[self.released..target].to_string();
        self.released = target;
        out
    }

    /// Held speech that turned out not to be a block.
    pub fn finish(self) -> String {
        let words = read_reply(&self.text).0.words;
        words.get(self.released..).unwrap_or("").to_string()
    }
}

impl Default for SpeechGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (Reply, bool) {
        read_reply(text)
    }

    fn squash(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn words_only() {
        let (r, bad) = read("  Two things need you.\nThe first is the deploy.  \n");
        assert_eq!(r.words, "Two things need you.\nThe first is the deploy.");
        assert_eq!(r.wish, None);
        assert!(!bad);
        assert_eq!(read("").0.words, "");
        assert_eq!(read("   \n\n").0.wish, None);
    }

    #[test]
    fn words_then_block() {
        let cases: Vec<(&str, Wish)> = vec![
            (r#"{"act":"needs"}"#, Wish::Needs),
            (
                r#"{"act":"read_question","q":"q1"}"#,
                Wish::ReadQuestion { q: "q1".into() },
            ),
            (
                r#"{"act":"answer","q":"q1","text":"use staging"}"#,
                Wish::Answer {
                    q: "q1".into(),
                    text: "use staging".into(),
                    unsure: false,
                },
            ),
            (
                r#"{"act":"answer","q":"q1","text":"maybe","unsure":true}"#,
                Wish::Answer {
                    q: "q1".into(),
                    text: "maybe".into(),
                    unsure: true,
                },
            ),
            (
                r#"{"act":"approve","a":"a1"}"#,
                Wish::Approve { a: "a1".into() },
            ),
            (r#"{"act":"deny","a":"a1"}"#, Wish::Deny { a: "a1".into() }),
            (
                r#"{"act":"create","description":"fix the build"}"#,
                Wish::Create {
                    description: "fix the build".into(),
                },
            ),
            (
                r#"{"act":"send","d":"d3","text":"go on"}"#,
                Wish::Send {
                    d: "d3".into(),
                    text: "go on".into(),
                    here: false,
                },
            ),
            (
                r#"{"act":"send","d":"d3","text":"go on","here":true}"#,
                Wish::Send {
                    d: "d3".into(),
                    text: "go on".into(),
                    here: true,
                },
            ),
            (r#"{"act":"run","d":"d3"}"#, Wish::Run { d: "d3".into() }),
            (
                r#"{"act":"open","target":"inbox"}"#,
                Wish::Open {
                    target: "inbox".into(),
                },
            ),
            (
                r#"{"act":"ask","question":"Which one?","options":["Staging","Production"]}"#,
                Wish::Ask {
                    question: "Which one?".into(),
                    options: vec!["Staging".into(), "Production".into()],
                },
            ),
        ];
        for (json, want) in cases {
            for text in [
                format!("Sure thing.\nDO {json}"),
                format!("Sure thing.\n\n  DO {json}  \n\n"),
            ] {
                let (r, bad) = read(&text);
                assert_eq!(r.words, "Sure thing.", "{json}");
                assert_eq!(r.wish, Some(want.clone()), "{json}");
                assert!(!bad, "{json}");
            }
        }
        // a block alone
        let (r, bad) = read(r#"DO {"act":"needs"}"#);
        assert_eq!(
            (r.words.as_str(), r.wish, bad),
            ("", Some(Wish::Needs), false)
        );
    }

    #[test]
    fn fenced_block() {
        let (r, bad) = read("Opening it.\n```\nDO {\"act\":\"open\",\"target\":\"usage\"}\n```\n");
        assert_eq!(r.words, "Opening it.");
        assert_eq!(
            r.wish,
            Some(Wish::Open {
                target: "usage".into()
            })
        );
        assert!(!bad);
        let (r, bad) = read("Opening it.\n```text\nDO {\"act\":\"needs\"}\n```");
        assert_eq!(
            (r.words.as_str(), r.wish, bad),
            ("Opening it.", Some(Wish::Needs), false)
        );
        // an unclosed fence: the last line is still a bare block, the stray fence is speech
        let (r, bad) = read("Hi.\n```\nDO {\"act\":\"needs\"}");
        assert_eq!(r.wish, Some(Wish::Needs));
        assert!(!bad);
        assert!(r.words.contains("```"));
        // a fence holding more than the block is no block
        let (r, _) = read("Hi.\n```\nsome code\nDO {\"act\":\"needs\"}\n```");
        assert_eq!(r.wish, None);
    }

    #[test]
    fn unknown_act_is_bad() {
        let (r, bad) = read("Done.\nDO {\"act\":\"delete_everything\",\"d\":\"d1\"}");
        assert_eq!(r.words, "Done.");
        assert_eq!(r.wish, None);
        assert!(bad);
    }

    #[test]
    fn malformed_json_is_bad() {
        for tail in [
            "DO {\"act\":\"needs\"",
            "DO {act: needs}",
            "DO {}",
            "DO {\"act\":\"needs\"} trailing",
            "DO {\"act\":\"run\"}",
            "DO {\"act\":\"run\",\"d\":3}",
        ] {
            let (r, bad) = read(&format!("Done.\n{tail}"));
            assert_eq!(r.words, "Done.", "{tail}");
            assert_eq!(r.wish, None, "{tail}");
            assert!(bad, "{tail}");
        }
    }

    #[test]
    fn invalid_values_are_bad() {
        let seven: Vec<String> = (0..7).map(|i| format!("\"o{i}\"")).collect();
        let six: Vec<String> = (0..6).map(|i| format!("\"o{i}\"")).collect();
        let long = "x".repeat(81);
        let fits = "x".repeat(80);
        let bad_blocks = [
            format!(
                "DO {{\"act\":\"ask\",\"question\":\"Which?\",\"options\":[{}]}}",
                seven.join(",")
            ),
            format!("DO {{\"act\":\"ask\",\"question\":\"Which?\",\"options\":[\"{long}\"]}}"),
            "DO {\"act\":\"ask\",\"question\":\"Which?\",\"options\":[]}".to_string(),
            "DO {\"act\":\"ask\",\"question\":\"Which?\",\"options\":[\"  \"]}".to_string(),
            "DO {\"act\":\"ask\",\"question\":\"\",\"options\":[\"a\"]}".to_string(),
            "DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"\"}".to_string(),
            "DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"   \"}".to_string(),
            "DO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"\"}".to_string(),
            "DO {\"act\":\"create\",\"description\":\"\"}".to_string(),
            "DO {\"act\":\"approve\",\"a\":\"\"}".to_string(),
            "DO {\"act\":\"open\",\"target\":\"\"}".to_string(),
        ];
        for b in bad_blocks {
            let (r, bad) = read(&format!("Hm.\n{b}"));
            assert_eq!((r.wish, bad), (None, true), "{b}");
            assert_eq!(r.words, "Hm.");
        }
        let ok = format!(
            "DO {{\"act\":\"ask\",\"question\":\"Which?\",\"options\":[{},\"{fits}\"]}}",
            six[..5].join(",")
        );
        let (r, bad) = read(&format!("Hm.\n{ok}"));
        assert!(!bad);
        assert!(matches!(r.wish, Some(Wish::Ask { ref options, .. }) if options.len() == 6));
    }

    #[test]
    fn extra_fields_ignored() {
        let (r, bad) = read("Approving.\nDO {\"act\":\"approve\",\"a\":\"a1\",\"confirmed\":true}");
        assert_eq!(r.wish, Some(Wish::Approve { a: "a1".into() }));
        assert!(!bad);
    }

    #[test]
    fn over_long_words_trimmed_on_sentence() {
        let sentence = "This is a sentence of some length. ";
        let text = sentence.repeat(30);
        assert!(text.len() > MAX_SPOKEN_BYTES);
        let (r, _) = read(&text);
        assert!(r.words.len() <= MAX_SPOKEN_BYTES);
        assert!(r.words.ends_with("length."));
        assert!(r.words.len() > MAX_SPOKEN_BYTES - sentence.len());
        // no sentence end: cut on a char boundary at the limit
        let (r, _) = read(&"word ".repeat(200));
        assert!(r.words.len() <= MAX_SPOKEN_BYTES && r.words.len() >= MAX_SPOKEN_BYTES - 1);
        // a dot inside a number is not a sentence end
        let text = format!("{} 3.14159 {}", "a".repeat(595), "b".repeat(50));
        let (r, _) = read(&text);
        assert!(r.words.len() <= MAX_SPOKEN_BYTES);
        // exactly at the limit is untouched
        let exact = "a".repeat(MAX_SPOKEN_BYTES);
        assert_eq!(read(&exact).0.words, exact);
        // the cap holds with a block too
        let (r, _) = read(&format!("{text}\nDO {{\"act\":\"needs\"}}"));
        assert!(r.words.len() <= MAX_SPOKEN_BYTES);
        assert_eq!(r.wish, Some(Wish::Needs));
    }

    #[test]
    fn multibyte_trim_is_on_boundary() {
        let text = "日本語の文章".repeat(100);
        let (r, _) = read(&text);
        assert!(r.words.len() <= MAX_SPOKEN_BYTES);
        assert!(r.words.len() >= MAX_SPOKEN_BYTES - 3);
        assert!(text.starts_with(&r.words));
        let text = "é".repeat(400);
        let (r, _) = read(&text);
        assert!(r.words.len() <= MAX_SPOKEN_BYTES);
        assert_eq!(r.words.chars().count(), r.words.len() / 2);
    }

    #[test]
    fn block_in_middle_is_speech_not_action() {
        let text = "First.\nDO {\"act\":\"needs\"}\nAnd then some more words.";
        let (r, bad) = read(text);
        assert_eq!(r.words, text);
        assert_eq!(r.wish, None);
        assert!(!bad);
        // only the last line counts when two blocks are given
        let (r, bad) = read("DO {\"act\":\"needs\"}\nDO {\"act\":\"open\",\"target\":\"home\"}");
        assert_eq!(r.words, "DO {\"act\":\"needs\"}");
        assert_eq!(
            r.wish,
            Some(Wish::Open {
                target: "home".into()
            })
        );
        assert!(!bad);
        // a DO that does not open a brace is just speech
        let (r, _) = read("Hello.\nDO it now");
        assert_eq!(r.wish, None);
        assert_eq!(r.words, "Hello.\nDO it now");
    }

    fn gate_all(chunks: &[&str]) -> (String, String) {
        let mut gate = SpeechGate::new();
        let mut live = String::new();
        for c in chunks {
            live.push_str(&gate.push(c));
        }
        let tail = gate.finish();
        (live, tail)
    }

    #[test]
    fn gate_never_releases_block() {
        let long: String = (1..=40)
            .map(|i| format!("Sentence number {i} is here. "))
            .collect();
        let replies = [
            "Two things need you.\nDO {\"act\":\"needs\"}".to_string(),
            "Two things need you. The deploy is first.\n\nDO {\"act\":\"open\",\"target\":\"inbox\"}\n\n".to_string(),
            "Opening it, café time.\n```\nDO {\"act\":\"open\",\"target\":\"usage\"}\n```\n".to_string(),
            "Done.\nDO {\"act\":\"nonsense\"}".to_string(),
            "Done.\nDO {broken".to_string(),
            "No block here at all.".to_string(),
            "First.\nDO {\"act\":\"needs\"}\nAnd more words after.".to_string(),
            "日本語の文章です。\nDO {\"act\":\"needs\"}".to_string(),
            format!("{long}\nDO {{\"act\":\"needs\"}}"),
            "word ".repeat(200),
            format!("{} 3.14159 {}", "a".repeat(595), "b".repeat(50)),
        ];
        for reply in &replies {
            let want = read_reply(reply).0.words;
            let has_block = read_reply(reply).0.wish.is_some() || read_reply(reply).1;
            // fed whole, at every char boundary, and a char at a time
            let mut splits: Vec<Vec<&str>> = vec![vec![reply.as_str()]];
            for i in 1..reply.len() {
                if reply.is_char_boundary(i) {
                    splits.push(vec![&reply[..i], &reply[i..]]);
                }
            }
            splits.push(
                reply
                    .char_indices()
                    .map(|(i, c)| &reply[i..i + c.len_utf8()])
                    .collect(),
            );
            for chunks in splits {
                let (live, tail) = gate_all(&chunks);
                assert!(live.len() <= MAX_SPOKEN_BYTES, "cap: {reply:?}");
                let all = format!("{live}{tail}");
                assert_eq!(squash(&all), squash(&want), "{reply:?} split {chunks:?}");
                if has_block {
                    // the block line is never in what was said
                    assert!(!all.contains("DO {\"act\""), "{reply:?}");
                    assert!(!all.contains("```"), "{reply:?}");
                }
            }
        }
    }

    #[test]
    fn gate_holds_a_block_until_the_end() {
        let (live, tail) = gate_all(&["Hello there.\n", "DO {\"act\":", "\"needs\"}"]);
        assert!(!live.contains("DO"));
        assert!(!tail.contains("DO"));
        assert_eq!(live.trim(), "Hello there.");
        // a held DO line is released once more words follow it
        let (live, tail) = gate_all(&["Hi.\nDO {\"act\":\"needs\"}\n", "More words.\n"]);
        assert!(live.contains("DO {"));
        assert_eq!(
            squash(&format!("{live}{tail}")),
            "Hi. DO {\"act\":\"needs\"} More words."
        );
    }

    #[test]
    fn gate_releases_a_line_starting_with_do_that_is_not_a_block() {
        let text = "Sure.\nDo you want me to run it?\nDOT is ready.\nDONE.\n";
        let mut gate = SpeechGate::new();
        let mut live = String::new();
        // a char at a time: the held "D", "DO" and "DOT" are let go once they fail to match
        for (i, c) in text.char_indices() {
            live.push_str(&gate.push(&text[i..i + c.len_utf8()]));
        }
        assert!(live.contains("Do you want me to run it?"), "{live:?}");
        assert!(live.contains("DOT is ready."), "{live:?}");
        let tail = gate.finish();
        assert_eq!(squash(&format!("{live}{tail}")), squash(text));
    }
}

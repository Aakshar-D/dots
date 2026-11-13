//! Text made fit to be spoken, and cut into sentences as it streams in.

/// The most a spoken reply may run to, in bytes.
pub const MAX_SPOKEN_CHARS: usize = 600;

const CODE_ON_SCREEN: &str = "(code is on screen)";
const LINK_ON_SCREEN: &str = "(link is on screen)";

/// Punctuation kept in spoken text; every other symbol (emoji, markup, arrows) is dropped.
const KEPT_PUNCTUATION: &str = ".,!?;:'\"()-_%&/\u{2019}\u{201C}\u{201D}";

/// Turns a reply written for the screen into text that reads aloud well: code and links are
/// pointed at rather than read, markdown marks are removed, and each line or list item becomes
/// a sentence.
pub fn speakable(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_fence = false;
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if !in_fence {
                lines.push(CODE_ON_SCREEN.to_string());
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let line = clean_line(raw);
        // A line of symbols alone ("---", emoji) leaves nothing to say.
        if line.chars().any(char::is_alphanumeric) {
            lines.push(line);
        }
    }
    // One line is spoken as written. Several lines (list items, headings, paragraphs) are
    // each made a sentence, so the voice pauses between them.
    if lines.len() < 2 {
        return lines.pop().unwrap_or_default();
    }
    let sentences: Vec<String> = lines
        .into_iter()
        .map(|line| {
            let is_placeholder = line == CODE_ON_SCREEN || line == LINK_ON_SCREEN;
            if is_placeholder || line.ends_with(['.', '!', '?', ':']) {
                line
            } else {
                format!("{line}.")
            }
        })
        .collect();
    sentences.join(" ")
}

/// One line of markdown to plain words; empty when nothing speakable is left.
fn clean_line(raw: &str) -> String {
    let mut line = raw.trim();
    line = line.trim_start_matches('>').trim_start();
    line = strip_heading(line);
    line = strip_list_marker(line);
    let line = strip_links(line);
    let line = line
        .split_whitespace()
        .map(replace_url)
        .collect::<Vec<_>>()
        .join(" ");
    let line = strip_emphasis(&line);
    let kept: String = line
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || KEPT_PUNCTUATION.contains(*c))
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_heading(line: &str) -> &str {
    let rest = line.trim_start_matches('#');
    if rest.len() != line.len() && rest.starts_with(char::is_whitespace) {
        rest.trim_start()
    } else {
        line
    }
}

fn strip_list_marker(line: &str) -> &str {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return rest.trim_start();
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        for marker in [". ", ") "] {
            if let Some(rest) = rest.strip_prefix(marker) {
                return rest.trim_start();
            }
        }
    }
    line
}

/// `[label](target)` and `![alt](target)` become their label.
fn strip_links(line: &str) -> String {
    let mut out = line.to_string();
    let mut from = 0;
    while let Some(found) = out[from..].find("](") {
        let close = from + found;
        let Some(open) = out[..close].rfind('[') else {
            from = close + 2;
            continue;
        };
        let Some(end) = out[close + 2..].find(')') else {
            break;
        };
        let end = close + 2 + end;
        let label = out[open + 1..close].to_string();
        let start = if out[..open].ends_with('!') {
            open - 1
        } else {
            open
        };
        out.replace_range(start..=end, &label);
        from = start + label.len();
    }
    out
}

/// A bare address becomes a pointer to the screen, keeping any punctuation that closed it.
fn replace_url(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www."))
    {
        return word.to_string();
    }
    let tail_start = word
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\''])
        .len();
    format!("{LINK_ON_SCREEN}{}", &word[tail_start..])
}

/// Removes `*`, `~~`, backticks and `_` used as emphasis (not the `_` inside `snake_case`).
fn strip_emphasis(line: &str) -> String {
    let line = line.replace("~~", "").replace(['*', '`'], "");
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            let before = i.checked_sub(1).map(|j| chars[j]);
            let after = chars.get(i + 1).copied();
            let inside_word = before.is_some_and(char::is_alphanumeric)
                && after.is_some_and(char::is_alphanumeric);
            if !inside_word {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Cuts a long reply at the last sentence end that fits [`MAX_SPOKEN_CHARS`]. A single sentence
/// longer than that is cut at a word instead and closed with a full stop.
pub fn trim_spoken(text: &str) -> String {
    let text = text.trim();
    if text.len() <= MAX_SPOKEN_CHARS {
        return text.to_string();
    }
    let mut stream = SentenceStream::new();
    let mut sentences = stream.push(text);
    sentences.extend(stream.finish());

    let mut out = String::new();
    for sentence in &sentences {
        let extra = if out.is_empty() { 0 } else { 1 };
        if out.len() + extra + sentence.len() > MAX_SPOKEN_CHARS {
            break;
        }
        if extra == 1 {
            out.push(' ');
        }
        out.push_str(sentence);
    }
    if !out.is_empty() {
        return out;
    }
    // The first sentence alone is too long: cut at a word, leaving room for the full stop.
    let mut end = MAX_SPOKEN_CHARS - 1;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    let head = head.rfind(char::is_whitespace).map_or(head, |i| &head[..i]);
    let head = head.trim_end_matches(|c: char| c.is_whitespace() || ",;:-".contains(c));
    format!("{head}.")
}

/// Cuts text arriving in pieces into sentences, each handed back as soon as it is known to be
/// whole. A sentence ends at `.`, `!` or `?` followed by whitespace, except a `.` after a single
/// letter (`a.m.`, `e.g.`). Closing quotes and brackets belong to the sentence they close.
#[derive(Default)]
pub struct SentenceStream {
    buf: String,
}

impl SentenceStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `delta` and returns the sentences it completed. A terminator at the very end of
    /// the text so far is held back until the next character shows whether it ends a sentence.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let chars: Vec<(usize, char)> = self.buf.char_indices().collect();
        let mut out = Vec::new();
        let mut start = 0;
        let mut start_at = 0;
        let mut in_fence = false;
        let mut i = 0;
        while i < chars.len() {
            let (_, c) = chars[i];
            // A fenced block is indivisible: no sentence ends inside it.
            if (i == 0 || chars[i - 1].1 == '\n') && opens_fence(&chars, i) {
                in_fence = !in_fence;
                while i < chars.len() && chars[i].1 != '\n' {
                    i += 1;
                }
                continue;
            }
            if in_fence || !matches!(c, '.' | '!' | '?') {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len()
                && matches!(chars[j].1, '"' | '\'' | ')' | '\u{201D}' | '\u{2019}')
            {
                j += 1;
            }
            let ends_here = chars.get(j).is_some_and(|&(_, next)| next.is_whitespace())
                && !(c == '.'
                    && (follows_single_letter(&chars, i) || is_list_number(&chars, i, start_at)));
            if ends_here {
                let end = chars[j].0;
                let sentence = self.buf[start..end].trim();
                if !sentence.is_empty() {
                    out.push(sentence.to_string());
                }
                start = end;
                start_at = j;
            }
            i = j.max(i + 1);
        }
        self.buf.drain(..start);
        let kept = self.buf.trim_start().len();
        self.buf.drain(..self.buf.len() - kept);
        out
    }

    /// Hands back whatever is left as a last sentence.
    pub fn finish(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.buf);
        let rest = rest.trim();
        if rest.is_empty() {
            Vec::new()
        } else {
            vec![rest.to_string()]
        }
    }
}

/// Whether the line at `chars[at]` opens or closes a fenced block (``` or ~~~).
fn opens_fence(chars: &[(usize, char)], at: usize) -> bool {
    let mut k = at;
    while k < chars.len() && matches!(chars[k].1, ' ' | '\t') {
        k += 1;
    }
    let marker = |m: char| (0..3).all(|n| chars.get(k + n).is_some_and(|&(_, c)| c == m));
    marker('`') || marker('~')
}

/// Whether the `.` at `chars[at]` ends a list number ("1." at a line or sentence start).
/// `sentence_at` is where the sentence being read began.
fn is_list_number(chars: &[(usize, char)], at: usize, sentence_at: usize) -> bool {
    let mut k = at;
    while k > 0 && chars[k - 1].1.is_ascii_digit() {
        k -= 1;
    }
    if k == at {
        return false;
    }
    k == 0 || chars[k - 1].1 == '\n' || (k - 1 == sentence_at && chars[k - 1].1.is_whitespace())
}

/// Whether the `.` at `chars[at]` closes a lone letter, as in `a.m.` or `e.g.`.
fn follows_single_letter(chars: &[(usize, char)], at: usize) -> bool {
    let Some(prev) = at.checked_sub(1).map(|j| chars[j].1) else {
        return false;
    };
    if !prev.is_alphabetic() {
        return false;
    }
    at.checked_sub(2)
        .is_none_or(|j| !chars[j].1.is_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_links_code_and_emoji_are_not_spoken() {
        assert_eq!(
            speakable("**Done.** See [the report](https://x.test/r)."),
            "Done. See the report."
        );
        assert_eq!(speakable("Run `cargo test` now"), "Run cargo test now");
        assert_eq!(
            speakable("Here:\n```rust\nfn main() {}\n```\nThat is all."),
            "Here: (code is on screen) That is all."
        );
        assert_eq!(speakable("- one\n- two"), "one. two.");
        assert_eq!(speakable("Great 🎉 news"), "Great news");
        assert_eq!(speakable("# Title\nBody"), "Title. Body.");
        assert_eq!(speakable("https://example.test/a/b"), "(link is on screen)");
        assert_eq!(speakable("   "), "");
    }

    #[test]
    fn sentences_come_out_as_soon_as_they_end() {
        let mut s = SentenceStream::new();
        assert!(s.push("Sent to Re").is_empty());
        assert_eq!(s.push("search. It will"), vec!["Sent to Research."]);
        assert_eq!(
            s.push(" start at 8.30 a.m. tomorrow! Ok"),
            vec!["It will start at 8.30 a.m. tomorrow!"]
        );
        assert_eq!(s.finish(), vec!["Ok"]);
        assert!(s.finish().is_empty());
    }

    #[test]
    fn a_long_reply_is_cut_at_a_sentence_end() {
        let long = "This is a sentence. ".repeat(60);
        let cut = trim_spoken(&long);
        assert!(cut.len() <= MAX_SPOKEN_CHARS);
        assert!(cut.ends_with('.'));
    }

    #[test]
    fn nested_markdown_is_flattened() {
        assert_eq!(
            speakable("**Read [the `docs`](https://x.test) _now_**"),
            "Read the docs now"
        );
        assert_eq!(
            speakable("1. **First** step\n2) second"),
            "First step. second."
        );
        assert_eq!(speakable("> quoted *text*"), "quoted text");
        assert_eq!(
            speakable("![chart](https://x.test/c.png) shows growth"),
            "chart shows growth"
        );
    }

    #[test]
    fn urls_inside_a_sentence_are_pointed_at() {
        assert_eq!(
            speakable("Open https://x.test/a, then go."),
            "Open (link is on screen), then go."
        );
        assert_eq!(
            speakable("See www.example.test."),
            "See (link is on screen)."
        );
    }

    #[test]
    fn letters_digits_and_snake_case_are_kept() {
        assert_eq!(
            speakable("Un café à Zürich, 3 for 100%"),
            "Un café à Zürich, 3 for 100%"
        );
        assert_eq!(speakable("call my_function now"), "call my_function now");
    }

    #[test]
    fn an_unclosed_fence_swallows_the_rest_as_code() {
        assert_eq!(speakable("Try:\n```\nrm -rf x"), "Try: (code is on screen)");
    }

    #[test]
    fn symbols_and_blank_lines_leave_nothing_to_say() {
        assert_eq!(speakable("🎉🎉\n\n---\n"), "");
    }

    #[test]
    fn a_sentence_ending_the_delta_waits_for_what_follows() {
        let mut s = SentenceStream::new();
        assert!(s.push("It is 8.").is_empty());
        assert!(s.push("30 now.").is_empty());
        assert_eq!(s.push(" Then"), vec!["It is 8.30 now."]);
        assert_eq!(s.finish(), vec!["Then"]);
    }

    #[test]
    fn a_terminator_alone_at_the_end_is_returned_by_finish() {
        let mut s = SentenceStream::new();
        assert!(s.push("All done!").is_empty());
        assert_eq!(s.finish(), vec!["All done!"]);
    }

    #[test]
    fn several_sentences_in_one_delta_come_out_together() {
        let mut s = SentenceStream::new();
        assert_eq!(
            s.push("One. Two? Three! Fo"),
            vec!["One.", "Two?", "Three!"]
        );
        assert_eq!(s.finish(), vec!["Fo"]);
    }

    #[test]
    fn abbreviations_and_closing_quotes_do_not_split_wrongly() {
        let mut s = SentenceStream::new();
        assert_eq!(
            s.push("Use e.g. this. He said \"go.\" Then"),
            vec!["Use e.g. this.", "He said \"go.\""]
        );
        assert_eq!(s.finish(), vec!["Then"]);
    }

    #[test]
    fn a_multibyte_delta_split_anywhere_is_handled() {
        let mut s = SentenceStream::new();
        assert!(s.push("Un café").is_empty());
        assert_eq!(s.push(" noir. Très bien"), vec!["Un café noir."]);
        assert_eq!(s.finish(), vec!["Très bien"]);
    }

    #[test]
    fn a_short_reply_is_left_alone() {
        assert_eq!(trim_spoken("  Short one.  "), "Short one.");
    }

    #[test]
    fn one_endless_sentence_is_cut_at_a_word() {
        let long = "word ".repeat(400);
        let cut = trim_spoken(&long);
        assert!(cut.len() <= MAX_SPOKEN_CHARS);
        assert!(cut.ends_with("word."));
    }

    #[test]
    fn a_long_reply_of_multibyte_text_is_cut_on_a_character_boundary() {
        let long = "é".repeat(700);
        let cut = trim_spoken(&long);
        assert!(cut.len() <= MAX_SPOKEN_CHARS);
        assert!(cut.ends_with('.'));
    }

    fn spoken_sentences(chunks: &[&str]) -> Vec<String> {
        let mut s = SentenceStream::new();
        let mut sentences = Vec::new();
        for chunk in chunks {
            sentences.extend(s.push(chunk));
        }
        sentences.extend(s.finish());
        sentences
    }

    #[test]
    fn a_numbered_list_is_not_split_after_its_numbers() {
        let sentences = spoken_sentences(&["1. First step.
2. Second step."]);
        assert!(!sentences.is_empty());
        for sentence in &sentences {
            assert!(
                sentence.chars().any(|c| c.is_alphabetic()),
                "only a number: {sentence:?}"
            );
        }
        let spoken = sentences
            .iter()
            .map(|s| speakable(s))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(spoken.contains("First step"));
        assert!(spoken.contains("Second step"));
        assert!(!spoken.contains("1."));
        assert!(!spoken.contains("2."));

        let streamed = spoken_sentences(&[
            "1. First",
            " step
2. Second",
            " step",
        ]);
        assert!(streamed
            .iter()
            .all(|s| s.chars().any(|c| c.is_alphabetic())));
    }

    #[test]
    fn a_fenced_block_is_never_split_or_read() {
        let text = "Here it is:
```rust
let a = 1. ;
fn b() {}
```
Done.";
        for size in [1, 7, text.len()] {
            let chunks: Vec<String> = text
                .chars()
                .collect::<Vec<_>>()
                .chunks(size)
                .map(|c| c.iter().collect())
                .collect();
            let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
            let spoken = spoken_sentences(&chunks)
                .iter()
                .map(|s| speakable(s))
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(
                spoken, "Here it is: (code is on screen) Done.",
                "chunk size {size}"
            );
            assert!(!spoken.contains("fn") && !spoken.contains("let"));
        }
    }

    #[test]
    fn a_fence_left_open_is_not_read_when_the_stream_finishes() {
        let spoken = spoken_sentences(&[
            "Try:
```
let a = 1. ",
            "fn b() {}",
        ])
        .iter()
        .map(|s| speakable(s))
        .collect::<Vec<_>>()
        .join(" ");
        assert_eq!(spoken, "Try: (code is on screen)");
    }

    #[test]
    fn edge_inputs_pin_the_output() {
        assert_eq!(
            speakable(
                "```
x
```"
            ),
            "(code is on screen)"
        );
        assert_eq!(speakable(""), "");
        assert_eq!(speakable("**"), "");
    }
}

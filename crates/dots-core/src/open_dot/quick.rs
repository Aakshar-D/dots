//! Quick matcher: spoken stop, cancel, repeat, yes, no and picks answered without a model.
//!
//! The whole utterance must match. When in doubt the answer is `None`, and the
//! model gets to read it instead.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quick {
    Stop,
    Cancel,
    Repeat,
    Yes,
    No,
    /// 0-based index into the options currently offered.
    Pick(usize),
}

const STOP: &[&str] = &[
    "stop",
    "stop it",
    "stop talking",
    "be quiet",
    "quiet",
    "shut up",
    "enough",
];
const CANCEL: &[&str] = &[
    "cancel",
    "cancel that",
    "never mind",
    "nevermind",
    "forget it",
    "forget that",
    "scrap that",
];
const REPEAT: &[&str] = &[
    "say that again",
    "repeat",
    "repeat that",
    "again",
    "what",
    "what did you say",
    "pardon",
    "come again",
    "sorry",
];
const YES: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "sure",
    "do it",
    "go ahead",
    "go for it",
    "approve",
    "approve it",
    "confirm",
    "correct",
    "thats right",
    "right",
    "please do",
    "yes please",
    "yes do it",
    "absolutely",
    "okay",
    "ok",
];
/// What a confirmation asks: the kind of step a firm yes passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmKind {
    /// "<Dot> wants to use <tool> ... Approve it?"
    Approve,
    /// "Deny <Dot>'s <tool> request?"
    Deny,
    /// "Run <Dot> now?"
    Run,
    /// "<Name>. <When>. ... Create it?"
    Create,
    /// "Send to <Dot>: ...?"
    Send,
    /// An answer to a dot's question, read back.
    Answer,
}

impl ConfirmKind {
    const ALL: [ConfirmKind; 6] = [
        ConfirmKind::Approve,
        ConfirmKind::Deny,
        ConfirmKind::Run,
        ConfirmKind::Create,
        ConfirmKind::Send,
        ConfirmKind::Answer,
    ];

    /// The words that pass this kind of step, beyond `FIRM_YES`.
    fn own_words(self) -> &'static [&'static str] {
        match self {
            ConfirmKind::Approve => &["approve", "approve it"],
            ConfirmKind::Deny => &["deny", "deny it", "deny that"],
            ConfirmKind::Run => &["run it"],
            ConfirmKind::Create => &["create it", "make it"],
            ConfirmKind::Send => &["send it", "send"],
            ConfirmKind::Answer => &["send it", "send that"],
        }
    }
}

/// The yes that passes any confirmation. "Okay", "sure", "right" and "correct" are not in it:
/// they are what speech-to-text makes of noise, and what a person says while still thinking.
/// Each kind of step adds its own verb (`ConfirmKind::own_words`): "approve" passes only an
/// approval, "deny" only a deny.
const FIRM_YES: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "yes please",
    "yes do it",
    "yes yes",
    "yes go ahead",
    "do it",
    "go ahead",
    "go for it",
    "please do",
    "confirm",
    "absolutely",
];
const NO: &[&str] = &[
    "no",
    "nope",
    "nah",
    "dont",
    "do not",
    "no thanks",
    "no thank you",
    "deny",
    "deny it",
    "deny that",
    "not now",
    "leave it",
];

const CARDINALS: [&str; 9] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];
const ORDINALS: [&str; 9] = [
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
];
/// Words that alone never pick an option by name.
const FILLER_WORDS: &[&str] = &["the", "a", "an", "of", "to", "and", "it", "that", "this"];

/// Lowercase, apostrophes removed, other punctuation as a space, whitespace collapsed.
pub(crate) fn words_of(text: &str) -> Vec<String> {
    let mut cleaned = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\'' || c == '\u{2019}' {
            continue;
        }
        if c.is_alphanumeric() {
            cleaned.extend(c.to_lowercase());
        } else {
            cleaned.push(' ');
        }
    }
    cleaned.split_whitespace().map(str::to_owned).collect()
}

/// Drop trailing, then leading, filler; never the only word.
fn trimmed(mut w: Vec<String>) -> Vec<String> {
    loop {
        let n = w.len();
        if n >= 3 && w[n - 2] == "thank" && w[n - 1] == "you" {
            w.truncate(n - 2);
        } else if n >= 2 && matches!(w[n - 1].as_str(), "please" | "dot" | "thanks") {
            w.pop();
        } else {
            break;
        }
    }
    loop {
        let n = w.len();
        if n >= 3 && w[0] == "hey" && w[1] == "dot" {
            w.drain(..2);
        } else if n >= 2 && matches!(w[0].as_str(), "dot" | "okay" | "ok" | "um" | "uh") {
            w.remove(0);
        } else {
            break;
        }
    }
    w
}

/// The 1-based number an utterance names, or `Some(usize::MAX)` for "last".
fn position(joined: &str) -> Option<usize> {
    if matches!(joined, "last" | "the last" | "the last one") {
        return Some(usize::MAX);
    }
    for i in 0..9 {
        let (card, ord, digit) = (CARDINALS[i], ORDINALS[i], (i + 1).to_string());
        let forms = [
            card.to_owned(),
            ord.to_owned(),
            digit.clone(),
            format!("the {ord}"),
            format!("the {ord} one"),
            format!("{ord} one"),
            format!("number {card}"),
            format!("number {digit}"),
            format!("option {card}"),
            format!("option {digit}"),
        ];
        if forms.iter().any(|f| f == joined) {
            return Some(i + 1);
        }
    }
    None
}

/// Whether a heard word can pick an option by part of its name: 3 or more letters, and not a
/// filler word or a word of the stop, cancel, repeat, yes or no phrases.
fn names_something(word: &str) -> bool {
    word.chars().count() >= 3
        && !FILLER_WORDS.contains(&word)
        && ![STOP, CANCEL, REPEAT, YES, NO]
            .iter()
            .flat_map(|table| table.iter())
            .any(|phrase| phrase.split(' ').any(|p| p == word))
}

fn is_run(option: &[String], heard: &[String]) -> bool {
    !heard.is_empty()
        && heard.len() <= option.len()
        && option.windows(heard.len()).any(|win| win == heard)
}

/// The whole utterance, trimmed as `quick` trims it, joined by single spaces.
fn joined(heard: &str) -> String {
    trimmed(words_of(heard)).join(" ")
}

/// Whether the whole utterance is a firm yes to a step of `kind` (`FIRM_YES` or the kind's own
/// verb, with the same filler trimmed as `quick`). Only this, or a click on Yes, passes a
/// confirmation; a spoken position word never does.
pub fn firm_yes(heard: &str, kind: ConfirmKind) -> bool {
    let j = joined(heard);
    !j.is_empty()
        && ((FIRM_YES.contains(&j.as_str()) && !soft_for(kind, &j))
            || kind.own_words().contains(&j.as_str()))
}

/// Words of `FIRM_YES` that are no firm yes to "Deny ...?": "go ahead" may mean "let it go
/// ahead".
const NOT_FIRM_FOR_DENY: &[&str] = &["go ahead", "go for it", "yes go ahead"];

fn soft_for(kind: ConfirmKind, joined: &str) -> bool {
    kind == ConfirmKind::Deny && NOT_FIRM_FOR_DENY.contains(&joined)
}

/// Whether the whole utterance is a yes that is firm elsewhere but not for `kind` ("go ahead"
/// said to "Deny ...?"): the person is asked once more for a clear yes, as after "okay".
pub fn unclear_yes(heard: &str, kind: ConfirmKind) -> bool {
    soft_for(kind, &joined(heard))
}

/// Whether the whole utterance is another kind's own verb and no answer to `kind`: "approve"
/// said to "Deny ...?", "send it" said to "Run ...?". Such words are neither yes nor no there:
/// the step is dropped and they go on as a new turn. ("deny" said to "Approve it?" is a `No`
/// before this is asked.)
pub fn firm_for_another(heard: &str, kind: ConfirmKind) -> bool {
    let j = joined(heard);
    !j.is_empty()
        && !firm_yes(heard, kind)
        && ConfirmKind::ALL
            .iter()
            .any(|k| k.own_words().contains(&j.as_str()))
}

/// Words too common to name a dot: articles, pronouns, common verbs and the words of asking a
/// dot something. With `FILLER_WORDS`, the quick tables and the position words, they never
/// point at a dot by its name.
const COMMON_WORDS: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "some", "any", "all", "every", "each",
    "no", "not", "and", "or", "but", "if", "then", "so", "of", "to", "in", "on", "at", "for",
    "from", "with", "by", "about", "into", "up", "out", "over", "off", "as", "i", "me", "my",
    "mine", "you", "your", "yours", "he", "him", "his", "she", "her", "hers", "it", "its", "we",
    "us", "our", "ours", "they", "them", "their", "theirs", "one", "ones", "be", "is", "am", "are",
    "was", "were", "been", "being", "do", "does", "did", "done", "have", "has", "had", "get",
    "got", "go", "goes", "make", "makes", "made", "let", "lets", "can", "could", "will", "would",
    "shall", "should", "may", "might", "must", "want", "need", "say", "says", "said", "tell",
    "tells", "told", "ask", "asks", "asked", "send", "sends", "sent", "give", "take", "put", "use",
    "run", "now", "just", "also", "too", "very", "really", "here", "there", "what", "which", "who",
    "how", "when", "where", "why", "dot", "dots", "please", "message", "answer", "reply", "thanks",
    "hey", "hi", "hello", "um", "uh",
];

/// Whether `word` (as `words_of` gives it) is too common to name a dot.
pub(crate) fn common_word(word: &str) -> bool {
    COMMON_WORDS.contains(&word)
        || FILLER_WORDS.contains(&word)
        || CARDINALS.contains(&word)
        || ORDINALS.contains(&word)
        || [STOP, CANCEL, REPEAT, YES, FIRM_YES, NO]
            .iter()
            .flat_map(|table| table.iter())
            .chain(ConfirmKind::ALL.iter().flat_map(|k| k.own_words().iter()))
            .any(|phrase| phrase.split(' ').any(|p| p == word))
}

/// `options`: the options currently offered (empty when none).
pub fn quick(heard: &str, options: &[String]) -> Option<Quick> {
    let w = trimmed(words_of(heard));
    if w.is_empty() {
        return None;
    }
    let joined = w.join(" ");
    for (table, answer) in [
        (STOP, Quick::Stop),
        (CANCEL, Quick::Cancel),
        (REPEAT, Quick::Repeat),
        (YES, Quick::Yes),
        (NO, Quick::No),
    ] {
        if table.contains(&joined.as_str()) {
            return Some(answer);
        }
    }
    if options.is_empty() {
        return None;
    }
    // An option whose whole text is what was said wins, even over a position word; two the
    // same is no pick.
    let mut exact = options
        .iter()
        .enumerate()
        .filter(|(_, o)| words_of(o) == w)
        .map(|(i, _)| i);
    match (exact.next(), exact.next()) {
        (Some(i), None) => return Some(Quick::Pick(i)),
        (Some(_), Some(_)) => return None,
        _ => {}
    }
    if let Some(n) = position(&joined) {
        return if n == usize::MAX {
            Some(Quick::Pick(options.len() - 1))
        } else if n <= options.len() {
            Some(Quick::Pick(n - 1))
        } else {
            None
        };
    }
    if !w.iter().any(|x| names_something(x)) {
        return None;
    }
    // Else every option the words are a whole-word run of; one match or nothing.
    let mut found = options
        .iter()
        .enumerate()
        .filter(|(_, o)| is_run(&words_of(o), &w))
        .map(|(i, _)| i);
    match (found.next(), found.next()) {
        (Some(i), None) => Some(Quick::Pick(i)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn quick_matches() {
        let table: [(&[&str], Quick); 5] = [
            (STOP, Quick::Stop),
            (CANCEL, Quick::Cancel),
            (REPEAT, Quick::Repeat),
            (YES, Quick::Yes),
            (NO, Quick::No),
        ];
        let mut phrases: Vec<(String, Quick)> = Vec::new();
        for (list, answer) in table {
            for p in list {
                phrases.push((p.to_string(), answer.clone()));
            }
        }
        // the spelled forms with an apostrophe, and stray punctuation and case
        phrases.push(("that's right".into(), Quick::Yes));
        phrases.push(("don't".into(), Quick::No));
        phrases.push(("Don't!".into(), Quick::No));
        phrases.push(("  Yes,   PLEASE. ".into(), Quick::Yes));
        for (p, want) in &phrases {
            assert_eq!(quick(p, &[]), Some(want.clone()), "bare: {p}");
            assert_eq!(
                quick(&format!("Hey Dot, {p}"), &[]),
                Some(want.clone()),
                "hey dot: {p}"
            );
            assert_eq!(
                quick(&format!("{p}, dot"), &[]),
                Some(want.clone()),
                "dot: {p}"
            );
            assert_eq!(
                quick(&format!("{p} thanks"), &[]),
                Some(want.clone()),
                "thanks: {p}"
            );
            assert_eq!(
                quick(&format!("Okay, {p}"), &[]),
                Some(want.clone()),
                "okay: {p}"
            );
            assert_eq!(
                quick(&format!("{p}, please."), &[]),
                Some(want.clone()),
                "please: {p}"
            );
            assert_eq!(
                quick(&format!("{p} thank you"), &[]),
                Some(want.clone()),
                "thank you: {p}"
            );
        }
        assert_eq!(phrases.len(), 7 + 7 + 9 + 20 + 12 + 4);
    }

    const EVERY_KIND: &[&str] = &[
        "yes",
        "yeah",
        "yep",
        "yup",
        "yes please",
        "yes do it",
        "yes yes",
        "yes go ahead",
        "do it",
        "go ahead",
        "go for it",
        "please do",
        "confirm",
        "absolutely",
    ];

    /// Per kind: the words that pass it beyond `EVERY_KIND`.
    fn own(kind: ConfirmKind) -> &'static [&'static str] {
        match kind {
            ConfirmKind::Approve => &["approve", "approve it"],
            ConfirmKind::Deny => &["deny", "deny it", "deny that"],
            ConfirmKind::Run => &["run it"],
            ConfirmKind::Create => &["create it", "make it"],
            ConfirmKind::Send => &["send it", "send"],
            ConfirmKind::Answer => &["send it", "send that"],
        }
    }

    #[test]
    fn firm_yes_by_kind() {
        let others: Vec<&str> = ConfirmKind::ALL
            .iter()
            .flat_map(|k| own(*k))
            .copied()
            .collect();
        // "go ahead" to "Deny ...?" may mean "let it go ahead": no firm yes there, a weak one.
        let not_for_deny = ["go ahead", "go for it", "yes go ahead"];
        for p in not_for_deny {
            assert!(!firm_yes(p, ConfirmKind::Deny), "{p}");
            assert!(!firm_for_another(p, ConfirmKind::Deny), "{p}");
            assert!(unclear_yes(p, ConfirmKind::Deny), "{p}");
            assert!(unclear_yes(
                &format!("Okay, {p}, please."),
                ConfirmKind::Deny
            ));
            assert!(!unclear_yes(p, ConfirmKind::Approve), "{p}");
        }
        assert!(!unclear_yes("yes", ConfirmKind::Deny));
        assert!(!unclear_yes("deny it", ConfirmKind::Deny));
        for kind in ConfirmKind::ALL {
            for p in EVERY_KIND.iter().chain(own(kind)) {
                if kind == ConfirmKind::Deny && not_for_deny.contains(p) {
                    continue;
                }
                assert!(firm_yes(p, kind), "{kind:?}: {p}");
                assert!(
                    firm_yes(&format!("Hey Dot, {p}, please."), kind),
                    "{kind:?}: {p}"
                );
                assert!(
                    firm_yes(&format!("okay, {p} thanks"), kind),
                    "{kind:?}: {p}"
                );
                assert!(!firm_for_another(p, kind), "{kind:?}: {p}");
            }
            // another kind's verb is no yes here, and is told apart from other words
            for p in others.iter().filter(|p| !own(kind).contains(p)) {
                assert!(!firm_yes(p, kind), "{kind:?}: {p}");
                assert!(firm_for_another(p, kind), "{kind:?}: {p}");
            }
            assert!(!firm_for_another("what about the backup", kind));
            assert!(!firm_for_another("no", kind));
        }
        // the per-kind table, spelled out
        assert!(firm_yes("approve it", ConfirmKind::Approve));
        assert!(!firm_yes("approve", ConfirmKind::Deny));
        assert!(firm_yes("deny that", ConfirmKind::Deny));
        assert!(!firm_yes("deny", ConfirmKind::Approve));
        assert_eq!(quick("deny", &[]), Some(Quick::No));
        assert!(firm_yes("run it", ConfirmKind::Run));
        assert!(!firm_yes("run it", ConfirmKind::Send));
        assert!(firm_yes("make it", ConfirmKind::Create));
        assert!(firm_yes("send", ConfirmKind::Send));
        assert!(!firm_yes("send", ConfirmKind::Answer));
        assert!(firm_yes("send that", ConfirmKind::Answer));
        assert!(firm_yes("send it", ConfirmKind::Answer));
    }

    #[test]
    fn firm_yes_is_strict() {
        assert!(firm_yes("Yes!", ConfirmKind::Run));
        assert!(firm_yes("  YES, please ", ConfirmKind::Run));
        for weak in [
            "okay",
            "ok",
            "OK.",
            "sure",
            "right",
            "correct",
            "that's right",
            "thats right",
            "one",
            "first",
            "the first one",
            "yes but wait",
            "",
            "no",
            "deny it please no",
        ] {
            for kind in ConfirmKind::ALL {
                assert!(!firm_yes(weak, kind), "{kind:?}: {weak}");
            }
            // the weak ones are still a yes where no confirmation waits
        }
        assert_eq!(quick("okay", &[]), Some(Quick::Yes));
        assert_eq!(quick("sure", &[]), Some(Quick::Yes));
    }

    #[test]
    fn okay_counts_only_alone() {
        assert_eq!(quick("okay", &[]), Some(Quick::Yes));
        assert_eq!(quick("OK.", &[]), Some(Quick::Yes));
        assert_eq!(quick("ok please", &[]), Some(Quick::Yes));
        assert_eq!(quick("okay dot", &[]), Some(Quick::Yes));
        assert_eq!(quick("please", &[]), None);
        assert_eq!(quick("okay but wait", &[]), None);
        assert_eq!(quick("um", &[]), None);
        assert_eq!(quick("hey dot", &[]), None);
        assert_eq!(quick("", &[]), None);
    }

    #[test]
    fn whole_utterance_only() {
        assert_eq!(quick("yes but use the other server", &[]), None);
        assert_eq!(quick("stop the backup dot", &[]), None);
        assert_eq!(quick("no idea what you mean", &[]), None);
        let o = opts(&["A", "B"]);
        assert_eq!(quick("the first thing I want is a report", &o), None);
        assert_eq!(quick("stop the backup dot", &o), None);
    }

    #[test]
    fn pick_by_number_position_and_name() {
        let o = opts(&["Staging server", "Production", "Test box"]);
        for form in [
            "one",
            "first",
            "the first one",
            "number one",
            "1",
            "option one",
            "Hey Dot, the first one please",
        ] {
            assert_eq!(quick(form, &o), Some(Quick::Pick(0)), "{form}");
        }
        assert_eq!(quick("second", &o), Some(Quick::Pick(1)));
        assert_eq!(quick("number 3", &o), Some(Quick::Pick(2)));
        assert_eq!(quick("option three", &o), Some(Quick::Pick(2)));
        assert_eq!(quick("the last one", &o), Some(Quick::Pick(2)));
        assert_eq!(quick("last", &o), Some(Quick::Pick(2)));
        assert_eq!(quick("staging", &o), Some(Quick::Pick(0)));
        assert_eq!(quick("Staging server.", &o), Some(Quick::Pick(0)));
        assert_eq!(quick("production", &o), Some(Quick::Pick(1)));
        assert_eq!(quick("test box please", &o), Some(Quick::Pick(2)));
        // no options offered: nothing to pick
        assert_eq!(quick("the first one", &[]), None);
        assert_eq!(quick("last", &[]), None);
        // yes and no stay yes and no
        let yn = opts(&["Yes", "No"]);
        assert_eq!(quick("yes", &yn), Some(Quick::Yes));
        assert_eq!(quick("no", &yn), Some(Quick::No));
        assert_eq!(quick("first", &yn), Some(Quick::Pick(0)));
        // a bare filler word never picks by name
        assert_eq!(quick("the", &opts(&["The server"])), None);
    }

    #[test]
    fn pick_out_of_range_is_none() {
        let o = opts(&["A thing", "B thing"]);
        assert_eq!(quick("third", &o), None);
        assert_eq!(quick("number nine", &o), None);
        assert_eq!(quick("5", &o), None);
        assert_eq!(quick("two", &o), Some(Quick::Pick(1)));
    }

    #[test]
    fn ambiguous_name_is_none() {
        let o = opts(&["Staging server", "Staging database"]);
        assert_eq!(quick("staging", &o), None);
        assert_eq!(quick("staging database", &o), Some(Quick::Pick(1)));
        assert_eq!(quick("database", &o), Some(Quick::Pick(1)));
        // an option that is exactly what was said wins over runs of others
        let o = opts(&["Staging", "Staging server"]);
        assert_eq!(quick("staging", &o), Some(Quick::Pick(0)));
        assert_eq!(quick("staging server", &o), Some(Quick::Pick(1)));
        // two options with the same text are no pick
        let o = opts(&["Staging", "staging!", "Staging server"]);
        assert_eq!(quick("staging", &o), None);
        assert_eq!(quick("nothing like it", &o), None);
    }

    #[test]
    fn exact_name_wins_over_position_words() {
        let o = opts(&["Second", "First"]);
        assert_eq!(quick("first", &o), Some(Quick::Pick(1)));
        assert_eq!(quick("second", &o), Some(Quick::Pick(0)));
        // a position word that names no option still counts as a position
        assert_eq!(quick("the first one", &o), Some(Quick::Pick(0)));
        let o = opts(&["Last resort", "One"]);
        assert_eq!(quick("one", &o), Some(Quick::Pick(1)));
        // "last" is the last position: "Last resort" is only a run of it
        assert_eq!(quick("last", &o), Some(Quick::Pick(1)));
        let o = opts(&["Last", "Other"]);
        assert_eq!(quick("last", &o), Some(Quick::Pick(0)));
    }

    #[test]
    fn short_or_phrase_words_pick_nothing() {
        let o = opts(&["Do not deploy", "Ship it"]);
        assert_eq!(quick("do", &o), None);
        assert_eq!(quick("not", &o), None);
        assert_eq!(quick("it", &o), None);
        assert_eq!(quick("deploy", &o), Some(Quick::Pick(0)));
        assert_eq!(quick("not deploy", &o), Some(Quick::Pick(0)));
        assert_eq!(quick("ship", &o), Some(Quick::Pick(1)));
        // the whole name still picks, however short its words
        assert_eq!(quick("do not deploy", &o), Some(Quick::Pick(0)));
        // yes, no, stop, cancel and repeat come before any name
        let o = opts(&["No deploy", "Deploy"]);
        assert_eq!(quick("no", &o), Some(Quick::No));
        let o = opts(&["Stop", "Go"]);
        assert_eq!(quick("stop", &o), Some(Quick::Stop));
        let o = opts(&["Go ahead now", "Wait"]);
        assert_eq!(quick("go ahead", &o), Some(Quick::Yes));
        assert_eq!(quick("go", &o), None);
    }
}

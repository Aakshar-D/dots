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
fn words_of(text: &str) -> Vec<String> {
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

fn is_run(option: &[String], heard: &[String]) -> bool {
    !heard.is_empty()
        && heard.len() <= option.len()
        && option.windows(heard.len()).any(|win| win == heard)
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
    if let Some(n) = position(&joined) {
        return if n == usize::MAX {
            Some(Quick::Pick(options.len() - 1))
        } else if n <= options.len() {
            Some(Quick::Pick(n - 1))
        } else {
            None
        };
    }
    if w.iter().all(|x| FILLER_WORDS.contains(&x.as_str())) {
        return None;
    }
    // An option whose whole text is what was said wins; two the same is no pick.
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
}

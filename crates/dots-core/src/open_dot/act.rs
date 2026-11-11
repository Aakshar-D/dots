//! Acting: a model's wish, named by handle, made into a checked step on real records; which
//! steps wait for the person's spoken yes; and a confirmed deed carried out through the
//! runtime, with the fixed sentence to say.
//!
//! A handle resolves only through the picture: one the picture does not hold is never guessed
//! from position or name. Approving, denying and running always ask first, and the question is
//! built from the picture's record, never from the model's words. Nothing here logs what was
//! said, answered or targeted.

use serde::Serialize;

use super::picture::{clean, Picture, View, CHOICE_BYTES, NAME_BYTES, QUESTION_BYTES};
use super::reply::Wish;
use crate::model::{QUESTION_ANSWERED, QUESTION_DISMISSED};
use crate::{Error, Result, Runtime};

/// A checked action on real ids.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Deed {
    Approve {
        approval_id: String,
        dot_name: String,
        tool: String,
        target: String,
    },
    Deny {
        approval_id: String,
        dot_name: String,
        tool: String,
        target: String,
    },
    Run {
        dot_id: String,
        dot_name: String,
    },
    Answer {
        message_id: String,
        dot_name: String,
        text: String,
    },
    Send {
        dot_id: String,
        dot_name: String,
        text: String,
        topic_root: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenTarget {
    Dot { dot_id: String },
    Approval { approval_id: String },
    Inbox,
    Settings,
    Usage,
    Home,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Nothing to do: fixed words.
    Say(String),
    /// Do, then say.
    Do(Deed),
    /// Ask the person first; the question is built from the real record.
    Confirm {
        deed: Deed,
        question: String,
    },
    Ask {
        question: String,
        options: Vec<String>,
        then: AskFor,
    },
    Open(OpenTarget, String),
    Create {
        description: String,
    },
}

/// What an answer to an `Ask` is for.
#[derive(Debug, Clone, PartialEq)]
pub enum AskFor {
    /// Which dot `wish` meant: the options are the dots of `handles`, in order.
    Dot { wish: Wish, handles: Vec<String> },
    /// The model asked; its next turn reads the answer.
    Model,
    /// Whether to send this answer as heard.
    AnswerCheck { deed: Deed },
}

/// Said for an approval or question handle the picture does not hold.
const GONE: &str = "I can't find that one. It may already be handled.";
/// Said when the runtime finds the record decided or gone.
const HANDLED: &str = "That one is already handled.";
const WHICH_DOT: &str = "Which dot did you mean?";
const NO_DOT: &str = "I can't find that dot.";
/// Said when the runtime no longer finds a dot that was in the picture.
pub const DOT_GONE: &str = "I can't find that dot anymore.";
const CANT_OPEN: &str = "I can't open that.";
/// Longest spoken approval target, question in the needs report, and error message, in bytes.
const SPOKEN_TARGET_BYTES: usize = 120;
const REPORT_QUESTION_BYTES: usize = 160;
const ERROR_BYTES: usize = 160;
/// A role refusal is said as it is, on one line; this only bounds it.
const REFUSAL_BYTES: usize = 1000;
/// Longest answer read back before it is sent.
const READ_BACK_BYTES: usize = 400;
/// Most dots offered when a dot is not found, and most options a model's question keeps.
const MAX_DOT_OPTIONS: usize = 4;
const MAX_OPTIONS: usize = 6;
/// Most items the needs report says.
const MAX_REPORTED: usize = 3;

/// The step for `wish`, on the records of `picture`. Approving, denying and running always
/// confirm first, with a question built from the record.
pub fn plan(wish: &Wish, picture: &Picture, view: &View) -> Step {
    match wish {
        Wish::Needs => Step::Say(needs_report(picture)),
        Wish::ReadQuestion { q } => match picture.question(q) {
            None => Step::Say(GONE.into()),
            Some(line) => {
                let mut said = sentence(&format!(
                    "{} asks: {}",
                    name(&line.dot_name),
                    clean(&line.text, QUESTION_BYTES)
                ));
                let choices: Vec<String> = line
                    .choices
                    .iter()
                    .map(|c| clean(c, CHOICE_BYTES))
                    .filter(|c| !c.is_empty())
                    .collect();
                if !choices.is_empty() {
                    said.push_str(&format!(" The choices are: {}.", list(&choices)));
                }
                Step::Say(said)
            }
        },
        Wish::Answer { q, text, unsure } => {
            let Some(line) = picture.question(q) else {
                return Step::Say(GONE.into());
            };
            let text = text.trim();
            if text.is_empty() {
                return Step::Say("I didn't catch the answer.".into());
            }
            let deed = Deed::Answer {
                message_id: line.id.clone(),
                dot_name: line.dot_name.clone(),
                text: text.to_string(),
            };
            if *unsure {
                let heard = clean(text, READ_BACK_BYTES);
                let heard = heard.trim_end_matches(['.', '!', '?']);
                Step::Ask {
                    question: format!("I heard: {heard}. Send that?"),
                    options: vec!["Yes".into(), "No".into()],
                    then: AskFor::AnswerCheck { deed },
                }
            } else {
                Step::Do(deed)
            }
        }
        Wish::Approve { a } => match picture.approval(a) {
            None => Step::Say(GONE.into()),
            Some(line) => {
                let (tool, on) = tool_on(&line.tool, &line.target);
                Step::Confirm {
                    question: format!(
                        "{} wants to use {tool}{on}. Approve it?",
                        name(&line.dot_name)
                    ),
                    deed: Deed::Approve {
                        approval_id: line.id.clone(),
                        dot_name: line.dot_name.clone(),
                        tool: line.tool.clone(),
                        target: line.target.clone(),
                    },
                }
            }
        },
        Wish::Deny { a } => match picture.approval(a) {
            None => Step::Say(GONE.into()),
            Some(line) => {
                let (tool, on) = tool_on(&line.tool, &line.target);
                Step::Confirm {
                    question: format!("Deny {}'s request to use {tool}{on}?", name(&line.dot_name)),
                    deed: Deed::Deny {
                        approval_id: line.id.clone(),
                        dot_name: line.dot_name.clone(),
                        tool: line.tool.clone(),
                        target: line.target.clone(),
                    },
                }
            }
        },
        Wish::Run { d } => match picture.dot(d) {
            None => which_dot(wish, d, picture),
            Some(line) => Step::Confirm {
                question: format!("Run {} now?", name(&line.name)),
                deed: Deed::Run {
                    dot_id: line.id.clone(),
                    dot_name: line.name.clone(),
                },
            },
        },
        Wish::Send { d, text, here } => {
            let Some(line) = picture.dot(d) else {
                return which_dot(wish, d, picture);
            };
            let text = text.trim();
            if text.is_empty() {
                return Step::Say("I didn't catch the message.".into());
            }
            // "Here" is the topic open on screen, and only for the dot on screen.
            let topic_root = if *here && view.dot_id.as_deref() == Some(line.id.as_str()) {
                view.topic_root.clone()
            } else {
                None
            };
            Step::Do(Deed::Send {
                dot_id: line.id.clone(),
                dot_name: line.name.clone(),
                text: text.to_string(),
                topic_root,
            })
        }
        Wish::Open { target } => open(target, picture),
        Wish::Create { description } => Step::Create {
            description: description.clone(),
        },
        Wish::Ask { question, options } => Step::Ask {
            question: question.clone(),
            options: options.iter().take(MAX_OPTIONS).cloned().collect(),
            then: AskFor::Model,
        },
    }
}

/// "Which dot did you mean?" with the dots whose names are close to what the model wrote,
/// else the first four; never a guess.
fn which_dot(wish: &Wish, written: &str, picture: &Picture) -> Step {
    let mut like = picture.dots_like(written);
    if like.is_empty() {
        like = picture.dots.iter().take(MAX_DOT_OPTIONS).collect();
    }
    if like.is_empty() {
        return Step::Say(NO_DOT.into());
    }
    Step::Ask {
        question: WHICH_DOT.into(),
        options: like.iter().map(|d| name(&d.name)).collect(),
        then: AskFor::Dot {
            wish: wish.clone(),
            handles: like.iter().map(|d| d.handle.clone()).collect(),
        },
    }
}

fn open(target: &str, picture: &Picture) -> Step {
    let screen = |to: OpenTarget, said: &str| Step::Open(to, said.into());
    match target.trim().to_lowercase().as_str() {
        "settings" => return screen(OpenTarget::Settings, "Opening Settings."),
        "inbox" => return screen(OpenTarget::Inbox, "Opening the Inbox."),
        "usage" => return screen(OpenTarget::Usage, "Opening Usage."),
        "home" => return screen(OpenTarget::Home, "Opening Home."),
        _ => {}
    }
    if let Some(d) = picture.dot(target) {
        return Step::Open(
            OpenTarget::Dot {
                dot_id: d.id.clone(),
            },
            format!("Opening {}.", name(&d.name)),
        );
    }
    if let Some(a) = picture.approval(target) {
        return Step::Open(
            OpenTarget::Approval {
                approval_id: a.id.clone(),
            },
            format!("Opening {}'s approval.", name(&a.dot_name)),
        );
    }
    Step::Say(CANT_OPEN.into())
}

/// The tool, and " on <target>" when there is a target, cut to 120 bytes.
fn tool_on(tool: &str, target: &str) -> (String, String) {
    let target = clean(target, SPOKEN_TARGET_BYTES);
    let on = if target.is_empty() {
        String::new()
    } else {
        format!(" on {target}")
    };
    (name(tool), on)
}

/// What needs the person, in a few sentences: the count, then the shortest items first (at
/// most three), then how many more; private approvals only counted.
pub fn needs_report(picture: &Picture) -> String {
    let private = picture.private_approvals as usize;
    let mut items: Vec<String> = picture
        .questions
        .iter()
        .map(|q| {
            sentence(&format!(
                "{} asks: {}",
                name(&q.dot_name),
                clean(&q.text, REPORT_QUESTION_BYTES)
            ))
        })
        .chain(picture.approvals.iter().map(|a| {
            format!(
                "{} wants approval to use {}.",
                name(&a.dot_name),
                name(&a.tool)
            )
        }))
        .collect();
    let described = items.len() + picture.more_questions + picture.more_approvals;
    let total = described + private;
    if total == 0 {
        return "Nothing needs you.".into();
    }
    let private_said = match private {
        0 => None,
        1 => Some("One private approval needs you.".to_string()),
        n => Some(format!("{} private approvals need you.", number(n))),
    };
    if described == 0 {
        return private_said.unwrap_or_default();
    }
    let mut out = vec![if total == 1 {
        "One thing needs you.".to_string()
    } else {
        format!("{} things need you.", number(total))
    }];
    items.sort_by_key(|s| s.len());
    items.truncate(MAX_REPORTED);
    let more = described - items.len();
    out.extend(items);
    if more > 0 {
        out.push(format!("And {more} more."));
    }
    out.extend(private_said);
    out.join(" ")
}

/// Carries out `deed` through the runtime, which checks it again, and returns the sentence to
/// say. Every error becomes one plain sentence.
pub async fn carry_out(rt: &Runtime, deed: &Deed) -> String {
    match deed {
        Deed::Approve { approval_id, .. } | Deed::Deny { approval_id, .. } => {
            let approved = matches!(deed, Deed::Approve { .. });
            match rt.decide_approval(approval_id, approved, None, None).await {
                Ok(_) if approved => "Approved.".into(),
                Ok(_) => "Denied.".into(),
                // decided already (`Conflict`) or gone (`NotFound`)
                Err(Error::Conflict(_)) | Err(Error::NotFound(_)) => HANDLED.into(),
                Err(e) => said_for(e, ""),
            }
        }
        Deed::Run { dot_id, dot_name } => match rt.run_now(dot_id).await {
            Ok(_) => format!("{} is running.", name(dot_name)),
            Err(e) => run_refused(e, dot_name),
        },
        Deed::Answer {
            message_id,
            dot_name,
            text,
        } => match rt.reply(message_id, text).await {
            Ok(_) => format!("Answered {}.", name(dot_name)),
            Err(Error::Conflict(m)) if m == QUESTION_ANSWERED || m == QUESTION_DISMISSED => {
                HANDLED.into()
            }
            Err(e) => said_for(e, dot_name),
        },
        Deed::Send {
            dot_id,
            dot_name,
            text,
            topic_root,
        } => match send(rt, dot_id, text, topic_root.as_deref()).await {
            Ok(()) => format!("Sent to {}.", name(dot_name)),
            Err(e) => send_refused(e, dot_name),
        },
    }
}

/// The sentence for a run the runtime refused: a dot that is gone is said so.
fn run_refused(e: Error, dot_name: &str) -> String {
    match e {
        Error::NotFound(_) => DOT_GONE.into(),
        e => said_for(e, dot_name),
    }
}

/// The sentence for a message the runtime refused. Only a thread with a run still working is
/// "still working"; any other refusal is the runtime's own sentence, which is written for a
/// person.
fn send_refused(e: Error, dot_name: &str) -> String {
    match e {
        Error::Conflict(m) if m == crate::runtime::CHAT_STILL_WORKING => {
            said_for(Error::Conflict(m), dot_name)
        }
        Error::Conflict(m) => {
            let m = plain_error(&m);
            let mut chars = m.chars();
            match chars.next() {
                None => "That didn't work.".into(),
                Some(first) => sentence(&format!("{}{}", first.to_uppercase(), chars.as_str())),
            }
        }
        Error::NotFound(_) => DOT_GONE.into(),
        e => said_for(e, dot_name),
    }
}

/// Sends `text` to the dot: a new topic, or after the end of the topic `root` as the chat
/// path does. A topic with no end left (its only message discarded) starts afresh.
async fn send(rt: &Runtime, dot_id: &str, text: &str, root: Option<&str>) -> Result<()> {
    let parent = match root {
        None => None,
        Some(root) => {
            let thread = rt.run_thread(root).await?;
            crate::send_later::thread_end(&thread).map(|r| r.id.clone())
        }
    };
    rt.chat(dot_id, text, parent.as_deref()).await?;
    Ok(())
}

/// The sentence for a refusal or failure of a deed of the dot `dot_name`.
fn said_for(e: Error, dot_name: &str) -> String {
    match e {
        Error::NotAllowed(t) => sentence(&clean(&t, REFUSAL_BYTES)),
        Error::Conflict(_) => format!(
            "{} is still working on that. Try again when it finishes.",
            if dot_name.is_empty() {
                "The dot".to_string()
            } else {
                name(dot_name)
            }
        ),
        Error::NotFound(_) => HANDLED.into(),
        Error::Invalid(m) | Error::Other(m) => didnt_work(&m),
        other => didnt_work(&other.to_string()),
    }
}

fn didnt_work(message: &str) -> String {
    let m = plain_error(message);
    if m.is_empty() {
        "That didn't work.".into()
    } else {
        sentence(&format!("That didn't work: {m}"))
    }
}

/// An error message for speech: on one line, words that look like ids taken out, cut to 160
/// bytes on a char boundary.
fn plain_error(message: &str) -> String {
    let words: Vec<&str> = message
        .split_whitespace()
        .filter(|w| !looks_like_id(w))
        .collect();
    clean(&words.join(" "), ERROR_BYTES)
}

/// A word holding a run of 16 or more letters and digits with at least one digit, such as a
/// ULID, a UUID part or a token.
fn looks_like_id(word: &str) -> bool {
    word.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .any(|part| part.len() >= 16 && part.chars().any(|c| c.is_ascii_digit()))
}

/// A dot or tool name for speech: on one line, without quotes around it.
fn name(s: &str) -> String {
    let n = clean(s, NAME_BYTES);
    n.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '“' | '”'))
        .to_string()
}

/// `s` ending in a full stop unless it ends in one already, or in a question or exclamation
/// mark.
fn sentence(s: &str) -> String {
    let s = s.trim_end();
    if s.ends_with(['.', '?', '!']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

/// "a", "a or b", "a, b, or c".
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} or {b}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// A count as a word, capitalised, up to ten; digits above.
fn number(n: usize) -> String {
    const WORDS: [&str; 11] = [
        "Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten",
    ];
    WORDS
        .get(n)
        .map(|w| w.to_string())
        .unwrap_or_else(|| n.to_string())
}

/// Whether `deed` waits for the person's spoken yes.
pub fn needs_confirmation(deed: &Deed) -> bool {
    matches!(
        deed,
        Deed::Approve { .. } | Deed::Deny { .. } | Deed::Run { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_dot::picture::{ApprovalLine, DotLine, QuestionLine};

    fn dot(handle: &str, name: &str) -> DotLine {
        DotLine {
            handle: handle.into(),
            id: format!("id-{name}"),
            name: name.into(),
            status: "idle".into(),
            needs_you: 0,
        }
    }

    fn approval(handle: &str, dot_name: &str, tool: &str, target: &str) -> ApprovalLine {
        ApprovalLine {
            handle: handle.into(),
            id: format!("ap-{handle}"),
            dot_id: format!("id-{dot_name}"),
            dot_name: dot_name.into(),
            tool: tool.into(),
            target: target.into(),
        }
    }

    fn question(handle: &str, dot_name: &str, text: &str, choices: &[&str]) -> QuestionLine {
        QuestionLine {
            handle: handle.into(),
            id: format!("m-{handle}"),
            dot_id: format!("id-{dot_name}"),
            dot_name: dot_name.into(),
            text: text.into(),
            choices: choices.iter().map(|c| c.to_string()).collect(),
        }
    }

    fn sample() -> Picture {
        Picture {
            dots: vec![
                dot("d1", "Research"),
                dot("d2", "Backup"),
                dot("d3", "Deploy watcher"),
                dot("d4", "Inbox sorter"),
                dot("d5", "Mail"),
            ],
            approvals: vec![
                approval("a1", "Research", "Bash", "git push origin main"),
                approval("a2", "Backup", "Glob", ""),
            ],
            questions: vec![question(
                "q1",
                "Research",
                "Which branch?",
                &["main", "dev", "release"],
            )],
            ..Picture::default()
        }
    }

    fn home() -> View {
        View {
            account: "personal".into(),
            workspace: "default".into(),
            screen: "home".into(),
            dot_id: None,
            topic_root: None,
        }
    }

    /// Every kind of wish, with handles that resolve and handles that do not.
    fn wishes() -> Vec<Wish> {
        let mut all = vec![Wish::Needs];
        for h in [
            "d1", "d2", "a1", "a2", "q1", "x", "", "Research", "d99", "a9", "q9",
        ] {
            all.push(Wish::ReadQuestion { q: h.into() });
            for unsure in [false, true] {
                all.push(Wish::Answer {
                    q: h.into(),
                    text: "main".into(),
                    unsure,
                });
            }
            all.push(Wish::Approve { a: h.into() });
            all.push(Wish::Deny { a: h.into() });
            for here in [false, true] {
                all.push(Wish::Send {
                    d: h.into(),
                    text: "hello".into(),
                    here,
                });
            }
            all.push(Wish::Run { d: h.into() });
            all.push(Wish::Open { target: h.into() });
        }
        for t in ["settings", "inbox", "usage", "home", "files"] {
            all.push(Wish::Open { target: t.into() });
        }
        all.push(Wish::Create {
            description: "watch my downloads".into(),
        });
        all.push(Wish::Ask {
            question: "Which one?".into(),
            options: vec!["a".into(), "b".into()],
        });
        all
    }

    #[test]
    fn approve_deny_run_always_confirm() {
        let mut viewing = home();
        viewing.dot_id = Some("id-Research".into());
        viewing.topic_root = Some("root-1".into());
        let pictures = [sample(), Picture::default(), {
            let mut p = sample();
            p.approvals.clear();
            p.private_approvals = 2;
            p
        }];
        let mut confirms = 0;
        for p in &pictures {
            for view in [home(), viewing.clone()] {
                for wish in wishes() {
                    let step = plan(&wish, p, &view);
                    if let Step::Do(d) = &step {
                        assert!(!needs_confirmation(d), "{wish:?} gave {step:?}");
                    }
                    if let Step::Ask {
                        then: AskFor::AnswerCheck { deed },
                        ..
                    } = &step
                    {
                        assert!(!needs_confirmation(deed), "{wish:?} gave {step:?}");
                    }
                    if let Step::Confirm { deed, .. } = &step {
                        assert!(needs_confirmation(deed), "{wish:?} gave {step:?}");
                        confirms += 1;
                    }
                }
            }
        }
        assert!(confirms > 0);
        let send = Deed::Send {
            dot_id: "x".into(),
            dot_name: "X".into(),
            text: "t".into(),
            topic_root: None,
        };
        let answer = Deed::Answer {
            message_id: "m".into(),
            dot_name: "X".into(),
            text: "t".into(),
        };
        assert!(!needs_confirmation(&send) && !needs_confirmation(&answer));
        let run = Deed::Run {
            dot_id: "x".into(),
            dot_name: "X".into(),
        };
        assert!(needs_confirmation(&run));
    }

    #[test]
    fn confirm_question_uses_the_record_not_the_model() {
        let p = sample();
        let v = home();
        assert_eq!(
            plan(&Wish::Approve { a: "a1".into() }, &p, &v),
            Step::Confirm {
                deed: Deed::Approve {
                    approval_id: "ap-a1".into(),
                    dot_name: "Research".into(),
                    tool: "Bash".into(),
                    target: "git push origin main".into(),
                },
                question: "Research wants to use Bash on git push origin main. Approve it?".into(),
            }
        );
        let Step::Confirm { question, .. } = plan(&Wish::Deny { a: "a1".into() }, &p, &v) else {
            panic!("deny must confirm");
        };
        assert_eq!(
            question,
            "Deny Research's request to use Bash on git push origin main?"
        );
        // no target: only the tool
        let Step::Confirm { question, .. } = plan(&Wish::Approve { a: "a2".into() }, &p, &v) else {
            panic!("approve must confirm");
        };
        assert_eq!(question, "Backup wants to use Glob. Approve it?");
        let Step::Confirm { question, .. } = plan(&Wish::Deny { a: "a2".into() }, &p, &v) else {
            panic!("deny must confirm");
        };
        assert_eq!(question, "Deny Backup's request to use Glob?");
        let Step::Confirm { question, deed } = plan(&Wish::Run { d: "d2".into() }, &p, &v) else {
            panic!("run must confirm");
        };
        assert_eq!(question, "Run Backup now?");
        assert_eq!(
            deed,
            Deed::Run {
                dot_id: "id-Backup".into(),
                dot_name: "Backup".into()
            }
        );
        // a long target is cut to 120 bytes on a char boundary
        let mut long = sample();
        long.approvals[0].target = format!("echo {}", "ü".repeat(100));
        let Step::Confirm { question, deed } = plan(&Wish::Approve { a: "a1".into() }, &long, &v)
        else {
            panic!("approve must confirm");
        };
        let said = question
            .strip_prefix("Research wants to use Bash on ")
            .and_then(|s| s.strip_suffix(". Approve it?"))
            .expect(&question);
        assert!(said.len() <= 120 && said.len() >= 119, "{}", said.len());
        assert!(long.approvals[0].target.starts_with(said));
        // the deed keeps the record as it is
        let Deed::Approve { target, .. } = deed else {
            panic!()
        };
        assert_eq!(target, long.approvals[0].target);
    }

    #[test]
    fn unknown_dot_asks_with_closest_names() {
        let p = sample();
        let v = home();
        let wish = Wish::Run { d: "deploy".into() };
        assert_eq!(
            plan(&wish, &p, &v),
            Step::Ask {
                question: "Which dot did you mean?".into(),
                options: vec!["Deploy watcher".into()],
                then: AskFor::Dot {
                    wish: wish.clone(),
                    handles: vec!["d3".into()],
                },
            }
        );
        // nothing close: the first four dots
        let wish = Wish::Send {
            d: "d42".into(),
            text: "hi".into(),
            here: false,
        };
        let Step::Ask { options, then, .. } = plan(&wish, &p, &v) else {
            panic!("an unknown dot asks");
        };
        assert_eq!(
            options,
            vec!["Research", "Backup", "Deploy watcher", "Inbox sorter"]
        );
        assert_eq!(
            then,
            AskFor::Dot {
                wish,
                handles: vec!["d1".into(), "d2".into(), "d3".into(), "d4".into()],
            }
        );
        // a handle the picture does not hold never resolves by position
        assert!(!matches!(
            plan(&Wish::Run { d: "d6".into() }, &p, &v),
            Step::Confirm { .. }
        ));
        // no dots at all
        assert_eq!(
            plan(&Wish::Run { d: "d1".into() }, &Picture::default(), &v),
            Step::Say("I can't find that dot.".into())
        );
    }

    #[test]
    fn stale_approval_handle_says_handled() {
        let p = sample();
        let v = home();
        let gone = Step::Say("I can't find that one. It may already be handled.".into());
        for a in ["a3", "a10", "A1", "", "Research", "ap-a1", "1"] {
            assert_eq!(plan(&Wish::Approve { a: a.into() }, &p, &v), gone, "{a}");
            assert_eq!(plan(&Wish::Deny { a: a.into() }, &p, &v), gone, "{a}");
        }
        for q in ["q2", "Q1", "m-q1"] {
            assert_eq!(plan(&Wish::ReadQuestion { q: q.into() }, &p, &v), gone);
            let answer = Wish::Answer {
                q: q.into(),
                text: "main".into(),
                unsure: false,
            };
            assert_eq!(plan(&answer, &p, &v), gone);
        }
        // the private ones are only counted: no handle reaches them
        let mut private = sample();
        private.approvals.clear();
        private.private_approvals = 3;
        assert_eq!(plan(&Wish::Approve { a: "a1".into() }, &private, &v), gone);
    }

    #[test]
    fn reading_a_question() {
        let mut p = sample();
        let v = home();
        assert_eq!(
            plan(&Wish::ReadQuestion { q: "q1".into() }, &p, &v),
            Step::Say(
                "Research asks: Which branch? The choices are: main, dev, or release.".into()
            )
        );
        p.questions[0].choices = vec!["main".into(), "dev".into()];
        assert_eq!(
            plan(&Wish::ReadQuestion { q: "q1".into() }, &p, &v),
            Step::Say("Research asks: Which branch? The choices are: main or dev.".into())
        );
        p.questions[0].choices.clear();
        p.questions[0].text = "Ship it".into();
        assert_eq!(
            plan(&Wish::ReadQuestion { q: "q1".into() }, &p, &v),
            Step::Say("Research asks: Ship it.".into())
        );
    }

    #[test]
    fn unsure_answer_is_read_back() {
        let p = sample();
        let v = home();
        let deed = Deed::Answer {
            message_id: "m-q1".into(),
            dot_name: "Research".into(),
            text: "the main one".into(),
        };
        let sure = Wish::Answer {
            q: "q1".into(),
            text: "the main one".into(),
            unsure: false,
        };
        assert_eq!(plan(&sure, &p, &v), Step::Do(deed.clone()));
        let unsure = Wish::Answer {
            q: "q1".into(),
            text: "the main one".into(),
            unsure: true,
        };
        assert_eq!(
            plan(&unsure, &p, &v),
            Step::Ask {
                question: "I heard: the main one. Send that?".into(),
                options: vec!["Yes".into(), "No".into()],
                then: AskFor::AnswerCheck { deed },
            }
        );
        // an empty answer is not sent
        let empty = Wish::Answer {
            q: "q1".into(),
            text: "  ".into(),
            unsure: false,
        };
        assert!(matches!(plan(&empty, &p, &v), Step::Say(_)));
    }

    #[test]
    fn send_here_uses_the_viewed_topic_only_for_that_dot() {
        let p = sample();
        let send = |d: &str, here: bool| Wish::Send {
            d: d.into(),
            text: "hello".into(),
            here,
        };
        let mut v = home();
        v.dot_id = Some("id-Research".into());
        v.topic_root = Some("root-1".into());
        let root = |step: Step| match step {
            Step::Do(Deed::Send { topic_root, .. }) => topic_root,
            other => panic!("{other:?}"),
        };
        assert_eq!(root(plan(&send("d1", true), &p, &v)), Some("root-1".into()));
        // not "here": a new topic
        assert_eq!(root(plan(&send("d1", false), &p, &v)), None);
        // another dot than the one on screen: a new topic
        assert_eq!(root(plan(&send("d2", true), &p, &v)), None);
        // the dot on screen with no topic open: a new topic
        v.topic_root = None;
        assert_eq!(root(plan(&send("d1", true), &p, &v)), None);
        assert_eq!(
            plan(&send("d1", false), &p, &v),
            Step::Do(Deed::Send {
                dot_id: "id-Research".into(),
                dot_name: "Research".into(),
                text: "hello".into(),
                topic_root: None,
            })
        );
    }

    #[test]
    fn open_targets() {
        let p = sample();
        let v = home();
        let open = |t: &str| plan(&Wish::Open { target: t.into() }, &p, &v);
        assert_eq!(
            open("d2"),
            Step::Open(
                OpenTarget::Dot {
                    dot_id: "id-Backup".into()
                },
                "Opening Backup.".into()
            )
        );
        assert_eq!(
            open("a1"),
            Step::Open(
                OpenTarget::Approval {
                    approval_id: "ap-a1".into()
                },
                "Opening Research's approval.".into()
            )
        );
        assert_eq!(
            open("settings"),
            Step::Open(OpenTarget::Settings, "Opening Settings.".into())
        );
        assert_eq!(
            open("inbox"),
            Step::Open(OpenTarget::Inbox, "Opening the Inbox.".into())
        );
        assert_eq!(
            open("usage"),
            Step::Open(OpenTarget::Usage, "Opening Usage.".into())
        );
        assert_eq!(
            open("home"),
            Step::Open(OpenTarget::Home, "Opening Home.".into())
        );
        for t in ["files", "d9", "Research", "C:\\Windows", ""] {
            assert_eq!(open(t), Step::Say("I can't open that.".into()), "{t}");
        }
    }

    #[test]
    fn model_asks_and_creates_pass_through() {
        let p = sample();
        let v = home();
        let options: Vec<String> = (1..=9).map(|i| format!("option {i}")).collect();
        let Step::Ask {
            question,
            options: got,
            then,
        } = plan(
            &Wish::Ask {
                question: "Which one?".into(),
                options,
            },
            &p,
            &v,
        )
        else {
            panic!("an ask passes through");
        };
        assert_eq!(question, "Which one?");
        assert_eq!(got.len(), 6);
        assert_eq!(then, AskFor::Model);
        assert_eq!(
            plan(
                &Wish::Create {
                    description: "watch my downloads".into()
                },
                &p,
                &v
            ),
            Step::Create {
                description: "watch my downloads".into()
            }
        );
        assert_eq!(plan(&Wish::Needs, &p, &v), Step::Say(needs_report(&p)));
    }

    #[test]
    fn needs_report_shapes() {
        // none
        assert_eq!(needs_report(&Picture::default()), "Nothing needs you.");
        // one
        let one = Picture {
            approvals: vec![approval("a1", "Backup", "Bash", "npm test")],
            ..Picture::default()
        };
        assert_eq!(
            needs_report(&one),
            "One thing needs you. Backup wants approval to use Bash."
        );
        // two, shortest first
        let two = Picture {
            approvals: vec![approval("a1", "Backup", "Bash", "npm test")],
            questions: vec![question(
                "q1",
                "Research",
                "Which branch should I use?",
                &[],
            )],
            ..Picture::default()
        };
        assert_eq!(
            needs_report(&two),
            "Two things need you. Backup wants approval to use Bash. \
             Research asks: Which branch should I use?"
        );
        // three
        let three = Picture {
            approvals: vec![
                approval("a1", "Backup", "Bash", "npm test"),
                approval("a2", "Mail", "Read", "notes.md"),
            ],
            questions: vec![question(
                "q1",
                "Research",
                "Which branch should I use?",
                &[],
            )],
            ..Picture::default()
        };
        assert_eq!(
            needs_report(&three),
            "Three things need you. Mail wants approval to use Read. \
             Backup wants approval to use Bash. Research asks: Which branch should I use?"
        );
        // five: three said, the rest counted; a long question is cut to 160 bytes
        let long = "é".repeat(200);
        let five = Picture {
            approvals: vec![
                approval("a1", "Backup", "Bash", "npm test"),
                approval("a2", "Mail", "Read", "notes.md"),
            ],
            questions: vec![
                question("q1", "Research", &long, &[]),
                question("q2", "Deploy watcher", "Ship?", &[]),
            ],
            more_approvals: 1,
            ..Picture::default()
        };
        let said = needs_report(&five);
        assert_eq!(
            said,
            "Five things need you. Deploy watcher asks: Ship? \
             Mail wants approval to use Read. Backup wants approval to use Bash. And 2 more."
        );
        let mut four = five.clone();
        four.more_approvals = 0;
        four.questions.swap(0, 1);
        four.approvals.clear();
        four.dots.clear();
        let said = needs_report(&four);
        let cut = said
            .strip_prefix("Two things need you. Deploy watcher asks: Ship? Research asks: ")
            .expect(&said);
        let cut = cut.trim_end_matches('.');
        assert!(cut.len() <= 160 && cut.len() >= 159, "{}", cut.len());
        assert!(long.starts_with(cut));
        // private only
        let private = Picture {
            private_approvals: 1,
            ..Picture::default()
        };
        assert_eq!(needs_report(&private), "One private approval needs you.");
        let private = Picture {
            private_approvals: 2,
            ..Picture::default()
        };
        assert_eq!(needs_report(&private), "Two private approvals need you.");
        // private with others: counted in the total, said last
        let mixed = Picture {
            private_approvals: 1,
            ..one.clone()
        };
        assert_eq!(
            needs_report(&mixed),
            "Two things need you. Backup wants approval to use Bash. \
             One private approval needs you."
        );
    }

    #[test]
    fn a_refused_send_says_why() {
        use crate::runtime::{CHAT_AWAITS_APPROVAL, CHAT_STILL_WORKING};
        let conflict = |m: &str| Error::Conflict(m.into());
        assert_eq!(
            send_refused(conflict(CHAT_STILL_WORKING), "Research"),
            "Research is still working on that. Try again when it finishes."
        );
        // any other refusal is the runtime's own sentence, said as one
        assert_eq!(
            send_refused(conflict(CHAT_AWAITS_APPROVAL), "Research"),
            "This chat is waiting for an approval; answer it, then send your message again."
        );
        assert_eq!(
            send_refused(conflict(crate::checkins::DECIDING), "Research"),
            "Dots is still adding the app you accepted in this chat; send your message again \
             in a moment."
        );
        assert_eq!(
            send_refused(
                conflict("run 01KA2B3C4D5E6F7G8H9J0K1M2N has a newer message"),
                "Research"
            ),
            "Run has a newer message."
        );
        let long = send_refused(conflict(&"ü".repeat(200)), "Research");
        assert!(long.len() <= 161, "{}", long.len());
        assert_eq!(
            send_refused(Error::NotFound("dot x".into()), "Research"),
            "I can't find that dot anymore."
        );
        assert_eq!(
            run_refused(Error::NotFound("dot x".into()), "Research"),
            "I can't find that dot anymore."
        );
        assert_eq!(
            run_refused(Error::Conflict("busy".into()), "Research"),
            "Research is still working on that. Try again when it finishes."
        );
    }

    #[test]
    fn plain_errors_drop_ids() {
        assert_eq!(
            plain_error("run 01KA2B3C4D5E6F7G8H9J0K1M2N belongs to a different dot"),
            "run belongs to a different dot"
        );
        assert_eq!(plain_error("line\nbreak"), "line break");
        let long = plain_error(&"ü".repeat(200));
        assert!(long.len() <= 160 && long.len() >= 159);
    }
}

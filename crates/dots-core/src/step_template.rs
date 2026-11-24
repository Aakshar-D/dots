//! The texts of a step that runs without a model (general workflows spec section 6): an
//! action's URL, headers and body, a file's path and content. A text holds `{{…}}` marks that
//! are filled when the step runs, from the workflow run alone:
//!
//! - `{{task}}`: the run's task;
//! - `{{input.<name>}}`: one of the run's inputs;
//! - `{{<step>.<name>}}`: an output of an earlier agent step, by the step's id;
//! - `{{<step>.summary}}`: that step's summary;
//! - `{{secret.<NAME>}}`: a Personal variable of this PC, read when the step runs and never
//!   stored with the run.
//!
//! This module parses the marks and fills them; what each one may name is the definition's to
//! check (`workflow::Definition`), and where its value comes from is the run's (`workflows`).

/// One mark of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Task,
    Input(String),
    Secret(String),
    /// An output of the step `step`.
    Output {
        step: String,
        name: String,
    },
    /// The summary of the step `step`.
    Summary {
        step: String,
    },
}

impl Mark {
    /// The mark as it is written, without its braces.
    pub fn written(&self) -> String {
        match self {
            Mark::Task => "task".to_string(),
            Mark::Input(name) => format!("input.{name}"),
            Mark::Secret(name) => format!("secret.{name}"),
            Mark::Output { step, name } => format!("{step}.{name}"),
            Mark::Summary { step } => format!("{step}.summary"),
        }
    }
}

/// A piece of a text: words kept as they are, or a mark to fill.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece<'a> {
    Words(&'a str),
    Mark(Mark),
}

/// Whether `name` is 1 to 40 of `a-z`, `0-9`, `-` and `_`: a step id, an input's or an
/// output's name.
pub fn name_ok(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Whether `name` can be a Personal variable's: 1 to 64 of `A-Z`, `a-z`, `0-9` and `_`, not
/// starting with a digit.
fn secret_ok(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

/// The mark written as `inner` (what stands between the braces), or why it is none.
fn mark_of(inner: &str) -> Result<Mark, String> {
    let written = inner.trim();
    let unknown = || format!("{{{{{written}}}}} is not something a step can use");
    if written == "task" {
        return Ok(Mark::Task);
    }
    let Some((head, tail)) = written.split_once('.') else {
        return Err(unknown());
    };
    match head {
        "input" if name_ok(tail) => Ok(Mark::Input(tail.to_string())),
        "secret" if secret_ok(tail) => Ok(Mark::Secret(tail.to_string())),
        "input" | "secret" => Err(unknown()),
        step if name_ok(step) && tail == "summary" => Ok(Mark::Summary {
            step: step.to_string(),
        }),
        step if name_ok(step) && name_ok(tail) => Ok(Mark::Output {
            step: step.to_string(),
            name: tail.to_string(),
        }),
        _ => Err(unknown()),
    }
}

/// `text` as its pieces, in order. `{{` with no `}}` after it is refused: it is a mark left
/// half written, not words.
fn pieces(text: &str) -> Result<Vec<Piece<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        if open > 0 {
            out.push(Piece::Words(&rest[..open]));
        }
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err("a {{ has no }} after it".to_string());
        };
        out.push(Piece::Mark(mark_of(&after[..close])?));
        rest = &after[close + 2..];
    }
    if !rest.is_empty() {
        out.push(Piece::Words(rest));
    }
    Ok(out)
}

/// The marks of `text`, in order, or why one of them is none.
pub fn marks(text: &str) -> Result<Vec<Mark>, String> {
    Ok(pieces(text)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Mark(m) => Some(m),
            Piece::Words(_) => None,
        })
        .collect())
}

/// `text` with each mark replaced by what `value` gives for it; `value`'s refusal ends the
/// fill with it. The words between the marks are kept as they are.
pub fn fill(
    text: &str,
    mut value: impl FnMut(&Mark) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text)? {
        match piece {
            Piece::Words(words) => out.push_str(words),
            Piece::Mark(mark) => out.push_str(&value(&mark)?),
        }
    }
    Ok(out)
}

/// The value an agent step gave for its output `name`: what stands between the last
/// `<name>` and the `</name>` after it in `reply`, trimmed. `None` when the reply has no such
/// block. The last one counts, so a reply that shows a draft and then the final text gives the
/// final text.
pub fn output_of(reply: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = reply.rfind(&open)? + open.len();
    let end = reply[start..].find(&close)? + start;
    Some(reply[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_filled_mark_by_mark_and_its_words_are_kept() {
        let text =
            "Hi {{input.name}}, {{ draft.post }} ({{task}}) {{draft.summary}} {{secret.API_KEY}}!";
        assert_eq!(
            marks(text).unwrap(),
            vec![
                Mark::Input("name".into()),
                Mark::Output {
                    step: "draft".into(),
                    name: "post".into()
                },
                Mark::Task,
                Mark::Summary {
                    step: "draft".into()
                },
                Mark::Secret("API_KEY".into()),
            ]
        );
        let filled = fill(text, |m| Ok(format!("[{}]", m.written()))).unwrap();
        assert_eq!(
            filled,
            "Hi [input.name], [draft.post] ([task]) [draft.summary] [secret.API_KEY]!"
        );
        // No mark: the text as it is, a single brace included.
        assert_eq!(
            fill("{\"a\": 1}", |_| unreachable!()).unwrap(),
            "{\"a\": 1}"
        );
        assert_eq!(marks("").unwrap(), vec![]);
    }

    #[test]
    fn a_mark_that_names_nothing_and_a_half_written_one_are_refused() {
        for (text, why) in [
            ("{{nope}}", "{{nope}} is not something a step can use"),
            ("{{input.}}", "{{input.}} is not something a step can use"),
            (
                "{{input.Bad Name}}",
                "{{input.Bad Name}} is not something a step can use",
            ),
            (
                "{{secret.1ST}}",
                "{{secret.1ST}} is not something a step can use",
            ),
            ("{{a.b.c}}", "{{a.b.c}} is not something a step can use"),
            ("{{}}", "{{}} is not something a step can use"),
            ("so {{task", "a {{ has no }} after it"),
        ] {
            assert_eq!(marks(text).unwrap_err(), why, "{text}");
        }
        // A value's refusal ends the fill.
        assert_eq!(
            fill("{{task}} {{input.a}}", |m| match m {
                Mark::Task => Ok("t".into()),
                _ => Err("no such input".into()),
            })
            .unwrap_err(),
            "no such input"
        );
    }

    #[test]
    fn an_output_is_the_last_block_of_its_name() {
        let reply = "A draft: <post>first</post>\n\nFinal:\n<post>\n  second, final\n</post>\n<caption>c</caption>";
        assert_eq!(output_of(reply, "post").as_deref(), Some("second, final"));
        assert_eq!(output_of(reply, "caption").as_deref(), Some("c"));
        assert_eq!(output_of(reply, "title"), None);
        // An open block with no end gives nothing.
        assert_eq!(output_of("<post>never closed", "post"), None);
        assert_eq!(output_of("<post></post>", "post").as_deref(), Some(""));
    }
}

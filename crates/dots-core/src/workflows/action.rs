//! Action steps (general workflows spec section 6): Dots itself sends one HTTP request or
//! writes one file, with no model. The step's texts hold `{{…}}` marks (`step_template`),
//! filled when it runs from the workflow run: its task, its inputs, the outputs and summaries
//! of the agent steps before it, and Personal variables of this PC.
//!
//! An action runs as a job outside the loop's lock, as a deploy does (`jobs`). What it did is
//! the step run's hand-off summary, which the run page shows. A request that Dots stopped
//! during may have gone out, so it is not sent again without the person: the step fails
//! `interrupted`, and Retry sends it again. A file is written again.
//!
//! A secret never reaches the store: where a text that is kept (the summary, an error) holds
//! one, it is replaced.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use super::{succeeded, Workflows, INTERRUPTED};
use crate::model::{
    Handoff, StepRun, StepRunChange, StepRunStatus, WorkflowRun, WorkflowRunChange,
};
use crate::step_template::{self, Mark};
use crate::vault::Vault;
use crate::workflow::{relative_path_ok, Action, Header, HttpMethod, Step, StepKind};
use crate::{Error, Result};

/// How long an HTTP action waits for its answer.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The most of an answer's body kept in the step's summary or error, in characters.
const KEPT_BODY: usize = 1_000;
/// The most of an answer a later step may use, in characters.
const KEPT_ANSWER: usize = 20_000;
/// What stands for a secret in a text that is kept.
const HIDDEN: &str = "•••";
/// Why a secret is refused in a team of an account: its variables are the account's to grant.
const PERSONAL_ONLY: &str =
    "a secret can be used only by a team in Personal: an account's variables are not read here";

/// How a value is written into the text around it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// As it is.
    None,
    /// To sit inside a JSON string.
    Json,
    /// To sit inside a URL's path or query, or a form body.
    Url,
    /// To be one part of a file's path: it may name no folder of its own.
    PathPart,
}

/// `value` as it sits inside a JSON string: without the quotes around it.
fn json_escaped(value: &str) -> String {
    let quoted = serde_json::Value::String(value.to_string()).to_string();
    quoted[1..quoted.len() - 1].to_string()
}

/// `value` percent-encoded, every byte but the unreserved ones (RFC 3986).
fn url_escaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn escaped(value: String, escape: Escape) -> String {
    match escape {
        Escape::None => value,
        Escape::Json => json_escaped(&value),
        Escape::Url => url_escaped(&value),
        Escape::PathPart => value,
    }
}

/// How the values of a body are written: a body that is JSON (it says so in `Content-Type`, or
/// names none and starts with `{` or `[`) escapes them for a JSON string, a form body
/// percent-encodes them, and any other takes them as they are.
fn body_escape(headers: &[Header], body: &str) -> Escape {
    let content_type = headers
        .iter()
        .find(|h| h.name.trim().eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase());
    match content_type {
        Some(t) if t.contains("json") => Escape::Json,
        Some(t) if t.contains("x-www-form-urlencoded") => Escape::Url,
        Some(_) => Escape::None,
        // A body that opens with a mark (`{{draft.post}}`) is not JSON for that.
        None if looks_like_json(body) => Escape::Json,
        None => Escape::None,
    }
}

/// Whether `body` reads as JSON: it opens with `{` or `[`, and not with a mark's `{{`.
fn looks_like_json(body: &str) -> bool {
    let body = body.trim_start();
    body.starts_with('[') || (body.starts_with('{') && !body.starts_with("{{"))
}

/// `text` in at most `max` characters, ending with an ellipsis when it was longer.
fn clipped(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// What an action's texts are filled from, and the secrets read on the way.
struct Values<'a> {
    wr: &'a WorkflowRun,
    vault: Option<&'a Vault>,
    /// Whether the run's team is in Personal: only then are secrets read.
    personal: bool,
    /// The secrets read so far, to hide in what is kept.
    secrets: Vec<String>,
}

impl Values<'_> {
    /// The value of `mark`, or why it has none.
    async fn value(&mut self, mark: &Mark) -> std::result::Result<String, String> {
        let written = format!("{{{{{}}}}}", mark.written());
        let wr = self.wr;
        // The newest succeeded run of the agent step `step`: on a path the run did not take,
        // it has none.
        let handoff = |step: &str| {
            succeeded(wr, step)
                .and_then(|s| s.handoff.as_ref())
                .ok_or_else(|| format!("{written}: that step has not run in this workflow run"))
        };
        match mark {
            Mark::Task => Ok(wr.task.clone()),
            Mark::Input(name) => Ok(wr.inputs.get(name).cloned().unwrap_or_default()),
            Mark::Summary { step } => Ok(handoff(step)?.summary.clone()),
            Mark::Output { step, name } => handoff(step)?
                .outputs
                .get(name)
                .cloned()
                .ok_or_else(|| format!("{written}: that step gave no such output")),
            Mark::Secret(name) => {
                if !self.personal {
                    return Err(format!("{written}: {PERSONAL_ONLY}"));
                }
                let Some(vault) = self.vault else {
                    return Err(format!("{written}: the variables cannot be read here"));
                };
                let found = vault
                    .personal_values(std::slice::from_ref(name))
                    .await
                    .map_err(|e| format!("{written}: {e}"))?;
                let Some(value) = found.get(name) else {
                    return Err(format!(
                        "{written}: there is no Personal variable named {name}"
                    ));
                };
                if !value.is_empty() {
                    self.secrets.push(value.clone());
                }
                Ok(value)
            }
        }
    }

    /// `text` with its marks filled, each value written as `escape` says.
    async fn fill(&mut self, text: &str, escape: Escape) -> std::result::Result<String, String> {
        // The values first: a mark's value may need the vault, and `fill` takes no future.
        let mut values = Vec::new();
        for mark in step_template::marks(text)? {
            let value = self.value(&mark).await?;
            let written = format!("{{{{{}}}}}", mark.written());
            // What a step wrote may fill a place in a path, never choose the path: no folder
            // of its own in a file's path, and no `.` or `..` that would move a URL's.
            match escape {
                Escape::PathPart if value.contains(['/', '\\']) => {
                    return Err(format!(
                        "{written} holds a slash, and may not add a folder to the file's path"
                    ));
                }
                Escape::Url if matches!(value.trim(), "." | "..") => {
                    return Err(format!(
                        "{written} is only dots, which would change the address"
                    ));
                }
                _ => {}
            }
            values.push(value);
        }
        let mut values = values.into_iter();
        step_template::fill(text, |_| {
            Ok(escaped(values.next().unwrap_or_default(), escape))
        })
    }

    /// `text` with every secret read so far replaced: what is kept never holds one.
    fn hidden(&self, text: &str) -> String {
        let mut out = text.to_string();
        // The longest first: a secret that starts with another is hidden whole.
        let mut secrets: Vec<&String> = self.secrets.iter().collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for secret in secrets {
            out = out.replace(secret.as_str(), HIDDEN);
            out = out.replace(&url_escaped(secret), HIDDEN);
            out = out.replace(&json_escaped(secret), HIDDEN);
        }
        out
    }
}

/// The URL `url` as a kept text shows it: without its query, where a key often rides.
fn shown_url(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

impl Workflows {
    /// The vault a `{{secret.…}}` mark is read from, set once by the runtime.
    pub fn set_vault(&self, vault: Vault) {
        let _ = self.vault.set(vault);
    }

    /// Begins the action step `step` as its next attempt: a step run that stays `Running`
    /// while a job acts.
    pub(super) async fn begin_action(&self, wr: &WorkflowRun, step: &Step) -> Result<()> {
        if !matches!(step.kind, StepKind::Action { .. }) {
            return Err(Error::Other(format!("step {} is not an action", step.id)));
        }
        // A step run a begin cut off before the move below left running is used, not doubled.
        let step_run = self
            .step_run_to_begin(wr, step, StepRunStatus::Running)
            .await?;
        let wr = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    current_step: Some(Some(step.id.clone())),
                    ..Default::default()
                },
            )
            .await?;
        // In the locked section that made the step run: no pass finds it without its job.
        self.start_action(&wr, &step_run);
        self.updated(&wr);
        Ok(())
    }

    /// Watches an action step run: its job is in flight, or Dots stopped during it. A file is
    /// written again; a request may have gone out, so it fails `interrupted` and waits for the
    /// person's Retry.
    pub(super) async fn watch_action(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
    ) -> Result<()> {
        if self.has_job(&step_run.id) {
            return Ok(());
        }
        match &step.kind {
            StepKind::Action {
                action: Action::File { .. },
                ..
            } => {
                self.start_action(wr, step_run);
                Ok(())
            }
            _ => self.fail_step(wr, step, step_run, INTERRUPTED).await,
        }
    }

    /// Starts the job that performs the step run `step_run` (`act`).
    fn start_action(&self, wr: &WorkflowRun, step_run: &StepRun) {
        let id = wr.id.clone();
        let step_run_id = step_run.id.clone();
        self.start_job(&step_run.id, move |this| async move {
            this.act(id, step_run_id).await
        });
    }

    /// The action job (`start_action`): reads the run under the lock (a Stop while the job
    /// waited wins, and nothing is sent), acts unlocked, then, if the step run is still open,
    /// succeeds it with what it did and moves on, or fails it with the error: the run waits
    /// for the person to retry it or stop.
    async fn act(&self, id: String, step_run_id: String) -> Result<()> {
        if !self.through_gate().await {
            return Ok(());
        }
        let wr = {
            let _one = self.lock.lock().await;
            match self.still_open(&id, &step_run_id).await? {
                Some((wr, _)) => wr,
                None => return Ok(()),
            }
        };
        let step_id = wr
            .steps
            .iter()
            .find(|s| s.id == step_run_id)
            .map(|s| s.step_id.clone())
            .unwrap_or_default();
        let done = self.act_now(&wr, &step_id).await;
        let _one = self.lock.lock().await;
        let Some((wr, step_run)) = self.still_open(&id, &step_run_id).await? else {
            return Ok(());
        };
        let Some(step) = wr.definition.step(&step_run.step_id) else {
            return Err(Error::Other(format!(
                "step {} is not in the workflow",
                step_run.step_id
            )));
        };
        match done {
            Ok((summary, outputs)) => {
                let handoff = Handoff {
                    summary,
                    outputs,
                    ..Default::default()
                };
                self.store
                    .set_step_run(
                        &step_run.id,
                        &StepRunChange {
                            status: Some(StepRunStatus::Succeeded),
                            handoff: Some(Some(handoff)),
                            ..Default::default()
                        },
                    )
                    .await?;
                let wr = self.store.get_workflow_run(&wr.id).await?;
                self.move_on(&wr, step).await
            }
            Err(error) => self.fail_step(&wr, step, &step_run, &error).await,
        }
    }

    /// Performs the action step `step_id` of `wr`. `Ok` says what it did, `Err` why it could
    /// not, both without any secret. Slow: run without the lock.
    async fn act_now(&self, wr: &WorkflowRun, step_id: &str) -> std::result::Result<Done, String> {
        let Some(StepKind::Action { action, .. }) = wr.definition.step(step_id).map(|s| &s.kind)
        else {
            return Err(format!("step {step_id} is not an action"));
        };
        let personal = match self.store.get_team(&wr.team_id).await {
            Ok(team) => team.org_id.is_none(),
            Err(e) => return Err(format!("the team cannot be read: {e}")),
        };
        let mut values = Values {
            wr,
            vault: self.vault.get(),
            personal,
            secrets: Vec::new(),
        };
        let done = match action {
            Action::Http {
                method,
                url,
                headers,
                body,
            } => send(&mut values, wr.test, *method, url, headers, body).await,
            Action::File { path, content } => {
                write(&mut values, Path::new(&wr.project.repo_path), path, content).await
            }
            Action::Telegram { token, chat, text } => {
                let base = self.telegram_base().to_string();
                message(&mut values, wr.test, &base, token, chat, text).await
            }
        };
        // What a later step may use of it, with no secret in it either.
        match done {
            Ok((summary, outputs)) => Ok((
                values.hidden(&summary),
                outputs
                    .into_iter()
                    .map(|(name, text)| (name, values.hidden(&text)))
                    .collect(),
            )),
            Err(error) => Err(values.hidden(&error)),
        }
    }
}

/// What an action did: its summary, and what a later step may use of it by name
/// (`Action::gives`).
type Done = (String, BTreeMap<String, String>);

/// `pairs` as the outputs of an action.
fn given<const N: usize>(pairs: [(&str, String); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(name, text)| (name.to_string(), text))
        .collect()
}

/// Sends the message of a Telegram action as its bot. In a test run nothing is sent.
async fn message(
    values: &mut Values<'_>,
    test: bool,
    base: &str,
    token: &str,
    chat: &str,
    text: &str,
) -> std::result::Result<Done, String> {
    let chat = values.fill(chat.trim(), Escape::None).await?;
    let text = values.fill(text, Escape::None).await?;
    let secret = values
        .value(&Mark::Secret(token.trim().to_string()))
        .await?;
    if test {
        return Ok((
            format!(
                "Test run: not sent. It would send a message of {} characters to the chat {}.",
                text.trim().chars().count(),
                chat.trim()
            ),
            given([("answer", String::new())]),
        ));
    }
    let answer = super::telegram::send_message(base, &secret, &chat, &text).await?;
    let answer = values.hidden(&answer);
    Ok((
        format!(
            "Sent a message of {} characters to the chat {}.",
            text.trim().chars().count(),
            chat.trim()
        ),
        given([("answer", clipped(&answer, KEPT_ANSWER))]),
    ))
}

/// Sends the request of an HTTP action. In a test run nothing is sent: the summary says what
/// would be.
async fn send(
    values: &mut Values<'_>,
    test: bool,
    method: HttpMethod,
    url: &str,
    headers: &[Header],
    body: &str,
) -> std::result::Result<Done, String> {
    let url = values.fill(url.trim(), Escape::Url).await?;
    let parsed = reqwest::Url::parse(&url).map_err(|e| format!("the URL cannot be read: {e}"))?;
    let host = parsed.host_str().unwrap_or_default().to_string();
    let original_body = body;
    let body = values.fill(body, body_escape(headers, body)).await?;
    let mut sent_headers = Vec::new();
    for header in headers {
        let value = values.fill(&header.value, Escape::None).await?;
        if value.contains(['\r', '\n']) {
            return Err(format!(
                "the header {} would hold a line break",
                header.name.trim()
            ));
        }
        sent_headers.push((header.name.trim().to_string(), value));
    }
    let line = format!("{} {}", method.as_str(), shown_url(&url));
    if test {
        return Ok((
            format!(
                "Test run: not sent. It would send {line} with a body of {} characters.",
                body.chars().count()
            ),
            given([("status", String::new()), ("answer", String::new())]),
        ));
    }
    // No redirect is followed: the headers are for the host the workflow names, and no other.
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("the request could not be made: {e}"))?;
    let verb = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| format!("the request could not be made: {e}"))?;
    let mut request = client.request(verb, parsed);
    let names_type = sent_headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if !body.is_empty() && !names_type {
        let kind = if looks_like_json(original_body) {
            "application/json"
        } else {
            "text/plain; charset=utf-8"
        };
        request = request.header("Content-Type", kind);
    }
    for (name, value) in sent_headers {
        request = request.header(name, value);
    }
    if !body.is_empty() {
        request = request.body(body);
    }
    let answer = request.send().await.map_err(|e| {
        if e.is_timeout() {
            format!(
                "{line}: no answer from {host} in {} seconds",
                TIMEOUT.as_secs()
            )
        } else {
            // The error's own text can hold the whole URL: only its kind is kept.
            format!("{line}: {host} could not be reached")
        }
    })?;
    let status = answer.status();
    // Hidden before it is cut: a secret the answer repeats across the cut would else keep
    // its first part.
    let text = values.hidden(&answer.text().await.unwrap_or_default());
    let said = clipped(&text, KEPT_BODY);
    if status.is_success() {
        let summary = match said.is_empty() {
            true => format!("{line} answered {}.", status.as_u16()),
            false => format!("{line} answered {}.\n\n{said}", status.as_u16()),
        };
        let outputs = given([
            ("status", status.as_u16().to_string()),
            ("answer", clipped(&text, KEPT_ANSWER)),
        ]);
        Ok((summary, outputs))
    } else {
        Err(match said.is_empty() {
            true => format!("{line} answered {}", status.as_u16()),
            false => format!("{line} answered {}: {said}", status.as_u16()),
        })
    }
}

/// Writes the file of a file action into `folder`, the project's, making the folders on its
/// path. The filled path must still stay inside the folder.
async fn write(
    values: &mut Values<'_>,
    folder: &Path,
    path: &str,
    content: &str,
) -> std::result::Result<Done, String> {
    let path = values.fill(path.trim(), Escape::PathPart).await?;
    let path = path.trim();
    // Git's own folder is never written into: a hook there is code that runs.
    let in_git = path
        .split(['/', '\\'])
        .any(|part| part.trim().eq_ignore_ascii_case(".git"));
    if in_git {
        return Err(format!(
            "{path} is inside git's own folder, which a step does not write to"
        ));
    }
    if !relative_path_ok(path) {
        return Err(format!(
            "the file's path would leave the project's folder: {path}"
        ));
    }
    let content = values.fill(content, Escape::None).await?;
    let file = folder.join(path);
    if let Some(parent) = file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("the folder of {path} could not be made: {e}"))?;
    }
    tokio::fs::write(&file, content.as_bytes())
        .await
        .map_err(|e| format!("{path} could not be written: {e}"))?;
    Ok((
        format!("Wrote {path} ({} characters).", content.chars().count()),
        given([("path", path.to_string())]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_escaped_for_where_it_sits() {
        let said = "He said \"hi\"\nthen left & more";
        assert_eq!(json_escaped(said), "He said \\\"hi\\\"\\nthen left & more");
        assert_eq!(url_escaped("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(url_escaped("plain-Text_1.0~"), "plain-Text_1.0~");
    }

    #[test]
    fn a_body_is_escaped_by_what_it_is() {
        let header = |value: &str| Header {
            name: "content-type".into(),
            value: value.into(),
        };
        assert!(body_escape(&[], " {\"a\": 1}") == Escape::Json);
        assert!(body_escape(&[], "[1]") == Escape::Json);
        assert!(body_escape(&[], "hello") == Escape::None);
        assert!(body_escape(&[header("application/json; charset=utf-8")], "x") == Escape::Json);
        assert!(body_escape(&[header("application/x-www-form-urlencoded")], "a=1") == Escape::Url);
        // A type it names wins over how the body looks.
        assert!(body_escape(&[header("text/plain")], "{not json}") == Escape::None);
    }

    #[test]
    fn a_kept_url_has_no_query() {
        assert_eq!(
            shown_url("https://a.example/x?key=1#f"),
            "https://a.example/x"
        );
        assert_eq!(shown_url("https://a.example/x"), "https://a.example/x");
        assert_eq!(clipped("  abc  ", 10), "abc");
        assert_eq!(clipped("abcdef", 3), "abc…");
    }
}

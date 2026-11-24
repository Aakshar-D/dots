//! Telegram for workflows (connected workflows spec sections 2 and 3): an action step sends a
//! message as a bot, and a workflow whose trigger is `telegram` starts when its bot gets a
//! message from a chat the trigger names.
//!
//! Messages are fetched by asking Telegram for them (`getUpdates`), so nothing outside this PC
//! has to reach it. One loop asks every `POLL`, once per bot: workflows that share a bot (the
//! same Personal variable holds its token) share its messages. Where a bot's messages have
//! been read up to is kept by the variable's name (`telegram_offsets`), written before a run
//! is started, so a message starts its run at most once.
//!
//! A bot's token is a secret: it is read from the Personal variables when it is needed, and no
//! text that is kept or logged holds it.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{StartRequest, Workflows};
use crate::model::{Workflow, WorkflowTrigger};
use crate::server::workflow_webhook::MAX_WEBHOOK_IN_FLIGHT;
use crate::workflow::Trigger;
use crate::{Error, Result};

/// Where Telegram's bot API is, unless a test says otherwise (`set_telegram_base`).
const API: &str = "https://api.telegram.org";
/// How often the bots' messages are asked for.
pub const POLL: Duration = Duration::from_secs(5);
/// How long one call to Telegram may take.
const TIMEOUT: Duration = Duration::from_secs(20);
/// The longest text Telegram takes in one message, in characters.
pub const MAX_TEXT: usize = 4_096;
/// The inputs a run started by a message is given, when its workflow asks for them by these
/// names: the chat's id, who wrote, and the message's id.
pub const CHAT_INPUT: &str = "chat_id";
pub const SENDER_INPUT: &str = "sender";
pub const MESSAGE_INPUT: &str = "message_id";

/// A message a bot got, as far as a workflow needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub update_id: i64,
    pub chat_id: String,
    /// The chat as a person would know it: its title, else who wrote.
    pub chat_name: String,
    pub sender: String,
    pub message_id: String,
    pub text: String,
}

/// A chat a bot has heard from lately, for the trigger's settings to offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenChat {
    pub id: String,
    pub name: String,
}

/// Whether `chat` is a chat's id as Telegram gives it: digits, with a minus for a group.
pub fn chat_id_ok(chat: &str) -> bool {
    let digits = chat.strip_prefix('-').unwrap_or(chat);
    (1..=20).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

/// The messages in `answer`, Telegram's answer to `getUpdates`: each update that is a message
/// with a text. Other updates (edits, joins, photos with no caption) are counted past with
/// their ids but give no message: `Ok((messages, next_offset))`, the offset past every update.
fn incoming(answer: &Value) -> std::result::Result<(Vec<Incoming>, Option<i64>), String> {
    if answer.get("ok").and_then(Value::as_bool) != Some(true) {
        let why = answer
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("it gave no reason");
        return Err(format!("Telegram refused: {why}"));
    }
    let updates = answer
        .get("result")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut next = None;
    for update in updates {
        let Some(id) = update.get("update_id").and_then(Value::as_i64) else {
            continue;
        };
        let past = id.saturating_add(1);
        next = Some(next.map_or(past, |n: i64| n.max(past)));
        let Some(message) = update.get("message") else {
            continue;
        };
        let text = message
            .get("text")
            .or_else(|| message.get("caption"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let Some(chat) = message.pointer("/chat/id").and_then(Value::as_i64) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let name_of = |at: &str| {
            let get = |key: &str| {
                message
                    .pointer(&format!("{at}/{key}"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
            };
            match (
                get("title"),
                get("first_name"),
                get("last_name"),
                get("username"),
            ) {
                (Some(title), ..) => Some(title.to_string()),
                (None, Some(first), Some(last), _) => Some(format!("{first} {last}")),
                (None, Some(first), None, _) => Some(first.to_string()),
                (None, None, _, Some(user)) => Some(format!("@{user}")),
                _ => None,
            }
        };
        let sender = name_of("/from").unwrap_or_else(|| "someone".to_string());
        out.push(Incoming {
            update_id: id,
            chat_id: chat.to_string(),
            chat_name: name_of("/chat").unwrap_or_else(|| sender.clone()),
            sender,
            message_id: message
                .get("message_id")
                .and_then(Value::as_i64)
                .map(|m| m.to_string())
                .unwrap_or_default(),
            text: text.to_string(),
        });
    }
    Ok((out, next))
}

/// One call of the bot `token`'s method `method` at `base`, with `body`; the answer as JSON.
/// The error never holds the token: it says only what failed.
async fn call(
    base: &str,
    token: &str,
    method: &str,
    body: &Value,
) -> std::result::Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "the request could not be made".to_string())?;
    let url = format!("{}/bot{token}/{method}", base.trim_end_matches('/'));
    let answer = client.post(url).json(body).send().await.map_err(|e| {
        if e.is_timeout() {
            "Telegram did not answer in time".to_string()
        } else {
            "Telegram could not be reached".to_string()
        }
    })?;
    let status = answer.status();
    match answer.json::<Value>().await {
        Ok(value) => Ok(value),
        Err(_) => Err(format!("Telegram answered {}", status.as_u16())),
    }
}

/// Sends `text` to the chat `chat` as the bot `token`. `Ok` is Telegram's answer, as JSON text.
pub(super) async fn send_message(
    base: &str,
    token: &str,
    chat: &str,
    text: &str,
) -> std::result::Result<String, String> {
    let chat = chat.trim();
    let text = text.trim();
    if chat.is_empty() {
        return Err("the message names no chat".to_string());
    }
    if text.is_empty() {
        return Err("the message has no text".to_string());
    }
    if text.chars().count() > MAX_TEXT {
        return Err(format!(
            "the message is {} characters, and Telegram takes at most 4,096",
            text.chars().count()
        ));
    }
    // A chat's id is a number to Telegram; a channel's name (`@name`) goes as it is.
    let chat_id = match chat.parse::<i64>() {
        Ok(id) => json!(id),
        Err(_) => json!(chat),
    };
    let answer = call(
        base,
        token,
        "sendMessage",
        &json!({"chat_id": chat_id, "text": text}),
    )
    .await?;
    if answer.get("ok").and_then(Value::as_bool) != Some(true) {
        let why = answer
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("it gave no reason");
        return Err(format!("Telegram refused the message: {why}"));
    }
    Ok(answer.to_string())
}

impl Workflows {
    /// For tests: where the bot API is.
    #[doc(hidden)]
    pub fn set_telegram_base(&self, base: &str) {
        let _ = self.telegram_base.set(base.to_string());
    }

    pub(super) fn telegram_base(&self) -> &str {
        self.telegram_base.get().map_or(API, String::as_str)
    }

    /// The token the Personal variable `name` holds; `Err` says why there is none.
    async fn bot_token(&self, name: &str) -> std::result::Result<String, String> {
        let Some(vault) = self.vault.get() else {
            return Err("the variables cannot be read here".to_string());
        };
        let found = vault
            .personal_values(std::slice::from_ref(&name.to_string()))
            .await
            .map_err(|e| e.to_string())?;
        found
            .get(name)
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| format!("there is no Personal variable named {name}"))
    }

    /// The chats the bot whose token the Personal variable `token` holds has heard from since
    /// its messages were last read, newest first: what the trigger's settings offer to listen
    /// to. Reading them takes none away.
    pub async fn telegram_chats(&self, token: &str) -> Result<Vec<SeenChat>> {
        let secret = self.bot_token(token).await.map_err(Error::Invalid)?;
        let offset = self.store.telegram_offset(token).await?;
        let body = json!({"offset": offset, "timeout": 0, "allowed_updates": ["message"]});
        let answer = call(self.telegram_base(), &secret, "getUpdates", &body)
            .await
            .map_err(Error::Invalid)?;
        let (messages, _) = incoming(&answer).map_err(Error::Invalid)?;
        let mut chats: Vec<SeenChat> = Vec::new();
        for message in messages.into_iter().rev() {
            if !chats.iter().any(|c| c.id == message.chat_id) {
                chats.push(SeenChat {
                    id: message.chat_id,
                    name: message.chat_name,
                });
            }
        }
        Ok(chats)
    }

    /// Runs the loop that asks the bots for their messages until `shutdown`.
    pub fn spawn_telegram(
        self: &std::sync::Arc<Self>,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = this.telegram_pass().await {
                    tracing::warn!("asking Telegram for messages failed: {e}");
                }
                tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(POLL) => {}
                }
            }
        })
    }

    /// One pass over the bots: each bot's new messages, and for each message from a chat one
    /// of its workflows listens to, a run of that workflow. A workflow with as many runs in
    /// flight as a webhook may start leaves the message where it is, for the next pass.
    pub async fn telegram_pass(&self) -> Result<()> {
        // The workflows that listen, by the variable that holds their bot's token.
        let mut bots: BTreeMap<String, Vec<Workflow>> = BTreeMap::new();
        for team in self.store.list_teams().await? {
            // An account's variables are not read here (general workflows spec section 6).
            if team.org_id.is_some() {
                continue;
            }
            for workflow in self.store.list_workflows(&team.id).await? {
                if !workflow.spec.enabled || workflow.problem.is_some() {
                    continue;
                }
                if let Trigger::Telegram { token, .. } = &workflow.spec.definition.trigger {
                    bots.entry(token.clone()).or_default().push(workflow);
                }
            }
        }
        for (token, workflows) in bots {
            if let Err(why) = self.bot_pass(&token, &workflows).await {
                tracing::warn!(bot = %token, "a bot's messages were not read: {why}");
            }
        }
        Ok(())
    }

    /// One bot's part of a pass.
    async fn bot_pass(
        &self,
        token: &str,
        workflows: &[Workflow],
    ) -> std::result::Result<(), String> {
        let secret = self.bot_token(token).await?;
        let offset = self
            .store
            .telegram_offset(token)
            .await
            .map_err(|e| e.to_string())?;
        let body = json!({"offset": offset, "timeout": 0, "allowed_updates": ["message"]});
        let answer = call(self.telegram_base(), &secret, "getUpdates", &body).await?;
        let (messages, next) = incoming(&answer)?;
        for message in &messages {
            let listening: Vec<&Workflow> = workflows
                .iter()
                .filter(|w| {
                    matches!(
                        &w.spec.definition.trigger,
                        Trigger::Telegram { chats, .. } if chats.iter().any(|c| c.trim() == message.chat_id)
                    )
                })
                .collect();
            // A workflow with as many runs in flight as a webhook may start takes no more:
            // the message is not started there, and the bot's other workflows go on.
            let mut open = Vec::new();
            for workflow in listening {
                let in_flight = self
                    .store
                    .count_workflow_runs_in_flight(&workflow.id)
                    .await
                    .map_err(|e| e.to_string())?;
                if in_flight >= MAX_WEBHOOK_IN_FLIGHT {
                    tracing::warn!(
                        workflow = %workflow.spec.name,
                        "a message was not started: the workflow has too many runs in flight"
                    );
                } else {
                    open.push(workflow);
                }
            }
            let listening = open;
            // Read past before its runs start: a message starts its runs at most once.
            self.store
                .set_telegram_offset(token, message.update_id + 1)
                .await
                .map_err(|e| e.to_string())?;
            for workflow in listening {
                if let Err(e) = self.start_from_message(workflow, message).await {
                    tracing::warn!(
                        workflow = %workflow.spec.name,
                        "a message did not start its workflow: {e}"
                    );
                }
            }
        }
        // Past what gave no message, too: an edit, a join, a chat nobody listens to.
        if let Some(next) = next {
            self.store
                .set_telegram_offset(token, next)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Starts `workflow` on `message`: its text is the task, and the inputs the workflow asks
    /// for by the names `chat_id`, `sender` and `message_id` are given.
    async fn start_from_message(&self, workflow: &Workflow, message: &Incoming) -> Result<()> {
        let Trigger::Telegram { project, .. } = &workflow.spec.definition.trigger else {
            return Ok(());
        };
        let project_id = if project.trim().is_empty() {
            None
        } else {
            let team = self.store.get_team(&workflow.team_id).await?;
            match crate::teams::project_named(&team, project) {
                Some(project) => Some(project.id.clone()),
                None => {
                    return Err(Error::NotFound(format!(
                        "the team has no project named \"{}\"",
                        project.trim()
                    )))
                }
            }
        };
        let asks = |name: &str| {
            workflow
                .spec
                .definition
                .inputs
                .iter()
                .any(|f| f.name == name)
        };
        let inputs = [
            (CHAT_INPUT, &message.chat_id),
            (SENDER_INPUT, &message.sender),
            (MESSAGE_INPUT, &message.message_id),
        ]
        .into_iter()
        .filter(|(name, _)| asks(name))
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect();
        // A task is at most 8,000 characters, and a message at most 4,096.
        self.start(StartRequest {
            workflow_id: workflow.id.clone(),
            project_id,
            task: message.text.clone(),
            inputs,
            trigger: WorkflowTrigger::Telegram,
            origin_run_id: None,
            test: false,
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_s_id_is_digits_with_a_minus_for_a_group() {
        for ok in ["1", "123456789", "-1001234567890"] {
            assert!(chat_id_ok(ok), "{ok}");
        }
        for bad in [
            "",
            "-",
            "@channel",
            "12a",
            "1 2",
            "--1",
            "123456789012345678901",
        ] {
            assert!(!chat_id_ok(bad), "{bad}");
        }
    }

    #[test]
    fn the_messages_of_an_answer_are_its_texts_and_every_update_is_read_past() {
        let answer = json!({"ok": true, "result": [
            {"update_id": 10, "message": {"message_id": 5, "text": " Hello ",
                "from": {"id": 7, "first_name": "Asha", "last_name": "Rao"},
                "chat": {"id": 7, "type": "private", "first_name": "Asha"}}},
            {"update_id": 11, "edited_message": {"message_id": 5, "text": "Hello!"}},
            {"update_id": 12, "message": {"message_id": 6, "caption": "A photo",
                "from": {"id": 8, "username": "sam"},
                "chat": {"id": -100123, "type": "supergroup", "title": "Client room"}}},
            {"update_id": 13, "message": {"message_id": 7,
                "from": {"id": 8}, "chat": {"id": -100123, "type": "supergroup"}}}
        ]});
        let (messages, next) = incoming(&answer).unwrap();
        assert_eq!(next, Some(14));
        assert_eq!(
            messages,
            vec![
                Incoming {
                    update_id: 10,
                    chat_id: "7".into(),
                    chat_name: "Asha".into(),
                    sender: "Asha Rao".into(),
                    message_id: "5".into(),
                    text: "Hello".into(),
                },
                Incoming {
                    update_id: 12,
                    chat_id: "-100123".into(),
                    chat_name: "Client room".into(),
                    sender: "@sam".into(),
                    message_id: "6".into(),
                    text: "A photo".into(),
                },
            ]
        );
        assert_eq!(
            incoming(&json!({"ok": true, "result": []})).unwrap(),
            (vec![], None)
        );
        assert_eq!(
            incoming(&json!({"ok": false, "description": "Unauthorized"})).unwrap_err(),
            "Telegram refused: Unauthorized"
        );
    }
}

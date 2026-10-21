//! The `messages` rows: the notes and questions dots post (check-ins spec sections 5 and 7).
//! A run asks at most one question; the insert checks it, so two calls at once cannot both
//! ask. Answering stores the answer's run in the same transaction.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{Message, MessageKind, NewRun, Run};
use crate::util::{new_id, now};
use crate::{Error, Result};

/// The messages `list_messages` gives besides those that wait.
const MESSAGES_SHOWN: i64 = 200;

const MESSAGE_COLUMNS: &str = "m.id AS id, m.dot_id AS dot_id, d.name AS dot_name, \
    m.run_id AS run_id, m.kind AS kind, m.text AS text, m.choices AS choices, \
    m.created_at AS created_at, m.read_at AS read_at, m.answered_at AS answered_at, \
    m.answer_run_id AS answer_run_id, m.dismissed_at AS dismissed_at";

/// A message of `m` that waits for the person (`Message::waits`).
const WAITS: &str = "((m.kind = 'question' AND m.answered_at IS NULL \
    AND m.dismissed_at IS NULL) OR (m.kind = 'note' AND m.read_at IS NULL))";

fn message_from_row(row: &SqliteRow) -> Result<Message> {
    Ok(Message {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        dot_name: row.try_get("dot_name")?,
        run_id: row.try_get("run_id")?,
        kind: MessageKind::parse(&row.try_get::<String, _>("kind")?)?,
        text: row.try_get("text")?,
        choices: row
            .try_get::<Option<String>, _>("choices")?
            .map(|s| serde_json::from_str(&s))
            .transpose()?,
        created_at: row.try_get("created_at")?,
        read_at: row.try_get("read_at")?,
        answered_at: row.try_get("answered_at")?,
        answer_run_id: row.try_get("answer_run_id")?,
        dismissed_at: row.try_get("dismissed_at")?,
    })
}

impl Store {
    /// Stores a note of the dot; `run_id` is `None` for a note of Dots' own.
    pub async fn add_note(
        &self,
        dot_id: &str,
        run_id: Option<&str>,
        text: &str,
    ) -> Result<Message> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO messages (id, dot_id, run_id, kind, text, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(run_id)
        .bind(MessageKind::Note.as_str())
        .bind(text)
        .bind(now())
        .execute(&self.pool)
        .await?;
        self.get_message(&id).await
    }

    /// Stores a note of the run `run_id`; `None` when the run sent `max` notes already. The
    /// count and the insert are one statement, so notes sent at once cannot pass the limit.
    pub async fn add_run_note(
        &self,
        dot_id: &str,
        run_id: &str,
        text: &str,
        max: usize,
    ) -> Result<Option<Message>> {
        let id = new_id();
        let res = sqlx::query(
            "INSERT INTO messages (id, dot_id, run_id, kind, text, created_at)              SELECT ?, ?, ?, 'note', ?, ? WHERE              (SELECT COUNT(*) FROM messages WHERE run_id = ? AND kind = 'note') < ?",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(run_id)
        .bind(text)
        .bind(now())
        .bind(run_id)
        .bind(max as i64)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_message(&id).await.map(Some)
    }

    /// Stores the question of the run `run_id`; `None` when the run asked one already. The
    /// check and the insert are one statement.
    pub async fn add_question(
        &self,
        dot_id: &str,
        run_id: &str,
        text: &str,
        choices: Option<&[String]>,
    ) -> Result<Option<Message>> {
        let id = new_id();
        let choices = match choices {
            Some(list) => Some(serde_json::to_string(list)?),
            None => None,
        };
        let res = sqlx::query(
            "INSERT INTO messages (id, dot_id, run_id, kind, text, choices, created_at) \
             SELECT ?, ?, ?, 'question', ?, ?, ? WHERE NOT EXISTS \
             (SELECT 1 FROM messages WHERE run_id = ? AND kind = 'question')",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(run_id)
        .bind(text)
        .bind(choices)
        .bind(now())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_message(&id).await.map(Some)
    }

    pub async fn get_message(&self, id: &str) -> Result<Message> {
        let row = sqlx::query(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN dots d ON d.id = m.dot_id \
             WHERE m.id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("message {id}")))?;
        message_from_row(&row)
    }

    /// Every message that waits for the person and the `MESSAGES_SHOWN` newest others, of one
    /// dot or of all, newest first.
    pub async fn list_messages(&self, dot_id: Option<&str>) -> Result<Vec<Message>> {
        let rows = sqlx::query(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages m JOIN dots d ON d.id = m.dot_id \
             WHERE (? IS NULL OR m.dot_id = ?) AND ({WAITS} OR m.id IN \
             (SELECT x.id FROM messages x WHERE ? IS NULL OR x.dot_id = ? \
              ORDER BY x.created_at DESC, x.id DESC LIMIT ?)) \
             ORDER BY m.created_at DESC, m.id DESC"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .bind(dot_id)
        .bind(dot_id)
        .bind(MESSAGES_SHOWN)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(message_from_row).collect()
    }

    /// The messages that wait for the person: open questions and unread notes.
    pub async fn count_waiting_messages(&self) -> Result<usize> {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM messages m WHERE {WAITS}"))
            .fetch_one(&self.pool)
            .await?;
        Ok(n as usize)
    }

    /// Marks a message read; a second read keeps the first time. A question stays open.
    pub async fn mark_message_read(&self, id: &str) -> Result<Message> {
        let res = sqlx::query("UPDATE messages SET read_at = COALESCE(read_at, ?) WHERE id = ?")
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("message {id}")));
        }
        self.get_message(id).await
    }

    /// Dismisses an open question; one dismissed already stays as it is. `Conflict` for an
    /// answered question, `Invalid` for a note.
    pub async fn dismiss_question(&self, id: &str) -> Result<Message> {
        let res = sqlx::query(
            "UPDATE messages SET dismissed_at = ? WHERE id = ? AND kind = 'question' \
             AND answered_at IS NULL AND dismissed_at IS NULL",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        let message = self.get_message(id).await?;
        if res.rows_affected() == 0 && message.dismissed_at.is_none() {
            if message.kind == MessageKind::Note {
                return Err(Error::Invalid("only a question can be dismissed".into()));
            }
            if let Some(refusal) = message.refusal() {
                return Err(refusal);
            }
        }
        Ok(message)
    }

    /// Answers the open question `id` with the run `new`, in one transaction: the run is stored
    /// and the question gets `answered_at` and `answer_run_id`. A closed question is refused
    /// (`Message::refusal`) and nothing is written.
    pub async fn answer_question(&self, id: &str, new: &NewRun) -> Result<(Message, Run)> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE messages SET answered_at = ?, answer_run_id = ? WHERE id = ? \
             AND kind = 'question' AND answered_at IS NULL AND dismissed_at IS NULL",
        )
        .bind(now())
        .bind(&run_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            let message = self.get_message(id).await?;
            return Err(message
                .refusal()
                .unwrap_or_else(|| Error::Conflict(format!("message {id} changed meanwhile"))));
        }
        tx.commit().await?;
        Ok((self.get_message(id).await?, self.get_run(&run_id).await?))
    }
}

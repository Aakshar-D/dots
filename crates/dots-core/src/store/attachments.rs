//! `chat_attachments`: files the person attached to chat messages (attachments spec section
//! 4.1). A row is staged until a message takes it: a chat run, linked in the same transaction
//! that inserts the run (`create_chat_run`), or a scheduled message (`hold_for_scheduled`).

use std::collections::HashSet;

use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::chat_files::GONE;
use crate::model::{ChatAttachment, NewRun, Run};
use crate::util::now;
use crate::{Error, Result};

const COLUMNS: &str = "a.id, a.dot_id, a.run_id, a.scheduled_id, a.file_name, a.name, a.size, \
                       a.created_at";

fn attachment_from_row(row: &SqliteRow) -> Result<ChatAttachment> {
    Ok(ChatAttachment {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        run_id: row.try_get("run_id")?,
        scheduled_id: row.try_get("scheduled_id")?,
        file_name: row.try_get("file_name")?,
        name: row.try_get("name")?,
        size: row.try_get::<i64, _>("size")?.max(0) as u64,
        created_at: row.try_get("created_at")?,
    })
}

/// A file the dot wrote or edited, from a `tool_call` event (attachments spec section 4.5).
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenFile {
    pub run_id: String,
    pub root_run_id: String,
    /// The event's time.
    pub at: String,
    /// The path as the tool call named it.
    pub path: String,
    /// The run's workspace.
    pub workspace_path: Option<String>,
}

/// The tools whose calls write or edit a file: Claude's and the local engine's.
pub const WRITING_TOOLS: [&str; 6] = [
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "write_file",
    "edit_file",
];

/// The rule for a topic's files (attachments spec section 4.4): rows linked to a run of the
/// topic, leaving out messages dismissed and never delivered.
const TOPIC_FILES: &str = "FROM chat_attachments a JOIN runs r ON r.id = a.run_id \
     WHERE r.root_run_id = ? AND NOT (r.dismissed_at IS NOT NULL AND r.delivered_at IS NULL)";

impl Store {
    /// Records a staged file (no message yet).
    pub async fn insert_staged_attachment(
        &self,
        id: &str,
        dot_id: &str,
        file_name: &str,
        size: u64,
    ) -> Result<ChatAttachment> {
        sqlx::query(
            "INSERT INTO chat_attachments (id, dot_id, file_name, size, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(dot_id)
        .bind(file_name)
        .bind(size as i64)
        .bind(now())
        .execute(&self.pool)
        .await?;
        self.get_attachment(id).await
    }

    pub async fn get_attachment(&self, id: &str) -> Result<ChatAttachment> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM chat_attachments a WHERE a.id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => attachment_from_row(&row),
            None => Err(Error::NotFound(format!("attachment {id}"))),
        }
    }

    /// Deletes a staged file's row, only while no message took it. `false` when it did (or the
    /// row is gone).
    pub async fn remove_unsent_attachment(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "DELETE FROM chat_attachments WHERE id = ? AND run_id IS NULL \
             AND scheduled_id IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// The files of the topic rooted at `root_run_id`, in the order their messages were queued.
    pub async fn topic_attachments(&self, root_run_id: &str) -> Result<Vec<ChatAttachment>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} {TOPIC_FILES} ORDER BY r.queued_at, r.id, a.id"
        ))
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(attachment_from_row).collect()
    }

    /// The files a run's message carries.
    pub async fn run_attachments(&self, run_id: &str) -> Result<Vec<ChatAttachment>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM chat_attachments a WHERE a.run_id = ? ORDER BY a.id"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(attachment_from_row).collect()
    }

    /// The files a scheduled message waits with.
    pub async fn scheduled_attachments(&self, scheduled_id: &str) -> Result<Vec<ChatAttachment>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM chat_attachments a WHERE a.scheduled_id = ? ORDER BY a.id"
        ))
        .bind(scheduled_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(attachment_from_row).collect()
    }

    /// Deletes every staged row (no chat box survives a restart) and returns their ids.
    pub async fn delete_unsent_attachments(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "DELETE FROM chat_attachments WHERE run_id IS NULL AND scheduled_id IS NULL \
             RETURNING id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Deletes the rows a scheduled message waits with, and returns their ids.
    pub async fn delete_scheduled_attachments(&self, scheduled_id: &str) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "DELETE FROM chat_attachments WHERE scheduled_id = ? AND run_id IS NULL RETURNING id",
        )
        .bind(scheduled_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn attachment_ids_of_dot(&self, dot_id: &str) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT id FROM chat_attachments WHERE dot_id = ?")
                .bind(dot_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn all_attachment_ids(&self) -> Result<HashSet<String>> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM chat_attachments")
            .fetch_all(&self.pool)
            .await?;
        Ok(ids.into_iter().collect())
    }

    /// The names already linked in the topic rooted at `root_run_id`, any message.
    pub async fn topic_attachment_names(&self, root_run_id: &str) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT a.name FROM chat_attachments a JOIN runs r ON r.id = a.run_id \
             WHERE r.root_run_id = ? AND a.name IS NOT NULL ORDER BY r.queued_at, a.id",
        )
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Inserts a chat run and links its files in one transaction (attachments spec section
    /// 4.3), so the dispatcher never claims a message whose files are not linked yet. Each of
    /// `links` (`(id, name)`) must still be unsent, and staged or held by `scheduled_id`;
    /// otherwise nothing is written and the answer is `Conflict`. With `carry_from`, the files
    /// of that run move to the new one (Retry).
    pub async fn create_chat_run(
        &self,
        new: &NewRun,
        links: &[(String, String)],
        scheduled_id: Option<&str>,
        carry_from: Option<&str>,
    ) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        let id = insert_run(&mut tx, new).await?;
        for (attachment, name) in links {
            let res = sqlx::query(
                "UPDATE chat_attachments SET run_id = ?, name = ?, scheduled_id = NULL \
                 WHERE id = ? AND dot_id = ? AND run_id IS NULL \
                 AND (scheduled_id IS NULL OR scheduled_id = ?)",
            )
            .bind(&id)
            .bind(name)
            .bind(attachment)
            .bind(&new.dot_id)
            .bind(scheduled_id)
            .execute(&mut *tx)
            .await?;
            if res.rows_affected() == 0 {
                tx.rollback().await?;
                return Err(Error::Conflict(GONE.into()));
            }
        }
        if let Some(old) = carry_from {
            sqlx::query("UPDATE chat_attachments SET run_id = ? WHERE run_id = ?")
                .bind(&id)
                .bind(old)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_run(&id).await
    }

    /// Holds staged files for the scheduled message `scheduled_id` (attachments spec section
    /// 4.6), inside the caller's transaction. A file that is not a staged file of `dot_id` is
    /// a `Conflict`.
    pub async fn hold_for_scheduled(
        conn: &mut SqliteConnection,
        scheduled_id: &str,
        dot_id: &str,
        ids: &[String],
    ) -> Result<()> {
        for id in ids {
            let res = sqlx::query(
                "UPDATE chat_attachments SET scheduled_id = ? WHERE id = ? AND dot_id = ? \
                 AND run_id IS NULL AND scheduled_id IS NULL",
            )
            .bind(scheduled_id)
            .bind(id)
            .bind(dot_id)
            .execute(&mut *conn)
            .await?;
            if res.rows_affected() == 0 {
                return Err(Error::Conflict(GONE.into()));
            }
        }
        Ok(())
    }

    /// The files the dot wrote or edited in the topics `roots` of `dot_id`, from the
    /// `tool_call` events of their runs (attachments spec section 4.5).
    pub async fn written_files(&self, dot_id: &str, roots: &[String]) -> Result<Vec<WrittenFile>> {
        let tools = serde_json::to_string(&WRITING_TOOLS)?;
        let roots = serde_json::to_string(roots)?;
        let rows = sqlx::query(
            "SELECT r.id AS run_id, r.root_run_id, e.ts, r.workspace_path, \
             COALESCE(json_extract(e.data, '$.input.file_path'), \
                      json_extract(e.data, '$.input.notebook_path')) AS path \
             FROM run_events e JOIN runs r ON r.id = e.run_id \
             WHERE r.dot_id = ? AND r.root_run_id IN (SELECT value FROM json_each(?)) \
             AND e.kind = 'tool_call' \
             AND json_extract(e.data, '$.tool') IN (SELECT value FROM json_each(?)) \
             AND typeof(COALESCE(json_extract(e.data, '$.input.file_path'), \
                      json_extract(e.data, '$.input.notebook_path'))) = 'text' \
             ORDER BY e.ts, e.run_id, e.seq",
        )
        .bind(dot_id)
        .bind(&roots)
        .bind(&tools)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(WrittenFile {
                    run_id: row.try_get("run_id")?,
                    root_run_id: row.try_get("root_run_id")?,
                    at: row.try_get("ts")?,
                    path: row.try_get("path")?,
                    workspace_path: row.try_get("workspace_path")?,
                })
            })
            .collect()
    }
}

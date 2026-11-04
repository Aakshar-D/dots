//! `scheduled_messages`: chat messages the person wrote for Dots to send later (send later
//! spec section 4.1). Every change is one conditional `UPDATE … RETURNING`, so of the sender,
//! Send now, Edit and Cancel only one ever wins a row: the others get `None`.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::model::{ScheduledMessage, ScheduledStatus, ScheduledWait};
use crate::util::{new_id, timestamp};
use crate::{Error, Result};

const COLUMNS: &str = "id, dot_id, root_run_id, text, due_at, status, waiting, error, run_id, \
                       sent_at, created_at, updated_at";

fn message_from_row(row: &SqliteRow) -> Result<ScheduledMessage> {
    let waiting: Option<String> = row.try_get("waiting")?;
    Ok(ScheduledMessage {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        root_run_id: row.try_get("root_run_id")?,
        text: row.try_get("text")?,
        due_at: row.try_get("due_at")?,
        status: ScheduledStatus::parse(&row.try_get::<String, _>("status")?)?,
        waiting: waiting.as_deref().map(ScheduledWait::parse).transpose()?,
        error: row.try_get("error")?,
        run_id: row.try_get("run_id")?,
        sent_at: row.try_get("sent_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Store {
    /// Stores a new message, `scheduled`, due at `due_at` (RFC 3339 UTC, checked by the
    /// caller).
    pub async fn insert_scheduled_message(
        &self,
        dot_id: &str,
        root_run_id: Option<&str>,
        text: &str,
        due_at: &str,
        now: DateTime<Utc>,
    ) -> Result<ScheduledMessage> {
        let id = new_id();
        let ts = timestamp(now);
        sqlx::query(
            "INSERT INTO scheduled_messages (id, dot_id, root_run_id, text, due_at, status, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'scheduled', ?, ?)",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(root_run_id)
        .bind(text)
        .bind(due_at)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await?;
        self.get_scheduled_message(&id).await
    }

    pub async fn get_scheduled_message(&self, id: &str) -> Result<ScheduledMessage> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM scheduled_messages WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => message_from_row(&row),
            None => Err(Error::NotFound(format!("scheduled message {id}"))),
        }
    }

    /// The dot's pending messages (scheduled, sending or failed), in due order.
    pub async fn pending_scheduled_messages(&self, dot_id: &str) -> Result<Vec<ScheduledMessage>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM scheduled_messages WHERE dot_id = ? \
             AND status IN ('scheduled', 'sending', 'failed') ORDER BY due_at, id"
        ))
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(message_from_row).collect()
    }

    /// Every `scheduled` message due at `now` or before, in due order.
    pub async fn due_scheduled_messages(&self, now: &str) -> Result<Vec<ScheduledMessage>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM scheduled_messages WHERE status = 'scheduled' \
             AND due_at <= ? ORDER BY due_at, id"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(message_from_row).collect()
    }

    /// The earliest `due_at` of a `scheduled` message due after `after`.
    pub async fn next_scheduled_due(&self, after: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT MIN(due_at) FROM scheduled_messages WHERE status = 'scheduled' AND due_at > ?",
        )
        .bind(after)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Runs one conditional update and returns the row it changed, if any.
    async fn change_scheduled(
        &self,
        sql: &str,
        binds: &[Option<&str>],
    ) -> Result<Option<ScheduledMessage>> {
        let sql = format!("{sql} RETURNING {COLUMNS}");
        let mut query = sqlx::query(&sql);
        for b in binds {
            query = query.bind(*b);
        }
        match query.fetch_optional(&self.pool).await? {
            Some(row) => Ok(Some(message_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// Takes a `scheduled` message that is due at `now` for sending: `sending`. `None` when it
    /// is not `scheduled`, or no longer due (an Edit moved it later since it was read).
    pub async fn claim_scheduled_message(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET status = 'sending', updated_at = ?1 \
             WHERE id = ?2 AND status = 'scheduled' AND due_at <= ?1",
            &[Some(&ts), Some(id)],
        )
        .await
    }

    /// A due message that cannot go yet: `scheduled` with the reason `wait`. From `sending`
    /// (a send that lost its race) always; from `scheduled` only when it is still due at `now`
    /// (an Edit may have moved it later since the pass read it) and the reason differs, so a
    /// pass that finds the same reason again changes nothing (`None`).
    pub async fn set_scheduled_waiting(
        &self,
        id: &str,
        wait: ScheduledWait,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET status = 'scheduled', waiting = ?1, updated_at = ?2 \
             WHERE id = ?3 AND (status = 'sending' \
             OR (status = 'scheduled' AND due_at <= ?2 AND waiting IS NOT ?1))",
            &[Some(wait.as_str()), Some(&ts), Some(id)],
        )
        .await
    }

    /// A message that was sent as the chat run `run_id`: `sent`, from `sending`.
    pub async fn mark_scheduled_sent(
        &self,
        id: &str,
        run_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET status = 'sent', run_id = ?1, sent_at = ?2, \
             updated_at = ?2, waiting = NULL, error = NULL WHERE id = ?3 AND status = 'sending'",
            &[Some(run_id), Some(&ts), Some(id)],
        )
        .await
    }

    /// A message that cannot be sent: `failed` with `error`, from `sending`, or from `scheduled`
    /// while it is due at `now` (an Edit may have moved it later since the pass read it).
    pub async fn mark_scheduled_failed(
        &self,
        id: &str,
        error: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET status = 'failed', error = ?1, waiting = NULL, \
             updated_at = ?2 WHERE id = ?3 \
             AND (status = 'sending' OR (status = 'scheduled' AND due_at <= ?2))",
            &[Some(error), Some(&ts), Some(id)],
        )
        .await
    }

    /// The person's Edit: new text and time, `scheduled` again with no waiting reason or
    /// error. Only from `scheduled` or `failed`.
    pub async fn edit_scheduled_message(
        &self,
        id: &str,
        text: &str,
        due_at: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET text = ?, due_at = ?, status = 'scheduled', \
             waiting = NULL, error = NULL, updated_at = ? \
             WHERE id = ? AND status IN ('scheduled', 'failed')",
            &[Some(text), Some(due_at), Some(&ts), Some(id)],
        )
        .await
    }

    /// The person's Cancel: `cancelled`, only from `scheduled` or `failed`. The row is kept.
    pub async fn cancel_scheduled_message(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET status = 'cancelled', waiting = NULL, \
             updated_at = ? WHERE id = ? AND status IN ('scheduled', 'failed')",
            &[Some(&ts), Some(id)],
        )
        .await
    }

    /// Send now's first step: due at `now` (so never late), `scheduled`, no error. Only from
    /// `scheduled` or `failed`. The waiting reason is kept: the send that follows decides it.
    pub async fn reset_for_send_now(
        &self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScheduledMessage>> {
        let ts = timestamp(now);
        self.change_scheduled(
            "UPDATE scheduled_messages SET status = 'scheduled', due_at = ?1, error = NULL, \
             updated_at = ?1 WHERE id = ?2 AND status IN ('scheduled', 'failed')",
            &[Some(&ts), Some(id)],
        )
        .await
    }

    /// The ids of the rows being sent.
    async fn sending_ids(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT id FROM scheduled_messages WHERE status = 'sending'")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// A row `sending` whose run is stored (a run carries its id in `send_later.id`: that run,
    /// queued then, is its send) is `sent`, as that run. The sender's pass does this for a
    /// send whose last write failed; the start does it after a crash. Returns the rows changed.
    pub async fn settle_sending_with_runs(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<ScheduledMessage>> {
        let ts = timestamp(now);
        let mut out = Vec::new();
        for id in self.sending_ids().await? {
            let run: Option<(String, String)> = sqlx::query_as(
                "SELECT id, queued_at FROM runs \
                 WHERE json_extract(payload, '$.send_later.id') = ? \
                 ORDER BY queued_at, id LIMIT 1",
            )
            .bind(&id)
            .fetch_optional(&self.pool)
            .await?;
            if let Some((run_id, queued_at)) = run {
                let changed = self
                    .change_scheduled(
                        "UPDATE scheduled_messages SET status = 'sent', run_id = ?1, \
                         sent_at = ?2, updated_at = ?3, waiting = NULL, error = NULL \
                         WHERE id = ?4 AND status = 'sending'",
                        &[Some(&run_id), Some(&queued_at), Some(&ts), Some(&id)],
                    )
                    .await?;
                out.extend(changed);
            }
        }
        Ok(out)
    }

    /// At start (send later spec section 4.5): a row a crash left `sending` is `sent` when its
    /// run is stored (`settle_sending_with_runs`), else `scheduled` again. Only at start: while
    /// Dots runs, a row with no run yet may be on its way through the chat path. Returns the
    /// rows changed.
    pub async fn recover_sending_messages(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<ScheduledMessage>> {
        let ts = timestamp(now);
        let mut out = self.settle_sending_with_runs(now).await?;
        for id in self.sending_ids().await? {
            let changed = self
                .change_scheduled(
                    "UPDATE scheduled_messages SET status = 'scheduled', updated_at = ? \
                     WHERE id = ? AND status = 'sending'",
                    &[Some(&ts), Some(&id)],
                )
                .await?;
            out.extend(changed);
        }
        Ok(out)
    }
}

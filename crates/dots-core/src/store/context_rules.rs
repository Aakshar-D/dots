//! The context meter's numbers, a topic's answer to "start fresh?" and the record of each
//! compaction and fresh start (context rules spec sections 3, 6 and 7).

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::context_rules::{EventKind, Request};
use crate::util::{new_id, now};
use crate::Result;

/// A compaction or fresh start of a run's session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextEvent {
    pub id: String,
    pub run_id: String,
    pub dot_id: String,
    pub root_run_id: String,
    pub kind: EventKind,
    /// `auto` (the rules, or the CLI's own compaction) or `manual` (the person asked).
    pub trigger: String,
    pub fill_before: u64,
    /// `None` for a fresh start until its first turn reports a fill.
    pub fill_after: Option<u64>,
    /// The session a fresh start left: the link from the new session to the old one.
    pub from_session: Option<String>,
    pub at: String,
}

/// A record to add (`Store::add_context_event`).
#[derive(Debug, Clone)]
pub struct NewContextEvent<'a> {
    pub run_id: &'a str,
    pub dot_id: &'a str,
    pub root_run_id: &'a str,
    pub kind: EventKind,
    pub manual: bool,
    pub fill_before: u64,
    pub fill_after: Option<u64>,
    pub from_session: Option<&'a str>,
}

/// The newest fill of a session or topic: the run's fill, the window it reported and its
/// model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastFill {
    pub run_id: String,
    pub fill: u64,
    pub window: Option<u64>,
    pub model: Option<String>,
}

/// A topic's answer to "start fresh?" and the person's request for its next turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThreadState {
    pub keep_going: bool,
    pub request: Option<Request>,
}

fn u(n: i64) -> u64 {
    n.max(0) as u64
}

fn event_from_row(row: &SqliteRow) -> Result<ContextEvent> {
    Ok(ContextEvent {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        dot_id: row.try_get("dot_id")?,
        root_run_id: row.try_get("root_run_id")?,
        kind: EventKind::parse(&row.try_get::<String, _>("kind")?)?,
        trigger: row.try_get("trigger")?,
        fill_before: u(row.try_get("fill_before")?),
        fill_after: row.try_get::<Option<i64>, _>("fill_after")?.map(u),
        from_session: row.try_get("from_session")?,
        at: row.try_get("at")?,
    })
}

fn last_fill(row: &SqliteRow) -> Result<LastFill> {
    Ok(LastFill {
        run_id: row.try_get("id")?,
        fill: u(row.try_get("context_fill")?),
        window: row.try_get::<Option<i64>, _>("context_window")?.map(u),
        model: row.try_get("model")?,
    })
}

impl Store {
    /// Writes what the engine reported of the run's context; a `None` keeps the stored value.
    /// True when something changed.
    pub async fn set_run_context(
        &self,
        run_id: &str,
        fill: Option<u64>,
        window: Option<u64>,
    ) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE runs SET context_fill = COALESCE(?, context_fill), \
             context_window = COALESCE(?, context_window) WHERE id = ? \
             AND (context_fill IS NOT COALESCE(?, context_fill) \
             OR context_window IS NOT COALESCE(?, context_window))",
        )
        .bind(fill.map(|n| n as i64))
        .bind(window.map(|n| n as i64))
        .bind(run_id)
        .bind(fill.map(|n| n as i64))
        .bind(window.map(|n| n as i64))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// The newest run of `session_id` with a fill.
    pub async fn session_fill(&self, session_id: &str) -> Result<Option<LastFill>> {
        let row = sqlx::query(
            "SELECT id, context_fill, context_window, model FROM runs \
             WHERE session_id = ? AND context_fill IS NOT NULL \
             ORDER BY queued_at DESC, id DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(last_fill).transpose()
    }

    /// The newest run of the topic `root_run_id` with a fill.
    pub async fn thread_fill(&self, root_run_id: &str) -> Result<Option<LastFill>> {
        let row = sqlx::query(
            "SELECT id, context_fill, context_window, model FROM runs \
             WHERE root_run_id = ? AND context_fill IS NOT NULL \
             ORDER BY queued_at DESC, id DESC LIMIT 1",
        )
        .bind(root_run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(last_fill).transpose()
    }

    pub async fn thread_state(&self, root_run_id: &str) -> Result<ThreadState> {
        let row =
            sqlx::query("SELECT keep_going, request FROM context_threads WHERE root_run_id = ?")
                .bind(root_run_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Ok(ThreadState::default());
        };
        Ok(ThreadState {
            keep_going: row.try_get::<i64, _>("keep_going")? != 0,
            request: row
                .try_get::<Option<String>, _>("request")?
                .map(|r| Request::parse(&r))
                .transpose()?,
        })
    }

    /// Sets the topic's request for its next turn (`None` clears it).
    pub async fn set_thread_request(
        &self,
        root_run_id: &str,
        request: Option<Request>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO context_threads (root_run_id, request) VALUES (?, ?) \
             ON CONFLICT (root_run_id) DO UPDATE SET request = excluded.request",
        )
        .bind(root_run_id)
        .bind(request.map(|r| r.as_str()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_keep_going(&self, root_run_id: &str, keep_going: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO context_threads (root_run_id, keep_going) VALUES (?, ?) \
             ON CONFLICT (root_run_id) DO UPDATE SET keep_going = excluded.keep_going",
        )
        .bind(root_run_id)
        .bind(keep_going as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Takes the topic's request: returns it and clears it, in one transaction.
    pub async fn take_thread_request(&self, root_run_id: &str) -> Result<Option<Request>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let was: Option<String> =
            sqlx::query_scalar("SELECT request FROM context_threads WHERE root_run_id = ?")
                .bind(root_run_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        if was.is_some() {
            sqlx::query("UPDATE context_threads SET request = NULL WHERE root_run_id = ?")
                .bind(root_run_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        was.map(|r| Request::parse(&r)).transpose()
    }

    pub async fn add_context_event(&self, new: &NewContextEvent<'_>) -> Result<ContextEvent> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO context_events (id, run_id, dot_id, root_run_id, kind, trigger, \
             fill_before, fill_after, from_session, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(new.run_id)
        .bind(new.dot_id)
        .bind(new.root_run_id)
        .bind(new.kind.as_str())
        .bind(if new.manual { "manual" } else { "auto" })
        .bind(new.fill_before as i64)
        .bind(new.fill_after.map(|n| n as i64))
        .bind(new.from_session)
        .bind(now())
        .execute(&self.pool)
        .await?;
        let row = sqlx::query("SELECT * FROM context_events WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        event_from_row(&row)
    }

    /// Fills in the fill after of the run's fresh start, once its first turn reports one.
    pub async fn set_clear_after(&self, run_id: &str, after: u64) -> Result<()> {
        sqlx::query(
            "UPDATE context_events SET fill_after = ?              WHERE run_id = ? AND kind = 'clear' AND fill_after IS NULL",
        )
        .bind(after as i64)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Takes back the topic's "keep going": its fill went under the clear threshold.
    pub async fn clear_keep_going(&self, root_run_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE context_threads SET keep_going = 0 WHERE root_run_id = ? AND keep_going = 1",
        )
        .bind(root_run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The run's compactions and fresh starts, oldest first.
    pub async fn list_context_events(&self, run_id: &str) -> Result<Vec<ContextEvent>> {
        let rows = sqlx::query("SELECT * FROM context_events WHERE run_id = ? ORDER BY at, id")
            .bind(run_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// The dot's compactions and fresh starts, counted.
    pub async fn context_counts(&self, dot_id: &str) -> Result<(u64, u64)> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(kind = 'compact'), 0) AS compacts, \
             COALESCE(SUM(kind = 'clear'), 0) AS clears FROM context_events WHERE dot_id = ?",
        )
        .bind(dot_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((u(row.try_get("compacts")?), u(row.try_get("clears")?)))
    }
}

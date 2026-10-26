//! The `watchers` rows (watchers spec section 3) and `watcher_fires`, the runs each watcher
//! started in its last hour (section 5). A watcher's cursor, where an email or GitHub watcher
//! read up to, is kept beside it and cleared when its config changes.

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::model::{PausedReason, PausedWatcher, Watcher, WatcherConfig, WatcherKind};
use crate::util::{new_id, now, timestamp};
use crate::{Error, Result};

const COLUMNS: &str = "id, dot_id, name, config, enabled, paused_reason, last_error, \
                       last_event_at, created_at, updated_at";

fn watcher_from_row(row: &SqliteRow) -> Result<Watcher> {
    let config: String = row.try_get("config")?;
    let paused: Option<String> = row.try_get("paused_reason")?;
    Ok(Watcher {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        name: row.try_get("name")?,
        config: serde_json::from_str(&config)?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        paused_reason: paused.as_deref().map(PausedReason::parse).transpose()?,
        last_error: row.try_get("last_error")?,
        last_event_at: row.try_get("last_event_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Store {
    /// The dot's watchers, oldest first; none for an unknown dot.
    pub async fn list_watchers(&self, dot_id: &str) -> Result<Vec<Watcher>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM watchers WHERE dot_id = ? ORDER BY created_at, id"
        ))
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(watcher_from_row).collect()
    }

    /// Every dot's watchers, oldest first.
    pub async fn all_watchers(&self) -> Result<Vec<Watcher>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM watchers ORDER BY created_at, id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(watcher_from_row).collect()
    }

    pub async fn get_watcher(&self, id: &str) -> Result<Watcher> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM watchers WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("watcher {id}")))?;
        watcher_from_row(&row)
    }

    pub async fn add_watcher(
        &self,
        dot_id: &str,
        name: &str,
        config: &WatcherConfig,
    ) -> Result<Watcher> {
        self.get_dot(dot_id).await?;
        let id = new_id();
        let at = now();
        sqlx::query(
            "INSERT INTO watchers (id, dot_id, kind, name, config, enabled, created_at, \
             updated_at) VALUES (?, ?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(&id)
        .bind(dot_id)
        .bind(config.kind().as_str())
        .bind(name)
        .bind(serde_json::to_string(config)?)
        .bind(&at)
        .bind(&at)
        .execute(&self.pool)
        .await?;
        self.get_watcher(&id).await
    }

    /// Changes the name and config; a config that differs from the stored one clears the
    /// cursor, so the watcher starts again from now.
    pub async fn update_watcher(
        &self,
        id: &str,
        name: &str,
        config: &WatcherConfig,
    ) -> Result<Watcher> {
        let before = self.get_watcher(id).await?;
        let cursor = if &before.config == config {
            "cursor"
        } else {
            "NULL"
        };
        sqlx::query(&format!(
            "UPDATE watchers SET name = ?, kind = ?, config = ?, cursor = {cursor}, \
             updated_at = ? WHERE id = ?"
        ))
        .bind(name)
        .bind(config.kind().as_str())
        .bind(serde_json::to_string(config)?)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_watcher(id).await
    }

    pub async fn set_watcher_enabled(&self, id: &str, enabled: bool) -> Result<Watcher> {
        let done = sqlx::query("UPDATE watchers SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(enabled as i64)
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("watcher {id}")));
        }
        self.get_watcher(id).await
    }

    pub async fn delete_watcher(&self, id: &str) -> Result<Watcher> {
        let watcher = self.get_watcher(id).await?;
        sqlx::query("DELETE FROM watchers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(watcher)
    }

    /// Dots pauses the watcher; `None` when it was paused by Dots already.
    pub async fn pause_watcher(
        &self,
        id: &str,
        reason: PausedReason,
        error: Option<&str>,
        at: &str,
    ) -> Result<Option<Watcher>> {
        let done = sqlx::query(
            "UPDATE watchers SET paused_reason = ?, paused_at = ?, last_error = ?, \
             updated_at = ? WHERE id = ? AND paused_reason IS NULL",
        )
        .bind(reason.as_str())
        .bind(at)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            self.get_watcher(id).await?;
            return Ok(None);
        }
        self.get_watcher(id).await.map(Some)
    }

    /// The person resumes a watcher Dots paused: its pause, its error and its hour of fires go.
    pub async fn resume_watcher(&self, id: &str) -> Result<Watcher> {
        let mut tx = self.pool.begin().await?;
        let done = sqlx::query(
            "UPDATE watchers SET paused_reason = NULL, paused_at = NULL, last_error = NULL, \
             updated_at = ? WHERE id = ?",
        )
        .bind(now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("watcher {id}")));
        }
        sqlx::query("DELETE FROM watcher_fires WHERE watcher_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_watcher(id).await
    }

    pub async fn set_watcher_error(&self, id: &str, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE watchers SET last_error = ? WHERE id = ?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_watcher_cursor(&self, id: &str) -> Result<Option<Value>> {
        let cursor: Option<String> = sqlx::query_scalar("SELECT cursor FROM watchers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("watcher {id}")))?;
        Ok(cursor.as_deref().map(serde_json::from_str).transpose()?)
    }

    pub async fn set_watcher_cursor(&self, id: &str, cursor: Option<&Value>) -> Result<()> {
        let text = cursor.map(serde_json::to_string).transpose()?;
        sqlx::query("UPDATE watchers SET cursor = ? WHERE id = ?")
            .bind(text)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn touch_watcher_event(&self, id: &str, at: &str) -> Result<()> {
        sqlx::query("UPDATE watchers SET last_event_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The watchers Dots paused, newest pause first, as the Inbox lists them.
    pub async fn paused_watchers(&self) -> Result<Vec<PausedWatcher>> {
        let rows = sqlx::query(
            "SELECT w.id, w.dot_id, d.name AS dot_name, w.kind, w.name, w.paused_reason, \
             w.last_error, w.paused_at FROM watchers w JOIN dots d ON d.id = w.dot_id \
             WHERE w.paused_reason IS NOT NULL ORDER BY w.paused_at DESC, w.id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(PausedWatcher {
                    watcher_id: row.try_get("id")?,
                    dot_id: row.try_get("dot_id")?,
                    dot_name: row.try_get("dot_name")?,
                    kind: WatcherKind::parse(&row.try_get::<String, _>("kind")?)?,
                    name: row.try_get("name")?,
                    reason: PausedReason::parse(&row.try_get::<String, _>("paused_reason")?)?,
                    error: row.try_get("last_error")?,
                    at: row.try_get("paused_at")?,
                })
            })
            .collect()
    }

    /// Takes one of the watcher's `cap` fires in the `window` ending at `at`: true when it was
    /// free (and is now taken). Rows that left the window are dropped in the same transaction.
    pub async fn take_fire(
        &self,
        watcher_id: &str,
        at: DateTime<Utc>,
        window: TimeDelta,
        cap: u32,
    ) -> Result<bool> {
        let since = timestamp(at - window);
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM watcher_fires WHERE watcher_id = ? AND at <= ?")
            .bind(watcher_id)
            .bind(&since)
            .execute(&mut *tx)
            .await?;
        let done = sqlx::query(
            "INSERT INTO watcher_fires (watcher_id, at) SELECT ?, ? WHERE \
             (SELECT COUNT(*) FROM watcher_fires WHERE watcher_id = ? AND at > ?) < ?",
        )
        .bind(watcher_id)
        .bind(timestamp(at))
        .bind(watcher_id)
        .bind(&since)
        .bind(cap as i64)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(done.rows_affected() == 1)
    }

    pub async fn clear_fires(&self, watcher_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM watcher_fires WHERE watcher_id = ?")
            .bind(watcher_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

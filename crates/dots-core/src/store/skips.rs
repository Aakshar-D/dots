//! The `schedule_skips` rows: repeating slots that came due while their dot was busy and were
//! not run (skip when busy spec §4.2). `Store::record_slot` writes them in its own transaction.

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::Store;
use crate::model::ScheduleSkip;
use crate::util::{new_id, timestamp};
use crate::Result;

/// How many skips are kept for a dot; older ones are dropped as new ones come in.
const KEPT_PER_DOT: i64 = 50;

const SKIP_COLUMNS: &str = "id, dot_id, slot_at, reason, run_id, created_at";

fn skip_from_row(row: &SqliteRow) -> Result<ScheduleSkip> {
    Ok(ScheduleSkip {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        slot_at: row.try_get("slot_at")?,
        reason: row.try_get("reason")?,
        run_id: row.try_get("run_id")?,
        created_at: row.try_get("created_at")?,
    })
}

/// A run of the dot that is running or waiting on the person (the newest), if it has one.
pub(super) async fn busy_run_id(
    conn: &mut SqliteConnection,
    dot_id: &str,
) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM runs WHERE dot_id = ? AND status IN ('running', 'awaiting_approval') \
         ORDER BY queued_at DESC, id DESC LIMIT 1",
    )
    .bind(dot_id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// Records the skip of slot `slot_at` for the busy run `run_id` and trims the dot's skips to
/// the newest `KEPT_PER_DOT`.
pub(super) async fn insert_skip(
    conn: &mut SqliteConnection,
    dot_id: &str,
    slot_at: &str,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<ScheduleSkip> {
    let skip = ScheduleSkip {
        id: new_id(),
        dot_id: dot_id.to_string(),
        slot_at: slot_at.to_string(),
        reason: "busy".to_string(),
        run_id: Some(run_id.to_string()),
        created_at: timestamp(now),
    };
    sqlx::query(
        "INSERT INTO schedule_skips (id, dot_id, slot_at, reason, run_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&skip.id)
    .bind(&skip.dot_id)
    .bind(&skip.slot_at)
    .bind(&skip.reason)
    .bind(&skip.run_id)
    .bind(&skip.created_at)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM schedule_skips WHERE dot_id = ? AND rowid NOT IN ( \
           SELECT rowid FROM schedule_skips WHERE dot_id = ? \
           ORDER BY slot_at DESC, rowid DESC LIMIT ?)",
    )
    .bind(dot_id)
    .bind(dot_id)
    .bind(KEPT_PER_DOT)
    .execute(&mut *conn)
    .await?;
    Ok(skip)
}

impl Store {
    /// Whether the dot has a run that is running or waiting for an approval. A queued run does
    /// not make it busy: a slot joins that one.
    pub async fn dot_is_busy(&self, dot_id: &str) -> Result<bool> {
        let mut conn = self.pool.acquire().await?;
        Ok(busy_run_id(&mut conn, dot_id).await?.is_some())
    }

    /// The dot's kept skips, newest slot first.
    pub async fn schedule_skips(&self, dot_id: &str, limit: u32) -> Result<Vec<ScheduleSkip>> {
        let rows = sqlx::query(&format!(
            "SELECT {SKIP_COLUMNS} FROM schedule_skips WHERE dot_id = ? \
             ORDER BY slot_at DESC, rowid DESC LIMIT ?"
        ))
        .bind(dot_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(skip_from_row).collect()
    }

    /// How many of the dot's kept skips have a slot after `after` (all of them for `None`).
    pub async fn skips_since(&self, dot_id: &str, after: Option<&str>) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM schedule_skips WHERE dot_id = ? AND (? IS NULL OR slot_at > ?)",
        )
        .bind(dot_id)
        .bind(after)
        .bind(after)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
    }
}

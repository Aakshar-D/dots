//! `inbox_marks`: the person's snooze and done marks on Inbox items (snooze and done spec
//! section 4.2). A mark is always in exactly one state; the table's check holds it.

use std::collections::HashSet;

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::inbox::{InboxKey, InboxKind, InboxMark, MarkState};
use crate::Result;

fn mark_from_row(row: &SqliteRow) -> Result<InboxMark> {
    Ok(InboxMark {
        kind: InboxKind::parse(&row.try_get::<String, _>("kind")?)?,
        id: row.try_get("item_id")?,
        title: String::new(),
        dot_id: None,
        snoozed_until: row.try_get("snoozed_until")?,
        done_at: row.try_get("done_at")?,
        back_at: row.try_get("back_at")?,
    })
}

impl Store {
    /// Snoozes every item of `keys` until `until`, in one transaction. A done or back mark of
    /// the item becomes the snooze.
    pub async fn snooze_marks(&self, keys: &[InboxKey], until: &str, now: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for key in keys {
            sqlx::query(
                "INSERT INTO inbox_marks (kind, item_id, snoozed_until, marked_at) \
                 VALUES (?, ?, ?, ?) ON CONFLICT(kind, item_id) DO UPDATE SET \
                 snoozed_until = excluded.snoozed_until, done_at = NULL, back_at = NULL, \
                 marked_at = excluded.marked_at",
            )
            .bind(key.kind.as_str())
            .bind(&key.id)
            .bind(until)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Marks the item done at `now`. A snooze or back mark of it becomes the done mark.
    pub async fn done_mark(&self, key: &InboxKey, now: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO inbox_marks (kind, item_id, done_at, marked_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(kind, item_id) DO UPDATE SET done_at = excluded.done_at, \
             snoozed_until = NULL, back_at = NULL, marked_at = excluded.marked_at",
        )
        .bind(key.kind.as_str())
        .bind(&key.id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Deletes the item's mark only if it is in `state`; true when one was deleted.
    pub async fn delete_mark_if(&self, key: &InboxKey, state: MarkState) -> Result<bool> {
        let column = match state {
            MarkState::Snoozed => "snoozed_until",
            MarkState::Done => "done_at",
        };
        let res = sqlx::query(&format!(
            "DELETE FROM inbox_marks WHERE kind = ? AND item_id = ? AND {column} IS NOT NULL"
        ))
        .bind(key.kind.as_str())
        .bind(&key.id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Every mark, by kind and id.
    pub async fn list_marks(&self) -> Result<Vec<InboxMark>> {
        let rows = sqlx::query(
            "SELECT kind, item_id, snoozed_until, done_at, back_at FROM inbox_marks \
             ORDER BY kind, item_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(mark_from_row).collect()
    }

    pub async fn count_marks(&self) -> Result<usize> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM inbox_marks")
            .fetch_one(&self.pool)
            .await?;
        Ok(n as usize)
    }

    /// Deletes the marks whose item is not in `keep` (it left the Inbox on its own), and
    /// returns how many went.
    pub async fn prune_marks(&self, keep: &HashSet<InboxKey>) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query("SELECT kind, item_id FROM inbox_marks")
            .fetch_all(&mut *tx)
            .await?;
        let mut gone = 0;
        for row in &rows {
            let kind: String = row.try_get("kind")?;
            let id: String = row.try_get("item_id")?;
            let kept = InboxKind::parse(&kind)
                .map(|kind| keep.contains(&InboxKey::new(kind, id.clone())))
                .unwrap_or(false);
            if !kept {
                gone += sqlx::query("DELETE FROM inbox_marks WHERE kind = ? AND item_id = ?")
                    .bind(&kind)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected() as usize;
            }
        }
        tx.commit().await?;
        Ok(gone)
    }

    /// Wakes every snooze due by `now`: it becomes a back mark at `now`. Returns the keys woken.
    pub async fn wake_due(&self, now: &str) -> Result<Vec<InboxKey>> {
        let rows = sqlx::query(
            "UPDATE inbox_marks SET back_at = ?, snoozed_until = NULL \
             WHERE snoozed_until IS NOT NULL AND snoozed_until <= ? RETURNING kind, item_id",
        )
        .bind(now)
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(InboxKey::new(
                    InboxKind::parse(&row.try_get::<String, _>("kind")?)?,
                    row.try_get::<String, _>("item_id")?,
                ))
            })
            .collect()
    }
}

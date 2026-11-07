//! The audit queue and History (env vault spec §10). `audit_chain` holds each org's last `seq`
//! and the hash of its last line, `audit_outbox` the lines not yet sent, and `audit_history` what
//! the account's History lists on this PC. None of them holds a value.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use super::Store;
use crate::vault::audit::record::REASON_QUEUE_FULL;
use crate::vault::audit::{line_hash, AuditRecord, Dropped, Event};
use crate::{Error, Result};

str_enum!(HistoryState {
    Local => "local",
    Queued => "queued",
    Sent => "sent",
    Dropped => "dropped",
});

/// How much is kept (env vault spec §10, and the person's answers of 2026-11-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditLimits {
    /// The most records queued per org. Reaching it drops the oldest `drop_chunk`.
    pub max_queued: u64,
    pub drop_chunk: u64,
    /// The most History rows kept per org, and for how many days.
    pub history_rows: u64,
    pub history_days: i64,
}

impl Default for AuditLimits {
    fn default() -> Self {
        AuditLimits {
            max_queued: 100_000,
            drop_chunk: 1_000,
            history_rows: 1_000,
            history_days: 30,
        }
    }
}

/// One row of an account's History. It never holds a value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryRow {
    pub id: String,
    /// RFC 3339, to the second.
    pub at: String,
    pub event: Event,
    pub name: Option<String>,
    pub consumer: Option<String>,
    pub dot_name: Option<String>,
    pub reason: Option<String>,
    /// `local` when the account sends no audit records, else `queued`, `sent` or `dropped`.
    pub state: HistoryState,
}

/// A record to keep: `queue` puts it in the org's chain and queue (the policy has `audit`),
/// `history` lists it in History (every event but `fetched`).
#[derive(Debug, Clone)]
pub struct NewAudit {
    pub record: AuditRecord,
    pub queue: bool,
    pub history: bool,
}

fn history_from_row(row: &SqliteRow) -> Result<HistoryRow> {
    Ok(HistoryRow {
        id: row.try_get("id")?,
        at: row.try_get("at")?,
        event: Event::parse(&row.try_get::<String, _>("event")?)?,
        name: row.try_get("name")?,
        consumer: row.try_get("consumer")?,
        dot_name: row.try_get("dot_name")?,
        reason: row.try_get("reason")?,
        state: HistoryState::parse(&row.try_get::<String, _>("state")?)?,
    })
}

fn seq_of(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Other(format!("audit seq {value} is not valid")))
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Other(format!("audit seq {value} is too large")))
}

impl Store {
    /// Keeps `new`, under `limits`, and gives the records it wrote with their `seq` and `prev`:
    /// the record, after an `expired` one when the org's queue was full and its oldest
    /// `drop_chunk` records were dropped. A record that is not queued keeps `seq` 0 and no
    /// `prev`. History rows older than `history_days` before the record, or past the newest
    /// `history_rows`, are deleted.
    pub async fn append_audit(
        &self,
        new: NewAudit,
        limits: &AuditLimits,
    ) -> Result<Vec<AuditRecord>> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let org_id = new.record.org_id.clone();
        let mut written = Vec::new();
        if new.queue {
            let chain = sqlx::query("SELECT seq, last_hash FROM audit_chain WHERE org_id = ?")
                .bind(&org_id)
                .fetch_optional(&mut *tx)
                .await?;
            let (mut seq, mut prev) = match chain {
                Some(row) => (
                    seq_of(row.try_get("seq")?)?,
                    Some(row.try_get("last_hash")?),
                ),
                None => (0, None),
            };
            let oldest: Option<i64> =
                sqlx::query_scalar("SELECT MIN(seq) FROM audit_outbox WHERE org_id = ?")
                    .bind(&org_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if let Some(oldest) = oldest.map(seq_of).transpose()? {
                if seq + 1 - oldest >= limits.max_queued {
                    let to = (oldest + limits.drop_chunk - 1).min(seq);
                    drop_queued(&mut tx, &org_id, oldest, to).await?;
                    let expired = AuditRecord {
                        id: crate::util::new_id(),
                        dot: None,
                        run_id: None,
                        event: Event::Expired,
                        name: None,
                        consumer: None,
                        reason: Some(REASON_QUEUE_FULL.into()),
                        dropped: Some(Dropped { from: oldest, to }),
                        ..new.record.clone()
                    };
                    let line = queue(&mut tx, expired.clone(), &mut seq, &mut prev).await?;
                    insert_history(&mut tx, &line, HistoryState::Queued).await?;
                    written.push(line);
                }
            }
            let line = queue(&mut tx, new.record, &mut seq, &mut prev).await?;
            if new.history {
                insert_history(&mut tx, &line, HistoryState::Queued).await?;
            }
            written.push(line);
        } else {
            if new.history {
                insert_history(&mut tx, &new.record, HistoryState::Local).await?;
            }
            written.push(new.record);
        }
        if let Some(last) = written.last() {
            prune_history(&mut tx, &org_id, &last.at, limits).await?;
        }
        tx.commit().await?;
        Ok(written)
    }

    /// The org's oldest queued lines, at most `limit`, with their `seq`, oldest first.
    pub async fn audit_batch(&self, org_id: &str, limit: u64) -> Result<Vec<(u64, String)>> {
        let rows =
            sqlx::query("SELECT seq, line FROM audit_outbox WHERE org_id = ? ORDER BY seq LIMIT ?")
                .bind(org_id)
                .bind(as_i64(limit)?)
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(|row| Ok((seq_of(row.try_get("seq")?)?, row.try_get("line")?)))
            .collect()
    }

    /// The receiver took the org's records up to `through`: they leave the queue, and their
    /// History rows are sent.
    pub async fn audit_sent(&self, org_id: &str, through: u64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM audit_outbox WHERE org_id = ? AND seq <= ?")
            .bind(org_id)
            .bind(as_i64(through)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE audit_history SET state = 'sent' \
             WHERE org_id = ? AND seq <= ? AND state = 'queued'",
        )
        .bind(org_id)
        .bind(as_i64(through)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every org with queued records, and how many, by org id.
    pub async fn audit_waiting(&self) -> Result<Vec<(String, u64)>> {
        let rows = sqlx::query(
            "SELECT org_id, COUNT(*) AS n FROM audit_outbox GROUP BY org_id ORDER BY org_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("org_id")?, seq_of(row.try_get("n")?)?)))
            .collect()
    }

    /// How many of the org's records wait to be sent.
    pub async fn audit_queued(&self, org_id: &str) -> Result<u64> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_outbox WHERE org_id = ?")
            .bind(org_id)
            .fetch_one(&self.pool)
            .await?;
        seq_of(n)
    }

    /// The org's History, newest first, at most `limit` rows.
    pub async fn audit_history(&self, org_id: &str, limit: u64) -> Result<Vec<HistoryRow>> {
        let rows = sqlx::query(
            "SELECT id, at, event, name, consumer, dot_name, reason, state FROM audit_history \
             WHERE org_id = ? ORDER BY n DESC LIMIT ?",
        )
        .bind(org_id)
        .bind(as_i64(limit)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(history_from_row).collect()
    }

    /// The org's History as of `now`, newest first: the rows of the last `history_days` days
    /// (the cutoff `append_audit` prunes by), at most `history_rows` of them.
    pub async fn audit_history_as_of(
        &self,
        org_id: &str,
        now: DateTime<Utc>,
        limits: &AuditLimits,
    ) -> Result<Vec<HistoryRow>> {
        let rows = sqlx::query(
            "SELECT id, at, event, name, consumer, dot_name, reason, state FROM audit_history              WHERE org_id = ? AND at >= ? ORDER BY n DESC LIMIT ?",
        )
        .bind(org_id)
        .bind(history_cutoff(now, limits))
        .bind(as_i64(limits.history_rows)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(history_from_row).collect()
    }
}

/// The oldest `at` History keeps: `history_days` before `now`.
fn history_cutoff(now: DateTime<Utc>, limits: &AuditLimits) -> String {
    crate::vault::audit::record::stamp(now - Duration::days(limits.history_days))
}

/// Gives `record` the next `seq` and the `prev` after `seq` and `prev`, queues its line and
/// moves the chain on.
async fn queue(
    tx: &mut Transaction<'_, Sqlite>,
    record: AuditRecord,
    seq: &mut u64,
    prev: &mut Option<String>,
) -> Result<AuditRecord> {
    let record = AuditRecord {
        seq: *seq + 1,
        prev: prev.clone(),
        ..record
    };
    let line = record.line();
    let hash = line_hash(&line);
    sqlx::query("INSERT INTO audit_outbox (org_id, seq, line) VALUES (?, ?, ?)")
        .bind(&record.org_id)
        .bind(as_i64(record.seq)?)
        .bind(&line)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO audit_chain (org_id, seq, last_hash) VALUES (?, ?, ?) \
         ON CONFLICT (org_id) DO UPDATE SET seq = excluded.seq, last_hash = excluded.last_hash",
    )
    .bind(&record.org_id)
    .bind(as_i64(record.seq)?)
    .bind(&hash)
    .execute(&mut **tx)
    .await?;
    *seq = record.seq;
    *prev = Some(hash);
    Ok(record)
}

/// Deletes the org's queued records `from` to `to`, unsent; their History rows are dropped.
async fn drop_queued(
    tx: &mut Transaction<'_, Sqlite>,
    org_id: &str,
    from: u64,
    to: u64,
) -> Result<()> {
    sqlx::query("DELETE FROM audit_outbox WHERE org_id = ? AND seq BETWEEN ? AND ?")
        .bind(org_id)
        .bind(as_i64(from)?)
        .bind(as_i64(to)?)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE audit_history SET state = 'dropped' \
         WHERE org_id = ? AND seq BETWEEN ? AND ? AND state = 'queued'",
    )
    .bind(org_id)
    .bind(as_i64(from)?)
    .bind(as_i64(to)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_history(
    tx: &mut Transaction<'_, Sqlite>,
    record: &AuditRecord,
    state: HistoryState,
) -> Result<()> {
    let seq = match state {
        HistoryState::Local => None,
        _ => Some(as_i64(record.seq)?),
    };
    sqlx::query(
        "INSERT INTO audit_history \
         (id, org_id, seq, at, event, name, consumer, dot_name, reason, state) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&record.id)
    .bind(&record.org_id)
    .bind(seq)
    .bind(&record.at)
    .bind(record.event.as_str())
    .bind(&record.name)
    .bind(&record.consumer)
    .bind(record.dot.as_ref().map(|d| d.name.as_str()))
    .bind(&record.reason)
    .bind(state.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the org's History rows older than `history_days` before `now`, then all but the
/// newest `history_rows`.
async fn prune_history(
    tx: &mut Transaction<'_, Sqlite>,
    org_id: &str,
    now: &str,
    limits: &AuditLimits,
) -> Result<()> {
    let now = DateTime::parse_from_rfc3339(now)
        .map_err(|e| Error::Other(format!("an audit record's time is not valid: {e}")))?
        .with_timezone(&Utc);
    sqlx::query("DELETE FROM audit_history WHERE org_id = ? AND at < ?")
        .bind(org_id)
        .bind(history_cutoff(now, limits))
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "DELETE FROM audit_history WHERE org_id = ?1 AND n < \
         (SELECT n FROM audit_history WHERE org_id = ?1 ORDER BY n DESC LIMIT 1 OFFSET ?2)",
    )
    .bind(org_id)
    .bind(as_i64(limits.history_rows.saturating_sub(1))?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

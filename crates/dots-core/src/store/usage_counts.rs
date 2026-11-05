//! The numbers an hourly usage counts report carries (usage counts spec): counts by time range,
//! never a name, a path or anything a run holds.

use chrono::{DateTime, TimeDelta, Utc};

use super::Store;
use crate::util::timestamp;
use crate::Result;

/// One UTC hour's counts, and the number of dots now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HourCounts {
    /// Runs whose engine started in the hour (`started_at`).
    pub runs_started: u64,
    /// Runs that ended in the hour and succeeded (`ended_at`).
    pub runs_succeeded: u64,
    /// Runs that ended in the hour and failed (`ended_at`).
    pub runs_failed: u64,
    /// Chat messages the person sent to a dot in the hour: chat runs queued in it.
    pub messages_sent: u64,
    /// Approvals asked in the hour (`approvals.created_at`).
    pub approvals_asked: u64,
    /// Dots there are when the counts are taken (the store keeps no history of them).
    pub dots: u64,
}

impl Store {
    /// The counts of the hour starting at `hour_start`: from it, up to the next hour's start.
    pub async fn hour_counts(&self, hour_start: DateTime<Utc>) -> Result<HourCounts> {
        let start = timestamp(hour_start);
        let end = timestamp(hour_start + TimeDelta::hours(1));
        let (started, succeeded, failed, messages): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
             COALESCE(SUM(started_at >= ?1 AND started_at < ?2), 0), \
             COALESCE(SUM(status = 'succeeded' AND ended_at >= ?1 AND ended_at < ?2), 0), \
             COALESCE(SUM(status = 'failed' AND ended_at >= ?1 AND ended_at < ?2), 0), \
             COALESCE(SUM(trigger_kind = 'chat' AND queued_at >= ?1 AND queued_at < ?2), 0) \
             FROM runs",
        )
        .bind(&start)
        .bind(&end)
        .fetch_one(&self.pool)
        .await?;
        let approvals: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM approvals WHERE created_at >= ? AND created_at < ?",
        )
        .bind(&start)
        .bind(&end)
        .fetch_one(&self.pool)
        .await?;
        let dots: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dots")
            .fetch_one(&self.pool)
            .await?;
        let n = |v: i64| u64::try_from(v).unwrap_or(0);
        Ok(HourCounts {
            runs_started: n(started),
            runs_succeeded: n(succeeded),
            runs_failed: n(failed),
            messages_sent: n(messages),
            approvals_asked: n(approvals),
            dots: n(dots),
        })
    }

    /// Sets an approval's `created_at`, for tests of the usage counts.
    #[doc(hidden)]
    pub async fn set_approval_created_at(&self, id: &str, at: &str) -> Result<()> {
        sqlx::query("UPDATE approvals SET created_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

//! The counts a problem report carries (report a problem spec §2): numbers and engine kinds
//! only, never a dot's name or anything it holds.

use chrono::{DateTime, Utc};

use super::Store;
use crate::util::timestamp;
use crate::Result;

/// What `Store::report_counts` gives.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportCounts {
    /// The engine kinds the dots use, each once, sorted.
    pub engines: Vec<String>,
    pub dots: u32,
    /// Runs queued at or after `since`, and how many of them failed.
    pub runs: u32,
    pub failed: u32,
}

impl Store {
    /// The engine kinds in use, the number of dots, and the runs queued since `since`.
    pub async fn report_counts(&self, since: DateTime<Utc>) -> Result<ReportCounts> {
        let engines: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT engine FROM dots ORDER BY engine")
                .fetch_all(&self.pool)
                .await?;
        let dots: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dots")
            .fetch_one(&self.pool)
            .await?;
        let (runs, failed): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(status = 'failed'), 0) FROM runs WHERE queued_at >= ?",
        )
        .bind(timestamp(since))
        .fetch_one(&self.pool)
        .await?;
        let count = |n: i64| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(ReportCounts {
            engines,
            dots: count(dots),
            runs: count(runs),
            failed: count(failed),
        })
    }
}

//! What Open Dot's requests to the model used: one row per request, counted toward the caps for
//! every dot together (`spend_between(None, ..)`) and shown on the Usage page.

use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::Store;
use crate::engine::claude::once::OnceUsage;
use crate::spend::Spend;
use crate::util::{new_id, now};
use crate::Result;

/// Open Dot's requests in a period, summed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenDotUsage {
    pub requests: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// The estimated API cost the CLI reported; a request that reported none adds nothing.
    pub cost_usd: f64,
}

/// A count as SQLite stores it: at most `i64::MAX`.
fn stored(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Store {
    /// Records one request of `model` that used `u`, now.
    pub async fn add_open_dot_usage(&self, model: &str, u: &OnceUsage) -> Result<()> {
        sqlx::query(
            "INSERT INTO open_dot_usage \
             (id, at, model, tokens_in, tokens_out, cache_read, cost_usd) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(now())
        .bind(model)
        .bind(stored(u.tokens_in))
        .bind(stored(u.tokens_out))
        .bind(stored(u.cache_read))
        .bind(u.cost_usd)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// What Open Dot's requests in `[start, end)` spent: cost and tokens in + out.
    pub async fn open_dot_spend_between(&self, start: &str, end: &str) -> Result<Spend> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(cost_usd), 0.0) AS cost, \
             COALESCE(SUM(tokens_in + tokens_out), 0) AS tokens \
             FROM open_dot_usage WHERE at >= ? AND at < ?",
        )
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await?;
        Ok(Spend {
            cost_usd: row.try_get("cost")?,
            tokens: row.try_get("tokens")?,
        })
    }

    /// Open Dot's requests since `since` (all of them for `None`); `None` when there are none.
    pub async fn open_dot_usage_since(&self, since: Option<&str>) -> Result<Option<OpenDotUsage>> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS requests, \
             COALESCE(SUM(tokens_in), 0) AS tokens_in, \
             COALESCE(SUM(tokens_out), 0) AS tokens_out, \
             COALESCE(SUM(cost_usd), 0.0) AS cost \
             FROM open_dot_usage WHERE (? IS NULL OR at >= ?)",
        )
        .bind(since)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        let requests: i64 = row.try_get("requests")?;
        if requests == 0 {
            return Ok(None);
        }
        Ok(Some(OpenDotUsage {
            requests,
            tokens_in: row.try_get("tokens_in")?,
            tokens_out: row.try_get("tokens_out")?,
            cost_usd: row.try_get("cost")?,
        }))
    }
}

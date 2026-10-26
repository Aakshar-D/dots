//! The latest reading of each Claude limit window (usage and limits spec section 3), kept so
//! the meter survives a restart.

use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::Store;
use crate::Result;

/// One window as the `claude` CLI last reported it: `utilization` is the share used (0 to 1,
/// stored as read), `resets_at` and `read_at` are RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitReading {
    pub window: String,
    pub utilization: f64,
    pub resets_at: String,
    pub status: String,
    pub read_at: String,
}

impl Store {
    /// Stores each reading, replacing the window's earlier one.
    pub async fn save_limit_readings(&self, readings: &[LimitReading]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for r in readings {
            sqlx::query(
                "INSERT INTO claude_limits (window, utilization, resets_at, status, read_at) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT(window) DO UPDATE SET \
                 utilization = excluded.utilization, resets_at = excluded.resets_at, \
                 status = excluded.status, read_at = excluded.read_at",
            )
            .bind(&r.window)
            .bind(r.utilization)
            .bind(&r.resets_at)
            .bind(&r.status)
            .bind(&r.read_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every stored reading, by window name.
    pub async fn limit_readings(&self) -> Result<Vec<LimitReading>> {
        let rows = sqlx::query(
            "SELECT window, utilization, resets_at, status, read_at FROM claude_limits \
             ORDER BY window",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(LimitReading {
                    window: row.try_get("window")?,
                    utilization: row.try_get("utilization")?,
                    resets_at: row.try_get("resets_at")?,
                    status: row.try_get("status")?,
                    read_at: row.try_get("read_at")?,
                })
            })
            .collect()
    }
}

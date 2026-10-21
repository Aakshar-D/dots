//! The store's part of browser use (browser spec section 3.6).

use super::Store;
use crate::Result;

impl Store {
    /// Records that an approved browser call is not performed: it was approved after its run
    /// parked, and the page may have moved on. Its `apply_state` becomes `skipped`. False when
    /// the call is not approved, or already has another outcome.
    pub async fn skip_late_browser_call(&self, approval_id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE approvals SET apply_state = 'skipped' WHERE id = ? AND status = 'approved' \
             AND (apply_state IS NULL OR apply_state = 'queued')",
        )
        .bind(approval_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }
}

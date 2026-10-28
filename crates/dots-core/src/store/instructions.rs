//! The instructions dots propose for themselves (conversational chat spec section 4).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::instructions::{InstructionProposal, InstructionStatus, DECIDED_ALREADY};
use crate::model::{NewRun, Run};
use crate::{Error, Result};

const COLUMNS: &str = "id, dot_id, run_id, root_run_id, base, instructions, reason, status, \
    answer_run_id, created_at, decided_at";

fn proposal_from_row(row: &SqliteRow) -> Result<InstructionProposal> {
    Ok(InstructionProposal {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        run_id: row.try_get("run_id")?,
        root_run_id: row.try_get("root_run_id")?,
        base: row.try_get("base")?,
        instructions: row.try_get("instructions")?,
        reason: row.try_get("reason")?,
        status: InstructionStatus::parse(&row.try_get::<String, _>("status")?)?,
        answer_run_id: row.try_get("answer_run_id")?,
        created_at: row.try_get("created_at")?,
        decided_at: row.try_get("decided_at")?,
    })
}

impl Store {
    /// Keeps the proposal `p` in place of the dot's waiting one, which becomes `replaced`, in
    /// one transaction. Returns the ids it replaced.
    pub async fn add_instruction_proposal(&self, p: &InstructionProposal) -> Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;
        // A write first: a transaction that read first could not take the write lock while
        // another connection writes, and would fail as "database is locked".
        let replaced: Vec<String> = sqlx::query_scalar(
            "UPDATE instruction_proposals SET status = 'replaced', decided_at = ? \
             WHERE dot_id = ? AND status = 'pending' RETURNING id",
        )
        .bind(crate::util::now())
        .bind(&p.dot_id)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "INSERT INTO instruction_proposals ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&p.id)
        .bind(&p.dot_id)
        .bind(&p.run_id)
        .bind(&p.root_run_id)
        .bind(&p.base)
        .bind(&p.instructions)
        .bind(&p.reason)
        .bind(p.status.as_str())
        .bind(&p.answer_run_id)
        .bind(&p.created_at)
        .bind(&p.decided_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(replaced)
    }

    pub async fn get_instruction_proposal(&self, id: &str) -> Result<InstructionProposal> {
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM instruction_proposals WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("instruction proposal {id}")))?;
        proposal_from_row(&row)
    }

    /// The proposals of one dot or of all, newest first.
    pub async fn list_instruction_proposals(
        &self,
        dot_id: Option<&str>,
    ) -> Result<Vec<InstructionProposal>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM instruction_proposals WHERE ? IS NULL OR dot_id = ? \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(proposal_from_row).collect()
    }

    /// Decides the pending proposal `id` and stores the run that continues the dot's chat, in
    /// one transaction: a second decision is refused.
    pub async fn decide_instruction_proposal(
        &self,
        id: &str,
        status: InstructionStatus,
        new: &NewRun,
    ) -> Result<(InstructionProposal, Run)> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE instruction_proposals SET status = ?, answer_run_id = ?, decided_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(&run_id)
        .bind(crate::util::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            self.get_instruction_proposal(id).await?;
            return Err(Error::Conflict(DECIDED_ALREADY.to_string()));
        }
        tx.commit().await?;
        Ok((
            self.get_instruction_proposal(id).await?,
            self.get_run(&run_id).await?,
        ))
    }
}

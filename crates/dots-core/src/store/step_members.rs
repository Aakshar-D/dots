//! The members a delegate step's run started (teams spec section 5.3): a row per member
//! thread, added with the thread's first run and marked reported with the run that tells the
//! delegating run how it ended.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::model::{NewRun, Run, StepMember};
use crate::util::now;
use crate::{Error, Result};

const COLUMNS: &str = "run_id, step_run_id, dot_id, task, reported, created_at";

fn member_from_row(row: &SqliteRow) -> Result<StepMember> {
    Ok(StepMember {
        run_id: row.try_get("run_id")?,
        step_run_id: row.try_get("step_run_id")?,
        dot_id: row.try_get("dot_id")?,
        task: row.try_get("task")?,
        reported: row.try_get::<i64, _>("reported")? != 0,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    /// Stores the first run of a member's thread, `new` (its own root, with the delegate step's
    /// run as its `workflow_step_run_id`), and its member row with `task`, in one transaction.
    /// `Conflict` when the step run has ended meanwhile: nothing is stored.
    pub async fn add_step_member(&self, new: &NewRun, task: &str) -> Result<Run> {
        let Some(step_run_id) = &new.workflow_step_run_id else {
            return Err(Error::Invalid(
                "a member's run must name its step run".into(),
            ));
        };
        let mut tx = self.pool.begin().await?;
        // The write first: a transaction that reads first cannot take the write lock while
        // another connection writes ("database is locked").
        let run_id = insert_run(&mut tx, new).await?;
        let open: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workflow_step_runs WHERE id = ? \
             AND status IN ('running', 'waiting'))",
        )
        .bind(step_run_id)
        .fetch_one(&mut *tx)
        .await?;
        if !open {
            tx.rollback().await?;
            return Err(Error::Conflict(format!("step run {step_run_id} has ended")));
        }
        sqlx::query(
            "INSERT INTO workflow_step_members (run_id, step_run_id, dot_id, task, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&run_id)
        .bind(step_run_id)
        .bind(&new.dot_id)
        .bind(task)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_run(&run_id).await
    }

    /// The members the step run's delegating run started, in the order started.
    pub async fn step_members(&self, step_run_id: &str) -> Result<Vec<StepMember>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM workflow_step_members WHERE step_run_id = ? \
             ORDER BY created_at, rowid"
        ))
        .bind(step_run_id)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(member_from_row)
        .collect()
    }

    /// The member whose thread has the root run `root_run_id`, if it is a member's thread.
    pub async fn step_member(&self, root_run_id: &str) -> Result<Option<StepMember>> {
        sqlx::query(&format!(
            "SELECT {COLUMNS} FROM workflow_step_members WHERE run_id = ?"
        ))
        .bind(root_run_id)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(member_from_row)
        .transpose()
    }

    /// Stores `new`, the run that continues the delegating thread with the results of the
    /// members whose threads have the roots `reported`, and marks those members reported, in
    /// one transaction. `Conflict` when one of them was reported meanwhile (a second pass, or
    /// a second loop after a restart): nothing is stored.
    pub async fn report_step_members(&self, new: &NewRun, reported: &[String]) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        for run_id in reported {
            let done = sqlx::query(
                "UPDATE workflow_step_members SET reported = 1 WHERE run_id = ? AND reported = 0",
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
            if done.rows_affected() == 0 {
                tx.rollback().await?;
                return Err(Error::Conflict(format!(
                    "the member {run_id} was reported already"
                )));
            }
        }
        let run_id = insert_run(&mut tx, new).await?;
        tx.commit().await?;
        self.get_run(&run_id).await
    }
}

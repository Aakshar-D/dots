//! The rows of workflow runs and their step runs (teams spec section 4), and what a workflow run
//! has spent (section 9), summed from its dot runs.

use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::{opt_json, Store};
use crate::model::{
    NewWorkflowRun, Run, StepRun, StepRunChange, StepRunStatus, WaitingOn, WorkflowRun,
    WorkflowRunChange, WorkflowRunStatus, WorkflowTrigger,
};
use crate::util::{new_id, now};
use crate::{Error, Result};

const WR_COLUMNS: &str = "id, workflow_id, team_id, definition, project_id, project, task, \
    trigger_kind, origin_run_id, test, status, waiting_on, current_step, budget, send_backs, \
    active_ms, error, started_at, ended_at";
const STEP_COLUMNS: &str = "id, workflow_run_id, step_id, attempt, status, handoff, decision, \
    error, started_at, ended_at";

fn step_from_row(row: &SqliteRow) -> Result<StepRun> {
    Ok(StepRun {
        id: row.try_get("id")?,
        workflow_run_id: row.try_get("workflow_run_id")?,
        step_id: row.try_get("step_id")?,
        attempt: row.try_get::<i64, _>("attempt")? as u32,
        status: StepRunStatus::parse(&row.try_get::<String, _>("status")?)?,
        handoff: opt_json(row.try_get("handoff")?)?
            .map(serde_json::from_value)
            .transpose()?,
        decision: opt_json(row.try_get("decision")?)?,
        error: row.try_get("error")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
    })
}

impl Store {
    pub async fn create_workflow_run(&self, new: &NewWorkflowRun) -> Result<WorkflowRun> {
        let id = new_id();
        let ts = now();
        sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, team_id, definition, project_id, \
             project, task, trigger_kind, origin_run_id, test, status, budget, clock_at, \
             started_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.workflow_id)
        .bind(&new.team_id)
        .bind(serde_json::to_string(&new.definition)?)
        .bind(&new.project_id)
        .bind(serde_json::to_string(&new.project)?)
        .bind(&new.task)
        .bind(new.trigger.as_str())
        .bind(&new.origin_run_id)
        .bind(new.test as i64)
        .bind(WorkflowRunStatus::Running.as_str())
        .bind(serde_json::to_string(&new.definition.budget)?)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await?;
        self.get_workflow_run(&id).await
    }

    pub async fn get_workflow_run(&self, id: &str) -> Result<WorkflowRun> {
        let row = sqlx::query(&format!(
            "SELECT {WR_COLUMNS} FROM workflow_runs WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("workflow run {id}")))?;
        let steps = sqlx::query(&format!(
            "SELECT {STEP_COLUMNS} FROM workflow_step_runs WHERE workflow_run_id = ? \
             ORDER BY started_at, id"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(step_from_row)
        .collect::<Result<Vec<_>>>()?;
        let (runs, tokens, cached): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(r.tokens_in - r.tokens_cache_read + r.tokens_out), 0), \
             COALESCE(SUM(r.tokens_cache_read), 0) FROM runs r \
             JOIN workflow_step_runs s ON s.id = r.workflow_step_run_id \
             WHERE s.workflow_run_id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let waiting_on: Option<String> = row.try_get("waiting_on")?;
        Ok(WorkflowRun {
            id: row.try_get("id")?,
            workflow_id: row.try_get("workflow_id")?,
            team_id: row.try_get("team_id")?,
            definition: serde_json::from_str(&row.try_get::<String, _>("definition")?)?,
            project_id: row.try_get("project_id")?,
            project: serde_json::from_str(&row.try_get::<String, _>("project")?)?,
            task: row.try_get("task")?,
            trigger: WorkflowTrigger::parse(&row.try_get::<String, _>("trigger_kind")?)?,
            origin_run_id: row.try_get("origin_run_id")?,
            test: row.try_get::<i64, _>("test")? != 0,
            status: WorkflowRunStatus::parse(&row.try_get::<String, _>("status")?)?,
            waiting_on: waiting_on.map(|w| WaitingOn::parse(&w)).transpose()?,
            current_step: row.try_get("current_step")?,
            budget: serde_json::from_str(&row.try_get::<String, _>("budget")?)?,
            send_backs: row.try_get::<i64, _>("send_backs")? as u32,
            spent_runs: runs as u32,
            spent_tokens: tokens.max(0) as u64,
            spent_cache_read: cached.max(0) as u64,
            active_ms: row.try_get::<i64, _>("active_ms")? as u64,
            error: row.try_get("error")?,
            started_at: row.try_get("started_at")?,
            ended_at: row.try_get("ended_at")?,
            steps,
        })
    }

    /// The newest workflow runs, of one team or of all.
    pub async fn list_workflow_runs(
        &self,
        team_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM workflow_runs WHERE (?1 IS NULL OR team_id = ?1) \
             ORDER BY started_at DESC, id DESC LIMIT ?2",
        )
        .bind(team_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for id in ids {
            out.push(self.get_workflow_run(&id).await?);
        }
        Ok(out)
    }

    /// The ids of the workflow runs that have not ended (running or waiting), oldest first.
    pub async fn workflow_runs_in_flight(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM workflow_runs WHERE status IN ('running', 'waiting') \
             ORDER BY started_at, id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Whether the team has workflow runs that have not ended.
    pub async fn team_has_runs_in_flight(&self, team_id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_runs WHERE team_id = ? \
             AND status IN ('running', 'waiting')",
        )
        .bind(team_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Whether a run of the step asked a question the person has neither answered nor
    /// dismissed: the step waits for the answer (teams spec section 13).
    pub async fn step_has_open_question(&self, step_run_id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages m JOIN runs r ON r.id = m.run_id \
             WHERE r.workflow_step_run_id = ? AND m.kind = 'question' \
             AND m.answered_at IS NULL AND m.dismissed_at IS NULL",
        )
        .bind(step_run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Whether the workflow run waits on the person through one of its dot runs: a run parked
    /// on an approval, an approval asked live, or a question not answered or dismissed. Its clock
    /// stands still meanwhile (teams spec section 9).
    pub async fn workflow_waits_on_person(&self, workflow_run_id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs r JOIN workflow_step_runs s ON s.id = r.workflow_step_run_id \
             WHERE s.workflow_run_id = ? AND (r.status = 'awaiting_approval' \
             OR EXISTS (SELECT 1 FROM approvals a WHERE a.run_id = r.id AND a.status = 'pending') \
             OR EXISTS (SELECT 1 FROM messages m WHERE m.run_id = r.id AND m.kind = 'question' \
             AND m.answered_at IS NULL AND m.dismissed_at IS NULL))",
        )
        .bind(workflow_run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// When the workflow run's clock last counted (`WorkflowRunChange::clock_at`).
    pub async fn workflow_run_clock(&self, id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT clock_at FROM workflow_runs WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    /// Writes `change` to the workflow run, only while it has not ended (`Conflict` after).
    pub async fn set_workflow_run(
        &self,
        id: &str,
        change: &WorkflowRunChange,
    ) -> Result<WorkflowRun> {
        let mut sets: Vec<String> = Vec::new();
        let mut binds: Vec<Option<String>> = Vec::new();
        let mut set = |column: &str, value: Option<String>| {
            sets.push(format!("{column} = ?"));
            binds.push(value);
        };
        if let Some(status) = change.status {
            set("status", Some(status.as_str().into()));
            if status.is_terminal() {
                set("ended_at", Some(now()));
            }
        }
        if let Some(w) = &change.waiting_on {
            set("waiting_on", w.map(|w| w.as_str().into()));
        }
        if let Some(s) = &change.current_step {
            set("current_step", s.clone());
        }
        if let Some(b) = &change.budget {
            set("budget", Some(serde_json::to_string(b)?));
        }
        if let Some(n) = change.send_backs {
            set("send_backs", Some(n.to_string()));
        }
        if let Some(ms) = change.active_ms {
            set("active_ms", Some(ms.to_string()));
        }
        if let Some(c) = &change.clock_at {
            set("clock_at", c.clone());
        }
        if let Some(e) = &change.error {
            set("error", e.clone());
        }
        if !sets.is_empty() {
            let sql = format!(
                "UPDATE workflow_runs SET {} WHERE id = ? AND status IN ('running', 'waiting')",
                sets.join(", ")
            );
            let mut q = sqlx::query(&sql);
            for b in &binds {
                q = q.bind(b);
            }
            let res = q.bind(id).execute(&self.pool).await?;
            if res.rows_affected() == 0 {
                let run = self.get_workflow_run(id).await?;
                return Err(Error::Conflict(format!(
                    "workflow run {id} is already {}",
                    run.status.as_str()
                )));
            }
        }
        self.get_workflow_run(id).await
    }

    pub async fn create_step_run(
        &self,
        workflow_run_id: &str,
        step_id: &str,
        attempt: u32,
        status: StepRunStatus,
    ) -> Result<StepRun> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO workflow_step_runs (id, workflow_run_id, step_id, attempt, status, \
             started_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workflow_run_id)
        .bind(step_id)
        .bind(attempt as i64)
        .bind(status.as_str())
        .bind(now())
        .execute(&self.pool)
        .await?;
        self.get_step_run(&id).await
    }

    pub async fn get_step_run(&self, id: &str) -> Result<StepRun> {
        let row = sqlx::query(&format!(
            "SELECT {STEP_COLUMNS} FROM workflow_step_runs WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("step run {id}")))?;
        step_from_row(&row)
    }

    /// Writes `change` to the step run. A terminal status also sets `ended_at`.
    pub async fn set_step_run(&self, id: &str, change: &StepRunChange) -> Result<StepRun> {
        let mut tx = self.pool.begin().await?;
        if let Some(status) = change.status {
            sqlx::query(
                "UPDATE workflow_step_runs SET status = ?, \
                 ended_at = CASE WHEN ? THEN ? ELSE ended_at END WHERE id = ?",
            )
            .bind(status.as_str())
            .bind(status.is_terminal())
            .bind(now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(h) = &change.handoff {
            let text = h.as_ref().map(serde_json::to_string).transpose()?;
            sqlx::query("UPDATE workflow_step_runs SET handoff = ? WHERE id = ?")
                .bind(text)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(d) = &change.decision {
            let text = d.as_ref().map(Value::to_string);
            sqlx::query("UPDATE workflow_step_runs SET decision = ? WHERE id = ?")
                .bind(text)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(e) = &change.error {
            sqlx::query("UPDATE workflow_step_runs SET error = ? WHERE id = ?")
                .bind(e)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_step_run(id).await
    }

    /// The dot runs of a step run, resume children included, oldest first.
    pub async fn runs_of_step_run(&self, step_run_id: &str) -> Result<Vec<Run>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM runs WHERE workflow_step_run_id = ? ORDER BY queued_at, id",
        )
        .bind(step_run_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for id in ids {
            out.push(self.get_run(&id).await?);
        }
        Ok(out)
    }
}

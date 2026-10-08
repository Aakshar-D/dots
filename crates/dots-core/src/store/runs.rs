use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::{opt_json, Store};
use crate::model::{NewRun, Run, RunEventRecord, RunStatus, TriggerKind};
use crate::util::{new_id, now};
use crate::{Error, Result};

const RUN_COLUMNS: &str = "id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, status, \
    session_id, workspace_path, branch, base_commit, summary, error, tokens_in, tokens_out, \
    queued_at, started_at, ended_at";

fn run_from_row(row: &SqliteRow) -> Result<Run> {
    Ok(Run {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        root_run_id: row.try_get("root_run_id")?,
        parent_run_id: row.try_get("parent_run_id")?,
        trigger: TriggerKind::parse(&row.try_get::<String, _>("trigger_kind")?)?,
        payload: opt_json(row.try_get("payload")?)?,
        status: RunStatus::parse(&row.try_get::<String, _>("status")?)?,
        session_id: row.try_get("session_id")?,
        workspace_path: row.try_get("workspace_path")?,
        branch: row.try_get("branch")?,
        base_commit: row.try_get("base_commit")?,
        summary: row.try_get("summary")?,
        error: row.try_get("error")?,
        tokens_in: row.try_get("tokens_in")?,
        tokens_out: row.try_get("tokens_out")?,
        queued_at: row.try_get("queued_at")?,
        started_at: row.try_get("started_at")?,
        ended_at: row.try_get("ended_at")?,
    })
}

fn event_from_row(row: &SqliteRow) -> Result<RunEventRecord> {
    Ok(RunEventRecord {
        run_id: row.try_get("run_id")?,
        seq: row.try_get("seq")?,
        ts: row.try_get("ts")?,
        kind: row.try_get("kind")?,
        data: serde_json::from_str(&row.try_get::<String, _>("data")?)?,
    })
}

impl Store {
    pub async fn create_run(&self, new: &NewRun) -> Result<Run> {
        let id = new_id();
        let root = new.root_run_id.clone().unwrap_or_else(|| id.clone());
        sqlx::query(
            "INSERT INTO runs (id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, \
             status, session_id, workspace_path, branch, base_commit, queued_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.dot_id)
        .bind(&root)
        .bind(&new.parent_run_id)
        .bind(new.trigger.as_str())
        .bind(new.payload.as_ref().map(|v| v.to_string()))
        .bind(RunStatus::Queued.as_str())
        .bind(&new.session_id)
        .bind(&new.workspace_path)
        .bind(&new.branch)
        .bind(&new.base_commit)
        .bind(now())
        .execute(&self.pool)
        .await?;
        self.get_run(&id).await
    }

    pub async fn get_run(&self, id: &str) -> Result<Run> {
        let row = sqlx::query(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("run {id}")))?;
        run_from_row(&row)
    }

    pub async fn list_runs(&self, dot_id: Option<&str>, limit: i64) -> Result<Vec<Run>> {
        let rows = match dot_id {
            Some(d) => {
                sqlx::query(&format!(
                    "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? \
                     ORDER BY queued_at DESC, id DESC LIMIT ?"
                ))
                .bind(d)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(&format!(
                    "SELECT {RUN_COLUMNS} FROM runs ORDER BY queued_at DESC, id DESC LIMIT ?"
                ))
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.iter().map(run_from_row).collect()
    }

    pub async fn dispatchable_runs(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs r WHERE r.status = 'queued' AND NOT EXISTS \
             (SELECT 1 FROM runs x WHERE x.dot_id = r.dot_id AND x.status = 'running') \
             ORDER BY r.queued_at, r.id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    pub async fn claim_run(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'running', started_at = ? WHERE id = ? AND status = 'queued'",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn has_queued_run(&self, dot_id: &str) -> Result<bool> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE dot_id = ? AND status = 'queued'")
                .bind(dot_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(n > 0)
    }

    pub async fn set_workspace(
        &self,
        id: &str,
        path: &str,
        branch: Option<&str>,
        base_commit: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE runs SET workspace_path = ?, branch = ?, base_commit = ? WHERE id = ?")
            .bind(path)
            .bind(branch)
            .bind(base_commit)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_session(&self, id: &str, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE runs SET session_id = ? WHERE id = ?")
            .bind(session_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_usage(&self, id: &str, tokens_in: i64, tokens_out: i64) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET tokens_in = tokens_in + ?, tokens_out = tokens_out + ? WHERE id = ?",
        )
        .bind(tokens_in)
        .bind(tokens_out)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn finish_run(
        &self,
        id: &str,
        status: RunStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<Run> {
        sqlx::query(
            "UPDATE runs SET status = ?, summary = COALESCE(?, summary), error = ?, ended_at = ? \
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(summary)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_run(id).await
    }

    /// Atomically cancels a run that is `queued` or `awaiting_approval`.
    /// Returns `None` when the run is in any other state (nothing changed).
    pub async fn cancel_if_inactive(&self, id: &str) -> Result<Option<Run>> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'cancelled', ended_at = ?              WHERE id = ? AND status IN ('queued', 'awaiting_approval')",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    pub async fn set_status(&self, id: &str, status: RunStatus) -> Result<Run> {
        sqlx::query("UPDATE runs SET status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
    }

    /// Marks runs left `running` by a previous process as failed / interrupted.
    pub async fn recover_interrupted(&self) -> Result<Vec<String>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE runs SET status = 'failed', error = 'interrupted', ended_at = ? \
             WHERE status = 'running' RETURNING id",
        )
        .bind(now())
        .fetch_all(&self.pool)
        .await?;
        Ok(ids)
    }

    pub async fn append_event(
        &self,
        run_id: &str,
        kind: &str,
        data: &Value,
    ) -> Result<RunEventRecord> {
        let ts = now();
        let seq: i64 = sqlx::query_scalar(
            "INSERT INTO run_events (run_id, seq, ts, kind, data) \
             SELECT ?, COALESCE(MAX(seq), 0) + 1, ?, ?, ? FROM run_events WHERE run_id = ? \
             RETURNING seq",
        )
        .bind(run_id)
        .bind(&ts)
        .bind(kind)
        .bind(data.to_string())
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(RunEventRecord {
            run_id: run_id.to_string(),
            seq,
            ts,
            kind: kind.to_string(),
            data: data.clone(),
        })
    }

    pub async fn list_events(&self, run_id: &str, after_seq: i64) -> Result<Vec<RunEventRecord>> {
        let rows = sqlx::query(
            "SELECT run_id, seq, ts, kind, data FROM run_events WHERE run_id = ? AND seq > ? \
             ORDER BY seq",
        )
        .bind(run_id)
        .bind(after_seq)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }
}

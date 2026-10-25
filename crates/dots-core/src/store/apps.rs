//! The apps' call log (app library spec section 5) and the apps dots propose (section 6.2).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::runs::insert_run;
use super::Store;
use crate::apps::propose::{AppProposal, ProposalStatus};
use crate::apps::AppCall;
use crate::model::{NewRun, Run};
use crate::{Error, Result};

const PROPOSAL_COLUMNS: &str = "id, dot_id, run_id, root_run_id, app_name, app_id, base_version, \
    note, summary, status, answer, answer_run_id, created_at, decided_at";

fn proposal_from_row(row: &SqliteRow) -> Result<AppProposal> {
    Ok(AppProposal {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        run_id: row.try_get("run_id")?,
        root_run_id: row.try_get("root_run_id")?,
        app_name: row.try_get("app_name")?,
        app_id: row.try_get("app_id")?,
        base_version: row
            .try_get::<Option<i64>, _>("base_version")?
            .map(|v| v as u32),
        note: row.try_get("note")?,
        summary: serde_json::from_str(&row.try_get::<String, _>("summary")?)?,
        status: ProposalStatus::parse(&row.try_get::<String, _>("status")?)?,
        answer: row.try_get("answer")?,
        answer_run_id: row.try_get("answer_run_id")?,
        created_at: row.try_get("created_at")?,
        decided_at: row.try_get("decided_at")?,
    })
}

impl Store {
    /// Records a call of an app tool that writes.
    pub async fn record_app_call(&self, call: &AppCall) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_calls (id, app_id, tool, actor, run_id, input, now, ids, result, \
             error, changes, took_ms, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&call.id)
        .bind(&call.app_id)
        .bind(&call.tool)
        .bind(serde_json::to_string(&call.actor)?)
        .bind(&call.run_id)
        .bind(serde_json::to_string(&call.input)?)
        .bind(&call.now)
        .bind(serde_json::to_string(&call.ids)?)
        .bind(
            call.result
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        )
        .bind(&call.error)
        .bind(call.changes as i64)
        .bind(call.took_ms as i64)
        .bind(&call.at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_app_proposal(&self, p: &AppProposal) -> Result<()> {
        sqlx::query(&format!(
            "INSERT INTO app_proposals ({PROPOSAL_COLUMNS}) VALUES \
             (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&p.id)
        .bind(&p.dot_id)
        .bind(&p.run_id)
        .bind(&p.root_run_id)
        .bind(&p.app_name)
        .bind(&p.app_id)
        .bind(p.base_version.map(i64::from))
        .bind(&p.note)
        .bind(serde_json::to_string(&p.summary)?)
        .bind(p.status.as_str())
        .bind(&p.answer)
        .bind(&p.answer_run_id)
        .bind(&p.created_at)
        .bind(&p.decided_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_app_proposal(&self, id: &str) -> Result<AppProposal> {
        let row = sqlx::query(&format!(
            "SELECT {PROPOSAL_COLUMNS} FROM app_proposals WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("app proposal {id}")))?;
        proposal_from_row(&row)
    }

    /// The proposals of one dot or of all, newest first.
    pub async fn list_app_proposals(&self, dot_id: Option<&str>) -> Result<Vec<AppProposal>> {
        let rows = sqlx::query(&format!(
            "SELECT {PROPOSAL_COLUMNS} FROM app_proposals WHERE ? IS NULL OR dot_id = ? \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(dot_id)
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(proposal_from_row).collect()
    }

    /// The run's proposal that waits for the person, if any: a run sends one at a time.
    pub async fn pending_app_proposal_for_run(&self, run_id: &str) -> Result<Option<AppProposal>> {
        sqlx::query(&format!(
            "SELECT {PROPOSAL_COLUMNS} FROM app_proposals WHERE run_id = ? AND status = 'pending'"
        ))
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(proposal_from_row)
        .transpose()
    }

    /// Decides the pending proposal `id` and stores the run that continues the dot's thread, in
    /// one transaction: a second decision is refused.
    pub async fn decide_app_proposal(
        &self,
        id: &str,
        status: ProposalStatus,
        answer: Option<&str>,
        new: &NewRun,
    ) -> Result<(AppProposal, Run)> {
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, new).await?;
        let res = sqlx::query(
            "UPDATE app_proposals SET status = ?, answer = ?, answer_run_id = ?, decided_at = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(answer)
        .bind(&run_id)
        .bind(crate::util::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            self.get_app_proposal(id).await?;
            return Err(Error::Conflict(
                crate::apps::propose::DECIDED_ALREADY.to_string(),
            ));
        }
        tx.commit().await?;
        Ok((
            self.get_app_proposal(id).await?,
            self.get_run(&run_id).await?,
        ))
    }

    /// The newest `limit` logged calls of the app `app_id`, newest first.
    pub async fn app_calls(&self, app_id: &str, limit: u32) -> Result<Vec<AppCall>> {
        let rows = sqlx::query(
            "SELECT id, app_id, tool, actor, run_id, input, now, ids, result, error, changes, \
             took_ms, at FROM app_calls WHERE app_id = ? ORDER BY at DESC, id DESC LIMIT ?",
        )
        .bind(app_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                let result: Option<String> = r.try_get("result")?;
                Ok(AppCall {
                    id: r.try_get("id")?,
                    app_id: r.try_get("app_id")?,
                    tool: r.try_get("tool")?,
                    actor: serde_json::from_str(&r.try_get::<String, _>("actor")?)?,
                    run_id: r.try_get("run_id")?,
                    input: serde_json::from_str(&r.try_get::<String, _>("input")?)?,
                    now: r.try_get("now")?,
                    ids: serde_json::from_str(&r.try_get::<String, _>("ids")?)?,
                    result: result.map(|t| serde_json::from_str(&t)).transpose()?,
                    error: r.try_get("error")?,
                    changes: r.try_get::<i64, _>("changes")? as u64,
                    took_ms: r.try_get::<i64, _>("took_ms")? as u64,
                    at: r.try_get("at")?,
                })
            })
            .collect()
    }
}

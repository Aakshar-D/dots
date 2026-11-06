use serde_json::Value;
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::{opt_json, Store};
use crate::model::{
    Delivery, DeliveryCounts, EngineKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind,
    MAX_RESENDS, RESENT, RESENT_AFTER_RESTART,
};
use crate::util::{new_id, now, timestamp};
use crate::vault::redact::global;
use crate::{Error, Result};

pub(super) const RUN_COLUMNS: &str =
    "id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, status, \
    session_id, workspace_path, branch, base_commit, summary, error, tokens_in, tokens_out, \
    queued_at, started_at, ended_at, reviewed_at, retry_of, workflow_step_run_id, \
    tokens_cache_read, cost_usd, hold_released, quiet_since, quiet_limit_secs, \
    quiet_stopped, delivered_at, undelivered_at, undelivered_reason, dismissed_at, result_check";

/// The reason a run ended undelivered: the first non-empty line of its error, or `failed`.
fn undelivered_reason(error: Option<&str>) -> String {
    error
        .and_then(|e| e.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("failed")
        .to_string()
}

pub(super) fn run_from_row(row: &SqliteRow) -> Result<Run> {
    let status = RunStatus::parse(&row.try_get::<String, _>("status")?)?;
    let delivered_at: Option<String> = row.try_get("delivered_at")?;
    let undelivered_at: Option<String> = row.try_get("undelivered_at")?;
    Ok(Run {
        delivery: Delivery::of(status, delivered_at.as_deref(), undelivered_at.as_deref()),
        wait_reason: None,
        // A check this Dots cannot read (written by a newer one) reads as none.
        result_check: row
            .try_get::<Option<String>, _>("result_check")?
            .and_then(|s| serde_json::from_str(&s).ok()),
        delivered_at,
        undelivered_at,
        undelivered_reason: row.try_get("undelivered_reason")?,
        dismissed_at: row.try_get("dismissed_at")?,
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        root_run_id: row.try_get("root_run_id")?,
        parent_run_id: row.try_get("parent_run_id")?,
        trigger: TriggerKind::parse(&row.try_get::<String, _>("trigger_kind")?)?,
        payload: opt_json(row.try_get("payload")?)?,
        status,
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
        reviewed_at: row.try_get("reviewed_at")?,
        retry_of: row.try_get("retry_of")?,
        cost_usd: row.try_get("cost_usd")?,
        hold_released: row.try_get::<i64, _>("hold_released")? != 0,
        workflow_step_run_id: row.try_get("workflow_step_run_id")?,
        tokens_cache_read: row.try_get("tokens_cache_read")?,
        quiet_since: row.try_get("quiet_since")?,
        quiet_limit_secs: row
            .try_get::<Option<i64>, _>("quiet_limit_secs")?
            .map(|n| n as u64),
        quiet_stopped: row.try_get::<i64, _>("quiet_stopped")? != 0,
    })
}

/// A run's time from start to end, in whole seconds; `None` until it has both.
fn run_seconds(run: &Run) -> Option<i64> {
    let at = |t: &Option<String>| {
        t.as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
    };
    Some((at(&run.ended_at)? - at(&run.started_at)?).num_seconds())
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

/// A topic's runs read along its line (topic tools spec section 7): each run after its parent,
/// the children of one run by `queued_at`. A move or merge hangs older runs after newer ones,
/// so time alone would put them in the middle. `runs` is in `queued_at` order; a run whose
/// parent is not among them starts a line of its own, in that order.
pub fn line_order(runs: Vec<Run>) -> Vec<Run> {
    use std::collections::{HashMap, HashSet};
    let ids: HashSet<&str> = runs.iter().map(|r| r.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut starts = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        match run
            .parent_run_id
            .as_deref()
            .filter(|p| ids.contains(p) && *p != run.id)
        {
            Some(parent) => children.entry(parent).or_default().push(i),
            None => starts.push(i),
        }
    }
    let mut order = Vec::with_capacity(runs.len());
    let mut seen = vec![false; runs.len()];
    let mut stack: Vec<usize> = starts.into_iter().rev().collect();
    loop {
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            order.push(i);
            if let Some(kids) = children.get(runs[i].id.as_str()) {
                stack.extend(kids.iter().rev());
            }
        }
        // A cycle of parents (never written by Dots) would hide its runs: they follow by time.
        match seen.iter().position(|s| !s) {
            Some(i) => stack.push(i),
            None => break,
        }
    }
    let mut slots: Vec<Option<Run>> = runs.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

pub(super) async fn insert_run(conn: &mut SqliteConnection, new: &NewRun) -> Result<String> {
    let id = new.id.clone().unwrap_or_else(new_id);
    let root = new.root_run_id.clone().unwrap_or_else(|| id.clone());
    // A run that continues another (a resume, a chat reply, an answer, a follow-up) works for
    // the same workflow step as its parent, unless it names one (teams spec section 4); only
    // while that step is open, so a step that has ended takes in no more runs.
    sqlx::query(
        "INSERT INTO runs (id, dot_id, root_run_id, parent_run_id, trigger_kind, payload, \
         status, session_id, workspace_path, branch, base_commit, queued_at, retry_of, \
         workflow_step_run_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
         COALESCE(?, (SELECT r.workflow_step_run_id FROM runs r \
         JOIN workflow_step_runs s ON s.id = r.workflow_step_run_id \
         WHERE r.id = ? AND s.status IN ('running', 'waiting'))))",
    )
    .bind(&id)
    .bind(&new.dot_id)
    .bind(&root)
    .bind(&new.parent_run_id)
    .bind(new.trigger.as_str())
    .bind(
        new.payload
            .as_ref()
            .map(|v| global().scrub_json(v).to_string()),
    )
    .bind(RunStatus::Queued.as_str())
    .bind(&new.session_id)
    .bind(&new.workspace_path)
    .bind(&new.branch)
    .bind(&new.base_commit)
    .bind(now())
    .bind(&new.retry_of)
    .bind(&new.workflow_step_run_id)
    .bind(&new.parent_run_id)
    .execute(&mut *conn)
    .await?;
    // A run added to an archived topic brings it back (topic tools spec section 3.2).
    sqlx::query(
        "UPDATE topics SET archived_at = NULL, unarchived_at = ? \
         WHERE root_run_id = ? AND archived_at IS NOT NULL",
    )
    .bind(now())
    .bind(&root)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

/// The run `id` read in `conn`.
async fn run_in(conn: &mut SqliteConnection, id: &str) -> Result<Run> {
    let row = sqlx::query(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?"))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| Error::NotFound(format!("run {id}")))?;
    run_from_row(&row)
}

/// How many times the message of `id` was sent before it: the `retry_of` links from `id` back
/// to its first run (crash resend spec section 2).
async fn tries_before(conn: &mut SqliteConnection, id: &str) -> Result<i64> {
    let n = sqlx::query_scalar(
        "WITH RECURSIVE chain(id, n) AS ( \
           SELECT retry_of, 1 FROM runs WHERE id = ? AND retry_of IS NOT NULL \
           UNION ALL SELECT r.retry_of, c.n + 1 FROM runs r JOIN chain c ON r.id = c.id \
           WHERE r.retry_of IS NOT NULL AND c.n < 100) \
         SELECT COALESCE(MAX(n), 0) FROM chain",
    )
    .bind(id)
    .fetch_one(conn)
    .await?;
    Ok(n)
}

/// Sends again the interrupted run `id` when it carried a person's message (crash resend spec
/// section 3): a chat run of no workflow step that made no tool call and has no approved call, sent
/// fewer than `MAX_RESENDS` times before. One sent that often fails with the reason.
async fn resend_interrupted(conn: &mut SqliteConnection, id: &str) -> Result<()> {
    let run = run_in(conn, id).await?;
    if run.trigger != TriggerKind::Chat || run.workflow_step_run_id.is_some() {
        return Ok(());
    }
    // An approved call may have run: sending the message again could do it twice.
    let approved: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM approvals WHERE run_id = ? \
         AND (status = 'approved' OR apply_state = 'unknown'))",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    // So could any call it made, asked about or not: the scheduler's rule for a retry
    // (Scheduler v2 spec §6). No run event says whether a call was read-only.
    if approved || super::schedules::called_a_tool(conn, id).await? {
        return Ok(());
    }
    if tries_before(conn, id).await? >= MAX_RESENDS {
        // Left for the person: not delivered, it shows in Needs you with Retry and Discard.
        sqlx::query(
            "UPDATE runs SET error = ?1, undelivered_reason = CASE WHEN delivered_at IS NULL \
             THEN ?1 ELSE undelivered_reason END WHERE id = ?2",
        )
        .bind(format!(
            "interrupted; stopped resending after {MAX_RESENDS} tries"
        ))
        .bind(id)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }
    let parent = match &run.parent_run_id {
        Some(pid) => Some(run_in(conn, pid).await?),
        None => None,
    };
    let mut new = NewRun::resend_of(&run, parent.as_ref());
    if let Some(Value::Object(payload)) = &mut new.payload {
        payload.insert(RESENT.into(), Value::from(RESENT_AFTER_RESTART));
    }
    let resent = insert_run(conn, &new).await?;
    // The resend is the message's Retry (delivery tracking spec section 3.9): its files go
    // with it, and a run that never reached the dot is dismissed, so it waits on no one in
    // Needs you.
    sqlx::query("UPDATE chat_attachments SET run_id = ? WHERE run_id = ?")
        .bind(&resent)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE runs SET dismissed_at = ? WHERE id = ? AND delivered_at IS NULL \
         AND undelivered_at IS NOT NULL AND dismissed_at IS NULL",
    )
    .bind(now())
    .bind(id)
    .execute(&mut *conn)
    .await?;
    tracing::info!(run = %id, resent = %resent, "sent an interrupted message again");
    Ok(())
}

impl Store {
    pub async fn create_run(&self, new: &NewRun) -> Result<Run> {
        let mut conn = self.pool.acquire().await?;
        let id = insert_run(&mut conn, new).await?;
        drop(conn);
        self.get_run(&id).await
    }

    /// Atomically turns the decided parked approvals of `parent_id` into a resume:
    /// inserts the one-shot `grants` (`(tool, grant_key, mcp_fingerprint)`, scoped to the
    /// parent's `root_run_id`, read inside the transaction), marks `resolved_ids` resolved, queues
    /// `queued_ids` (approved calls that Dots performs itself in the child run, which get no
    /// grant), inserts the optional `child` run, and closes the parent as `succeeded`.
    /// The parent must still be `awaiting_approval` (e.g. not cancelled concurrently), and each
    /// of `queued_ids` an approved, self-apply, not yet queued approval of the parent;
    /// otherwise nothing is written and `Error::Conflict` is returned.
    pub async fn resume_parked(
        &self,
        parent_id: &str,
        grants: &[(String, String, Option<String>)],
        resolved_ids: &[String],
        queued_ids: &[String],
        child: Option<&NewRun>,
    ) -> Result<Option<Run>> {
        let mut tx = self.pool.begin().await?;
        // The guarded update goes first: it takes the write lock and makes
        // check-and-close atomic with everything below.
        let res = sqlx::query(
            "UPDATE runs SET status = 'succeeded' WHERE id = ? AND status = 'awaiting_approval'",
        )
        .bind(parent_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::Conflict(format!(
                "run {parent_id} is no longer awaiting approval"
            )));
        }
        let root: String = sqlx::query_scalar("SELECT root_run_id FROM runs WHERE id = ?")
            .bind(parent_id)
            .fetch_one(&mut *tx)
            .await?;
        for (tool, key, mcp_fingerprint) in grants {
            sqlx::query(
                "INSERT INTO grants (id, root_run_id, tool, input_hash, mcp_fingerprint, \
                 created_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&root)
            .bind(tool)
            .bind(key)
            .bind(mcp_fingerprint)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
        for id in resolved_ids {
            sqlx::query("UPDATE approvals SET resolved = 1 WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        for id in queued_ids {
            // Only an approved call of this run that Dots is to perform, and not yet queued.
            let res = sqlx::query(
                "UPDATE approvals SET apply_state = 'queued' WHERE id = ? AND run_id = ? \
                 AND status = 'approved' AND self_apply = 1 AND apply_state IS NULL",
            )
            .bind(id)
            .bind(parent_id)
            .execute(&mut *tx)
            .await?;
            if res.rows_affected() != 1 {
                tx.rollback().await?;
                return Err(Error::Conflict(format!(
                    "approval {id} is not an approved call of run {parent_id} for Dots to perform"
                )));
            }
        }
        let child_id = match child {
            Some(new) => Some(insert_run(&mut tx, new).await?),
            None => None,
        };
        tx.commit().await?;
        match child_id {
            Some(id) => Ok(Some(self.get_run(&id).await?)),
            None => Ok(None),
        }
    }

    /// Replaces the run's payload (scrubbed, as `insert_run` stores it): a watcher run's
    /// attachment paths, once they are in its workspace (watchers spec section 4.2).
    pub async fn set_run_payload(&self, id: &str, payload: &Value) -> Result<Run> {
        sqlx::query("UPDATE runs SET payload = ? WHERE id = ?")
            .bind(global().scrub_json(payload).to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
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

    /// Every run of the lineage rooted at `root_run_id`, oldest first.
    pub async fn list_thread(&self, root_run_id: &str) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE root_run_id = ? ORDER BY queued_at, id"
        ))
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        let runs = rows.iter().map(run_from_row).collect::<Result<Vec<_>>>()?;
        Ok(line_order(runs))
    }

    /// The `limit` newest ended runs of `dot_id` outside the lineage `root_run_id`, latest end
    /// first; with `ended_after`, only those that ended after it (conversational chat spec
    /// section 3.1).
    pub async fn ended_runs_outside(
        &self,
        dot_id: &str,
        root_run_id: &str,
        ended_after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? AND root_run_id <> ? \
             AND status IN ('succeeded', 'failed', 'cancelled') AND ended_at IS NOT NULL \
             AND (? IS NULL OR ended_at > ?) \
             ORDER BY ended_at DESC, id DESC LIMIT ?"
        ))
        .bind(dot_id)
        .bind(root_run_id)
        .bind(ended_after)
        .bind(ended_after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// The newest run of `dot_id`, optionally only among queued, running and awaiting runs.
    pub async fn latest_run(&self, dot_id: &str, active_only: bool) -> Result<Option<Run>> {
        let filter = if active_only {
            "AND status IN ('queued', 'running', 'awaiting_approval')"
        } else {
            ""
        };
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? {filter} \
             ORDER BY queued_at DESC, id DESC LIMIT 1"
        ))
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(run_from_row).transpose()
    }

    /// The newest run of each lineage, if it is finished and not reviewed yet. Only runs the
    /// Inbox can ever show (failed, or with a branch) are returned; `Runtime::inbox` filters further.
    /// A failed scheduled run that made no tool call and that the scheduler judged into a retry
    /// or into "Needs attention" is marked reviewed when it is judged (Scheduler v2), so it is
    /// not listed. Nor is a run of a workflow step: its workflow run's page shows it (teams spec
    /// section 10).
    pub async fn unreviewed_thread_ends(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs r \
             WHERE r.reviewed_at IS NULL AND r.status IN ('succeeded', 'failed', 'cancelled') \
             AND (r.status = 'failed' OR r.branch IS NOT NULL) \
             AND r.workflow_step_run_id IS NULL \
             ORDER BY r.ended_at DESC, r.id DESC"
        ))
        .fetch_all(&self.pool)
        .await?;
        let candidates = rows.iter().map(run_from_row).collect::<Result<Vec<_>>>()?;
        // A thread's end is the last run along its line (`line_order`), not the newest by time:
        // a move or merge hangs older runs after newer ones (topic tools spec section 4.6).
        let mut ends: std::collections::HashMap<String, Option<String>> = Default::default();
        let mut out = Vec::new();
        for run in candidates {
            if !ends.contains_key(&run.root_run_id) {
                let end = self
                    .list_thread(&run.root_run_id)
                    .await?
                    .pop()
                    .map(|r| r.id);
                ends.insert(run.root_run_id.clone(), end);
            }
            if ends[&run.root_run_id].as_deref() == Some(run.id.as_str()) {
                out.push(run);
            }
        }
        Ok(out)
    }

    /// Chat runs marked not delivered that the person has not retried or discarded and that
    /// were not delivered after all: the Inbox's "Not delivered" section, newest mark first.
    /// A run of a workflow step is not listed: its workflow run waits on the failed step, and
    /// the run page shows it (teams spec section 10).
    pub async fn undelivered_chat_runs(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE trigger_kind = 'chat' \
             AND undelivered_at IS NOT NULL AND dismissed_at IS NULL AND delivered_at IS NULL \
             AND workflow_step_run_id IS NULL \
             ORDER BY undelivered_at DESC, id DESC"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// Sets `reviewed_at` on every run of the lineage that has none; returns the runs changed.
    pub async fn mark_thread_reviewed(&self, root_run_id: &str) -> Result<Vec<Run>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE runs SET reviewed_at = ? WHERE root_run_id = ? AND reviewed_at IS NULL \
             RETURNING id",
        )
        .bind(now())
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        let mut runs = Vec::new();
        for id in ids {
            runs.push(self.get_run(&id).await?);
        }
        Ok(runs)
    }

    /// The queued runs that may start, oldest first: a dot runs one ordinary run at a time, and
    /// a run of a workflow step whenever a slot is free (teams spec section 8).
    pub async fn dispatchable_runs(&self) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs r WHERE r.status = 'queued' AND \
             (r.workflow_step_run_id IS NOT NULL OR NOT EXISTS \
             (SELECT 1 FROM runs x WHERE x.dot_id = r.dot_id AND x.status = 'running' \
             AND x.workflow_step_run_id IS NULL)) \
             ORDER BY r.queued_at, r.id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// The queued runs with their dot's engine, oldest first: all of them, or with
    /// `dispatchable` only those the dispatcher may start, as `dispatchable_runs` picks them.
    pub async fn queued_with_engine(&self, dispatchable: bool) -> Result<Vec<(Run, EngineKind)>> {
        let only = if dispatchable {
            " AND (r.workflow_step_run_id IS NOT NULL OR NOT EXISTS \
             (SELECT 1 FROM runs x WHERE x.dot_id = r.dot_id AND x.status = 'running' \
             AND x.workflow_step_run_id IS NULL))"
        } else {
            ""
        };
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS}, (SELECT d.engine FROM dots d WHERE d.id = r.dot_id) \
             AS dot_engine FROM runs r WHERE r.status = 'queued'{only} \
             ORDER BY r.queued_at, r.id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let engine = EngineKind::parse(&row.try_get::<String, _>("dot_engine")?)?;
                Ok((run_from_row(row)?, engine))
            })
            .collect()
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

    /// The dots with a run whose status is `running`, each once, by id.
    pub async fn running_dot_ids(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT dot_id FROM runs WHERE status = 'running' ORDER BY dot_id",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// How many runs have not ended: queued, running or waiting for an approval (updates UI
    /// spec section 4.2, the idle install).
    pub async fn unfinished_run_count(&self) -> Result<usize> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM runs WHERE status IN ('queued', 'running', 'awaiting_approval')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(n as usize)
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

    /// Adds a usage report to the run's totals; `cache_read` is the part of `tokens_in` read
    /// from the prompt cache. A cost is added to the run's (a run with none yet takes it as is);
    /// a report without one leaves the cost as it was.
    pub async fn add_usage(
        &self,
        id: &str,
        tokens_in: i64,
        tokens_out: i64,
        cache_read: i64,
        cost_usd: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET tokens_in = tokens_in + ?, tokens_out = tokens_out + ?, \
             tokens_cache_read = tokens_cache_read + ?, \
             cost_usd = CASE WHEN ? IS NULL THEN cost_usd ELSE COALESCE(cost_usd, 0) + ? END \
             WHERE id = ?",
        )
        .bind(tokens_in)
        .bind(tokens_out)
        .bind(cache_read)
        .bind(cost_usd)
        .bind(cost_usd)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Puts a `running` run back in the queue as if it had never started (usage and limits spec
    /// section 4: Claude's limit ended it before it did anything). Its trigger and payload stay;
    /// its session, workspace and outcome go, so it starts afresh. `Conflict` if not running.
    /// `queued` is the run as it was queued: its session, folder, branch and base commit are
    /// put back (a follow-up keeps its thread's), and what this attempt set goes.
    pub async fn requeue_run(&self, id: &str, queued: &Run) -> Result<Run> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'queued', started_at = NULL, session_id = ?, \
             workspace_path = ?, branch = ?, base_commit = ?, error = NULL, \
             summary = NULL, quiet_since = NULL WHERE id = ? AND status = 'running'",
        )
        .bind(&queued.session_id)
        .bind(&queued.workspace_path)
        .bind(&queued.branch)
        .bind(&queued.base_commit)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() != 1 {
            return Err(Error::Conflict(format!("run {id} is not running")));
        }
        self.get_run(id).await
    }

    /// Sets a run's start and end, for tests of the Usage page's periods.
    #[doc(hidden)]
    pub async fn set_run_times(&self, id: &str, started: &str, ended: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE runs SET started_at = ?, ended_at = ? WHERE id = ?")
            .bind(started)
            .bind(ended)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets an event's `ts`, for tests of the Files panel.
    #[doc(hidden)]
    pub async fn set_event_ts(&self, run_id: &str, seq: i64, ts: &str) -> Result<()> {
        sqlx::query("UPDATE run_events SET ts = ? WHERE run_id = ? AND seq = ?")
            .bind(ts)
            .bind(run_id)
            .bind(seq)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets a run's `queued_at`, for tests of delivery tracking.
    #[doc(hidden)]
    pub async fn set_run_queued_at(&self, id: &str, at: &str) -> Result<()> {
        sqlx::query("UPDATE runs SET queued_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records that the run's engine started on it (delivery tracking spec section 2), and
    /// clears any not-delivered mark. `None` when it was already delivered (nothing changed).
    pub async fn mark_delivered(
        &self,
        id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Run>> {
        if !self.set_delivered(id, at).await? {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    /// `mark_delivered`'s write alone: whether it changed the row. For the runner, whose run
    /// must not fail when only reading the row back for the announcement fails.
    pub async fn set_delivered(&self, id: &str, at: chrono::DateTime<chrono::Utc>) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE runs SET delivered_at = ?, undelivered_at = NULL, undelivered_reason = NULL \
             WHERE id = ? AND delivered_at IS NULL",
        )
        .bind(timestamp(at))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Marks a run not delivered, only while it is still queued and neither delivered, marked
    /// nor dismissed: a run just cancelled or claimed by the dispatcher is never "not
    /// delivered" (delivery tracking spec section 3.9.1). `None` otherwise (nothing changed).
    pub async fn mark_undelivered(
        &self,
        id: &str,
        reason: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Run>> {
        let res = sqlx::query(
            "UPDATE runs SET undelivered_at = ?, undelivered_reason = ? WHERE id = ? \
             AND status = 'queued' AND delivered_at IS NULL AND undelivered_at IS NULL \
             AND dismissed_at IS NULL",
        )
        .bind(timestamp(at))
        .bind(reason)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    /// Marks a run dismissed (retried or discarded), only while it is queued or has ended: a
    /// run the dispatcher has claimed is reaching its dot and stays. `None` if it already was
    /// dismissed (which guards against a double Retry) or is running or awaiting an approval.
    pub async fn dismiss_run(
        &self,
        id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Run>> {
        let res = sqlx::query(
            "UPDATE runs SET dismissed_at = ? WHERE id = ? AND dismissed_at IS NULL \
             AND status NOT IN ('running', 'awaiting_approval')",
        )
        .bind(timestamp(at))
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    /// Clears a run's dismissal (a refused Retry leaves the message retryable).
    pub async fn undismiss_run(&self, id: &str) -> Result<Run> {
        sqlx::query("UPDATE runs SET dismissed_at = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
    }

    /// Queued runs with no delivery mark and no dismissal that were queued at or before
    /// `queued_before`, oldest first.
    pub async fn overdue_undelivered(&self, queued_before: &str) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE status = 'queued' AND delivered_at IS NULL \
             AND undelivered_at IS NULL AND dismissed_at IS NULL AND queued_at <= ? \
             ORDER BY queued_at, id"
        ))
        .bind(queued_before)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    /// The dot's sends queued in `[day_start, day_end)`; a resume run continues a send and is
    /// left out (delivery tracking spec section 3.9).
    ///
    /// `queued_at` is compared as text, so both bounds must be written as stored times are:
    /// with `util::timestamp` (RFC 3339 UTC, milliseconds, `Z`). Any other spelling of the same
    /// instant (`+00:00`, no milliseconds) sorts differently, and a run queued exactly at
    /// midnight would land in the wrong day.
    pub async fn delivery_counts(
        &self,
        dot_id: &str,
        day_start: &str,
        day_end: &str,
    ) -> Result<DeliveryCounts> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS sent, \
             COALESCE(SUM(delivered_at IS NOT NULL), 0) AS delivered, \
             COALESCE(SUM(undelivered_at IS NOT NULL AND delivered_at IS NULL), 0) AS not_delivered \
             FROM runs WHERE dot_id = ? AND trigger_kind <> 'resume' \
             AND queued_at >= ? AND queued_at < ?",
        )
        .bind(dot_id)
        .bind(day_start)
        .bind(day_end)
        .fetch_one(&self.pool)
        .await?;
        Ok(DeliveryCounts {
            sent: row.try_get::<i64, _>("sent")? as u32,
            delivered: row.try_get::<i64, _>("delivered")? as u32,
            not_delivered: row.try_get::<i64, _>("not_delivered")? as u32,
        })
    }

    /// Each dot's runs started at or after `since` (usage and limits spec section 5), heaviest
    /// dot first, then by name.
    pub async fn usage_rows(&self, since: &str) -> Result<Vec<crate::model::UsageRow>> {
        use crate::model::{UsageRow, UsageRun};
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS}, \
             (SELECT d.name FROM dots d WHERE d.id = runs.dot_id) AS dot_name, \
             (SELECT d.engine FROM dots d WHERE d.id = runs.dot_id) AS dot_engine \
             FROM runs WHERE started_at >= ? ORDER BY started_at, id"
        ))
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        let mut by_dot: Vec<UsageRow> = Vec::new();
        for row in &rows {
            let run = run_from_row(row)?;
            let secs = run_seconds(&run);
            let at = match by_dot.iter().position(|r| r.dot_id == run.dot_id) {
                Some(i) => i,
                None => {
                    by_dot.push(UsageRow {
                        dot_id: run.dot_id.clone(),
                        dot_name: row.try_get("dot_name")?,
                        engine: EngineKind::parse(&row.try_get::<String, _>("dot_engine")?)?,
                        runs: 0,
                        tokens_in: 0,
                        tokens_out: 0,
                        run_secs: 0,
                        cost_usd: None,
                        heaviest: Vec::new(),
                    });
                    by_dot.len() - 1
                }
            };
            let entry = &mut by_dot[at];
            entry.runs += 1;
            entry.tokens_in += run.tokens_in;
            entry.tokens_out += run.tokens_out;
            entry.run_secs += secs.unwrap_or(0);
            if let Some(cost) = run.cost_usd {
                entry.cost_usd = Some(entry.cost_usd.unwrap_or(0.0) + cost);
            }
            entry.heaviest.push(UsageRun {
                run_id: run.id,
                trigger: run.trigger,
                started_at: run.started_at.unwrap_or_default(),
                tokens_in: run.tokens_in,
                tokens_out: run.tokens_out,
                run_secs: secs,
                cost_usd: run.cost_usd,
            });
        }
        for entry in &mut by_dot {
            entry
                .heaviest
                .sort_by_key(|r| std::cmp::Reverse(r.tokens_in + r.tokens_out));
            entry.heaviest.truncate(crate::limits::HEAVIEST);
        }
        by_dot.sort_by(|a, b| {
            (b.tokens_in + b.tokens_out)
                .cmp(&(a.tokens_in + a.tokens_out))
                .then_with(|| a.dot_name.cmp(&b.dot_name))
        });
        Ok(by_dot)
    }

    /// Marks a run released from the hold by **Run anyway** (usage and limits spec section 4).
    pub async fn set_hold_released(&self, id: &str) -> Result<Run> {
        sqlx::query("UPDATE runs SET hold_released = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
    }

    /// Records the quiet limit the run started with (stuck-run watchdog spec section 4.3).
    pub async fn set_run_quiet_limit(&self, id: &str, secs: Option<u64>) -> Result<()> {
        sqlx::query("UPDATE runs SET quiet_limit_secs = ? WHERE id = ?")
            .bind(secs.map(|n| n as i64))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets or clears the run's quiet mark, only while it is `running`; returns the row.
    pub async fn set_run_quiet(&self, id: &str, since: Option<&str>) -> Result<Run> {
        sqlx::query("UPDATE runs SET quiet_since = ? WHERE id = ? AND status = 'running'")
            .bind(since)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
    }

    pub async fn finish_run(
        &self,
        id: &str,
        status: RunStatus,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<Run> {
        self.finish_run_with(id, status, summary, error, false)
            .await
    }

    /// `finish_run`, recording whether the watchdog stopped the run (`quiet_stopped`). The
    /// quiet mark is cleared either way.
    pub async fn finish_run_with(
        &self,
        id: &str,
        status: RunStatus,
        summary: Option<&str>,
        error: Option<&str>,
        quiet_stopped: bool,
    ) -> Result<Run> {
        let error = error.map(|e| global().scrub(e).into_owned());
        // A run that fails before it was delivered is marked in this write; a cancelled one
        // leaves Needs you if it was marked (delivery tracking spec section 3.9).
        sqlx::query(
            "UPDATE runs SET status = ?1, summary = COALESCE(?2, summary), error = ?3, \
             ended_at = ?4, quiet_since = NULL, quiet_stopped = ?5, \
             undelivered_at = CASE WHEN ?1 = 'failed' AND delivered_at IS NULL \
              THEN COALESCE(undelivered_at, ?4) ELSE undelivered_at END, \
             undelivered_reason = CASE WHEN ?1 = 'failed' AND delivered_at IS NULL \
              THEN ?6 ELSE undelivered_reason END, \
             dismissed_at = CASE WHEN ?1 = 'cancelled' AND undelivered_at IS NOT NULL \
              THEN COALESCE(dismissed_at, ?4) ELSE dismissed_at END \
             WHERE id = ?7",
        )
        .bind(status.as_str())
        .bind(summary.map(|s| global().scrub(s).into_owned()))
        .bind(&error)
        .bind(now())
        .bind(quiet_stopped as i64)
        .bind(undelivered_reason(error.as_deref()))
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_run(id).await
    }

    /// Sets the run's result check (`None` clears it) and returns the run.
    pub async fn set_result_check(
        &self,
        id: &str,
        check: Option<&crate::result_check::ResultCheck>,
    ) -> Result<Run> {
        let json = check.map(serde_json::to_string).transpose()?;
        sqlx::query("UPDATE runs SET result_check = ? WHERE id = ?")
            .bind(json)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_run(id).await
    }

    /// Marks every check still `checking` as skipped with `reason`: Dots stopped while it ran.
    /// A run that did not end succeeded (Dots stopped before it was stored) loses its check.
    /// Answers how many it changed.
    pub async fn skip_unfinished_checks(&self, reason: &str) -> Result<u64> {
        let skipped = crate::result_check::ResultCheck::Skipped {
            reason: reason.to_string(),
        };
        let res = sqlx::query(
            "UPDATE runs SET result_check = ? \
             WHERE result_check LIKE '{\"state\":\"checking\"%'",
        )
        .bind(serde_json::to_string(&skipped)?)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    /// Marks a run `failed` with `error` only if it is still `running`; `None` otherwise.
    pub async fn fail_if_running(&self, id: &str, error: &str) -> Result<Option<Run>> {
        let res = sqlx::query(
            "UPDATE runs SET status = 'failed', error = ?1, ended_at = ?2, quiet_since = NULL, \
             undelivered_at = CASE WHEN delivered_at IS NULL \
              THEN COALESCE(undelivered_at, ?2) ELSE undelivered_at END, \
             undelivered_reason = CASE WHEN delivered_at IS NULL \
              THEN ?3 ELSE undelivered_reason END \
             WHERE id = ?4 AND status = 'running'",
        )
        .bind(error)
        .bind(now())
        .bind(undelivered_reason(Some(error)))
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(self.get_run(id).await?))
    }

    /// Atomically cancels a run that is `queued` or `awaiting_approval`.
    /// Returns `None` when the run is in any other state (nothing changed).
    pub async fn cancel_if_inactive(&self, id: &str) -> Result<Option<Run>> {
        self.cancel_if_status(id, "('queued', 'awaiting_approval')")
            .await
    }

    /// `cancel_if_inactive` for a `queued` run only: one awaiting an approval reached its dot
    /// and is left alone. `None` when the run is not queued (nothing changed).
    pub async fn cancel_if_queued(&self, id: &str) -> Result<Option<Run>> {
        self.cancel_if_status(id, "('queued')").await
    }

    /// Cancels the run if its status is in `statuses`, an SQL list of literals.
    async fn cancel_if_status(&self, id: &str, statuses: &str) -> Result<Option<Run>> {
        let res = sqlx::query(&format!(
            "UPDATE runs SET status = 'cancelled', ended_at = ?1, quiet_since = NULL, \
             dismissed_at = CASE WHEN undelivered_at IS NOT NULL \
              THEN COALESCE(dismissed_at, ?1) ELSE dismissed_at END \
             WHERE id = ?2 AND status IN {statuses}"
        ))
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

    /// Marks runs left `running` by a previous process as failed / interrupted and, in the
    /// same transaction, expires their pending approvals (no engine is left to act on them)
    /// and closes the approved calls they were performing (`applying` becomes `unknown`,
    /// `queued` becomes `skipped`). Also closes every call left `applying`, and every call left
    /// `queued` whose resume run is neither queued nor running. A person's message among them
    /// is queued again (`resend_interrupted`), in the same transaction, so a recovery cut short
    /// does nothing and the next one does it all. Returns the interrupted runs.
    pub async fn recover_interrupted(&self) -> Result<Vec<String>> {
        let ts = now();
        let mut tx = self.pool.begin().await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "UPDATE runs SET status = 'failed', error = 'interrupted', ended_at = ?1, \
             quiet_since = NULL, \
             undelivered_at = CASE WHEN delivered_at IS NULL \
              THEN COALESCE(undelivered_at, ?1) ELSE undelivered_at END, \
             undelivered_reason = CASE WHEN delivered_at IS NULL \
              THEN 'interrupted' ELSE undelivered_reason END \
             WHERE status = 'running' RETURNING id",
        )
        .bind(&ts)
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            sqlx::query(
                "UPDATE approvals SET status = 'expired', decided_at = ? \
                 WHERE run_id = ? AND status = 'pending'",
            )
            .bind(&ts)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            // An interrupted resume run was performing its parent's approved calls: one it had
            // claimed has no result, and one it had not reached will not be performed.
            sqlx::query(
                "UPDATE approvals SET apply_state = CASE apply_state WHEN 'applying' \
                 THEN 'unknown' ELSE 'skipped' END \
                 WHERE apply_state IN ('queued', 'applying') \
                 AND run_id = (SELECT parent_run_id FROM runs WHERE id = ?)",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        // No run is running now, so nothing claimed can still get a result.
        sqlx::query("UPDATE approvals SET apply_state = 'unknown' WHERE apply_state = 'applying'")
            .execute(&mut *tx)
            .await?;
        // A queued call is performed only by its run's resume run while that run is still to
        // start or works. One whose resume run ended without closing it (the process stopped
        // between the run's last status and the close, or the close failed) is given up.
        sqlx::query(
            "UPDATE approvals SET apply_state = 'skipped' WHERE apply_state = 'queued' \
             AND NOT EXISTS (SELECT 1 FROM runs c WHERE c.parent_run_id = approvals.run_id \
             AND c.trigger_kind = 'resume' AND c.status IN ('queued', 'running'))",
        )
        .execute(&mut *tx)
        .await?;
        for id in &ids {
            resend_interrupted(&mut tx, id).await?;
        }
        tx.commit().await?;
        Ok(ids)
    }

    pub async fn list_runs_by_status(&self, status: RunStatus) -> Result<Vec<Run>> {
        let rows = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE status = ? ORDER BY queued_at, id"
        ))
        .bind(status.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_from_row).collect()
    }

    pub async fn append_event(
        &self,
        run_id: &str,
        kind: &str,
        data: &Value,
    ) -> Result<RunEventRecord> {
        let ts = now();
        // What the run event shows, the live view included: no secret value (env vault §10).
        let data = &global().scrub_json(data);
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

use chrono::{Local, SubsecRound, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::library::check_uses;
use super::Store;
use crate::library::{LibraryConfig, Uses};
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
use crate::schedule::Schedule;
use crate::util::{ct_eq, new_id, new_token, now, token_hash};
use crate::vault::Grants;
use crate::{Error, Result};

pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, variables, browser, \
    use_global_instructions, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at, seen_at, org_id, \
    workspace_id";

pub(super) fn dot_from_row(row: &SqliteRow) -> Result<Dot> {
    Ok(Dot {
        id: row.try_get("id")?,
        has_webhook_token: !row.try_get::<String, _>("webhook_token")?.is_empty(),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        seen_at: row.try_get("seen_at")?,
        org_id: row.try_get("org_id")?,
        workspace_id: row.try_get("workspace_id")?,
        spec: DotSpec {
            name: row.try_get("name")?,
            instructions: row.try_get("instructions")?,
            engine: EngineKind::parse(&row.try_get::<String, _>("engine")?)?,
            model: row.try_get("model")?,
            endpoint_url: row.try_get("endpoint_url")?,
            workdir: row.try_get("workdir")?,
            workspace_mode: WorkspaceMode::parse(&row.try_get::<String, _>("workspace_mode")?)?,
            schedule: row
                .try_get::<Option<String>, _>("schedule")?
                .map(|s| serde_json::from_str(&s))
                .transpose()?,
            policy: serde_json::from_str(&row.try_get::<String, _>("policy")?)?,
            uses: serde_json::from_str(&row.try_get::<String, _>("uses")?)?,
            variables: serde_json::from_str(&row.try_get::<String, _>("variables")?)?,
            browser: row.try_get::<i64, _>("browser")? != 0,
            use_global_instructions: row.try_get::<i64, _>("use_global_instructions")? != 0,
            use_user_settings: row.try_get::<i64, _>("use_user_settings")? != 0,
            max_turns: row.try_get::<i64, _>("max_turns")? as u32,
            timeout_secs: row.try_get::<i64, _>("timeout_secs")? as u64,
            approval_wait_secs: row.try_get::<i64, _>("approval_wait_secs")? as u64,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
        },
    })
}

/// The spec as it is stored: validated, its schedule prepared against `previous`, the stored
/// one (`Schedule::prepare_save`: the `Every` anchor, and the refusal of a new or changed
/// schedule that would never run).
fn prepared(spec: &DotSpec, previous: Option<&Schedule>) -> Result<DotSpec> {
    spec.validate()?;
    let mut spec = spec.clone();
    if let Some(schedule) = &mut spec.schedule {
        // Whole milliseconds, as `util::now()`: the stored anchor reads back the same.
        schedule.prepare_save(previous, Utc::now().trunc_subsecs(3), &Local)?;
    }
    Ok(spec)
}

fn schedule_json(spec: &DotSpec) -> Result<Option<String>> {
    Ok(spec
        .schedule
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?)
}

fn map_unique(e: sqlx::Error, name: &str) -> Error {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return Error::Conflict(format!("a dot named {name:?} already exists"));
        }
    }
    e.into()
}

impl Store {
    /// `spec`'s grants as a save stores them (`Grants::synced`): a server newly used, alone or
    /// with its plugin, is granted the variables of its config; `before` is what the dot used.
    pub(crate) async fn synced_grants(
        &self,
        before: Option<&Uses>,
        spec: &DotSpec,
    ) -> Result<Grants> {
        let had: Vec<String> = match before {
            Some(uses) => self
                .used_mcp_servers(uses)
                .await?
                .into_iter()
                .map(|i| i.name)
                .collect(),
            None => Vec::new(),
        };
        let using: Vec<(String, Vec<String>)> = self
            .used_mcp_servers(&spec.uses)
            .await?
            .into_iter()
            .filter_map(|i| match i.config {
                LibraryConfig::McpServer(c) => Some((i.name, c.variables())),
                _ => None,
            })
            .collect();
        Ok(spec.variables.synced(&had, &using))
    }

    /// Creates a dot, in the org `org_id` or, with `None`, in Personal. Nothing changes the org
    /// afterwards (env vault spec §5.2). Whether the org is joined is the caller's check
    /// (`Runtime::create_dot`); so is whether `workspace_id` exists in that account. Every item
    /// its `uses` names must be in the Library (`Invalid`). The servers it uses are granted
    /// their variables.
    pub async fn create_dot(
        &self,
        spec: &DotSpec,
        org_id: Option<&str>,
        workspace_id: &str,
    ) -> Result<Dot> {
        let mut spec = prepared(spec, None)?;
        spec.variables = self.synced_grants(None, &spec).await?;
        let spec = &spec;
        let id = new_id();
        let ts = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, variables, browser, \
             use_global_instructions, use_user_settings, max_turns, timeout_secs, \
             approval_wait_secs, enabled, created_at, updated_at, org_id, workspace_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&spec.name)
        .bind(&spec.instructions)
        .bind(spec.engine.as_str())
        .bind(&spec.model)
        .bind(&spec.endpoint_url)
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(schedule_json(spec)?)
        // No token until one is made, which shows it once.
        .bind("")
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(serde_json::to_string(&spec.variables)?)
        .bind(spec.browser as i64)
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
        .bind(spec.approval_wait_secs as i64)
        .bind(spec.enabled as i64)
        .bind(&ts)
        .bind(&ts)
        .bind(org_id)
        .bind(workspace_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        check_uses(&mut tx, &spec.uses).await?;
        tx.commit().await?;
        self.get_dot(&id).await
    }

    /// Moves a dot to another workspace of its account. Whether the workspace exists is the
    /// caller's check.
    pub async fn set_dot_workspace(&self, id: &str, workspace_id: &str) -> Result<()> {
        let done = sqlx::query("UPDATE dots SET workspace_id = ? WHERE id = ?")
            .bind(workspace_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
        Ok(())
    }

    pub async fn get_dot(&self, id: &str) -> Result<Dot> {
        let row = sqlx::query(&format!("SELECT {DOT_COLUMNS} FROM dots WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("dot {id}")))?;
        dot_from_row(&row)
    }

    pub async fn list_dots(&self) -> Result<Vec<Dot>> {
        let rows = sqlx::query(&format!("SELECT {DOT_COLUMNS} FROM dots ORDER BY name"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(dot_from_row).collect()
    }

    /// Updates a dot. `workdir`/`workspace_mode` may only change while the dot has no
    /// queued, running or awaiting runs (their workspaces were derived from the old values);
    /// otherwise nothing changes and `Error::Conflict` is returned. The check is atomic. The
    /// schedule is prepared against the stored one (`prepared`). Every item its `uses` names
    /// must be in the Library (`Invalid`). A server it uses now and did not before is granted
    /// its variables; one it no longer uses loses its grants.
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        let stored = self.get_dot(id).await?.spec;
        let mut spec = prepared(spec, stored.schedule.as_ref())?;
        spec.variables = self.synced_grants(Some(&stored.uses), &spec).await?;
        let spec = &spec;
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, variables = ?, \
             browser = ?, \
             use_global_instructions = ?, use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, updated_at = ? WHERE id = ? \
             AND ((workdir = ? AND workspace_mode = ?) OR NOT EXISTS (SELECT 1 FROM runs \
             WHERE dot_id = ? AND status IN ('queued','running','awaiting_approval')))",
        )
        .bind(&spec.name)
        .bind(&spec.instructions)
        .bind(spec.engine.as_str())
        .bind(&spec.model)
        .bind(&spec.endpoint_url)
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(schedule_json(spec)?)
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(serde_json::to_string(&spec.uses)?)
        .bind(serde_json::to_string(&spec.variables)?)
        .bind(spec.browser as i64)
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
        .bind(spec.approval_wait_secs as i64)
        .bind(spec.enabled as i64)
        .bind(now())
        .bind(id)
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            self.get_dot(id).await?;
            return Err(Error::Conflict(
                "workdir and workspace_mode cannot change while the dot has queued, running or \
                 awaiting runs"
                    .into(),
            ));
        }
        check_uses(&mut tx, &spec.uses).await?;
        tx.commit().await?;
        self.get_dot(id).await
    }

    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM dots WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
        Ok(())
    }

    /// Deletes the dot only if it has no queued/running/awaiting runs, atomically.
    pub async fn delete_dot_if_idle(&self, id: &str) -> Result<()> {
        let res = sqlx::query(
            "DELETE FROM dots WHERE id = ? AND NOT EXISTS (SELECT 1 FROM runs WHERE dot_id = ?              AND status IN ('queued','running','awaiting_approval'))",
        )
        .bind(id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            self.get_dot(id).await?;
            return Err(Error::Conflict(
                "dot has queued, running or awaiting runs; cancel them first".into(),
            ));
        }
        Ok(())
    }

    /// Sets a dot's grants as they are, for a change Dots makes itself (env vault spec §11).
    pub(crate) async fn set_dot_variables(&self, id: &str, grants: &Grants) -> Result<()> {
        sqlx::query("UPDATE dots SET variables = ? WHERE id = ?")
            .bind(serde_json::to_string(grants)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_dot_enabled(&self, id: &str, enabled: bool) -> Result<Dot> {
        let res = sqlx::query("UPDATE dots SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(enabled as i64)
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
        self.get_dot(id).await
    }

    /// Marks the dot seen now (needs-you spec section 3). `updated_at` stays: looking at a dot
    /// does not change it. `NotFound` for an unknown dot.
    pub async fn mark_dot_seen(&self, id: &str) -> Result<Dot> {
        let res = sqlx::query("UPDATE dots SET seen_at = ? WHERE id = ?")
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
        self.get_dot(id).await
    }

    /// What the dot has for the person (needs-you spec section 2), as `(needs_you, unread)`:
    /// its open questions, pending approvals, paused watchers and app proposals waiting for
    /// review; and those plus its runs that ended and its notes posted after its `seen_at`
    /// (none without one). `NotFound` for an unknown dot.
    pub async fn dot_counts(&self, id: &str) -> Result<(u32, u32)> {
        let row = sqlx::query(
            "SELECT \
             (SELECT COUNT(*) FROM messages m WHERE m.dot_id = d.id AND m.kind = 'question' \
              AND m.answered_at IS NULL AND m.dismissed_at IS NULL) \
             + (SELECT COUNT(*) FROM approvals a JOIN runs r ON r.id = a.run_id \
              WHERE r.dot_id = d.id AND a.status = 'pending') \
             + (SELECT COUNT(*) FROM watchers w WHERE w.dot_id = d.id \
              AND w.paused_reason IS NOT NULL) \
             + (SELECT COUNT(*) FROM app_proposals p WHERE p.dot_id = d.id \
              AND p.status = 'pending') AS needs_you, \
             CASE WHEN d.seen_at IS NULL THEN 0 ELSE \
             (SELECT COUNT(*) FROM runs r WHERE r.dot_id = d.id \
              AND r.status IN ('succeeded', 'failed', 'cancelled') AND r.ended_at > d.seen_at) \
             + (SELECT COUNT(*) FROM messages m WHERE m.dot_id = d.id AND m.kind = 'note' \
              AND m.created_at > d.seen_at) END AS fresh \
             FROM dots d WHERE d.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("dot {id}")))?;
        let needs_you = row.try_get::<i64, _>("needs_you")? as u32;
        let fresh = row.try_get::<i64, _>("fresh")? as u32;
        Ok((needs_you, needs_you + fresh))
    }

    /// Whether `token` is the dot's webhook token. A dot without one takes none.
    pub async fn check_webhook_token(&self, id: &str, token: &str) -> Result<bool> {
        let stored: Option<String> =
            sqlx::query_scalar("SELECT webhook_token FROM dots WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let stored = stored.ok_or_else(|| Error::NotFound(format!("dot {id}")))?;
        Ok(!stored.is_empty() && ct_eq(&token_hash(token), &stored))
    }

    /// Makes the dot a new webhook token and returns it: the only time it is shown. Only its
    /// hash is kept.
    pub async fn regenerate_webhook_token(&self, id: &str) -> Result<String> {
        let token = new_token();
        let res = sqlx::query("UPDATE dots SET webhook_token = ?, updated_at = ? WHERE id = ?")
            .bind(token_hash(&token))
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
        Ok(token)
    }
}

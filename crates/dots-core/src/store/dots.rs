use chrono::{Local, SubsecRound, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::library::check_uses;
use super::Store;
use crate::inbox::{hidden_sql, InboxKind};
use crate::library::{LibraryConfig, Uses};
use crate::model::{Dot, DotSpec, EngineKind, Made, Making, WorkspaceMode};
use crate::schedule::Schedule;
use crate::util::{ct_eq, new_id, new_token, now, token_hash};
use crate::vault::Grants;
use crate::{Error, Result};

pub(super) const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, \
    workdir, workspace_mode, schedule, webhook_token, policy, uses, variables, browser, \
    quiet_limit_secs, use_global_instructions, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at, seen_at, org_id, \
    workspace_id, spend_daily_usd, spend_daily_tokens, check_results, context_rules, \
    use_graph, making_on, making_per_run, making_total, made_by_dot_id, made_by_name, \
    made_in_run, made_purpose";

pub(super) fn dot_from_row(row: &SqliteRow) -> Result<Dot> {
    // A dot is a made dot when it has the run that made it (migration 0046).
    let made = match row.try_get::<Option<String>, _>("made_in_run")? {
        Some(run_id) => Some(Made {
            by_dot_id: row.try_get("made_by_dot_id")?,
            by_name: row
                .try_get::<Option<String>, _>("made_by_name")?
                .unwrap_or_default(),
            run_id,
            purpose: row
                .try_get::<Option<String>, _>("made_purpose")?
                .unwrap_or_default(),
        }),
        None => None,
    };
    Ok(Dot {
        made,
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
            quiet_limit_secs: row
                .try_get::<Option<i64>, _>("quiet_limit_secs")?
                .map(|n| n as u64),
            spend_cap: crate::spend::DotCaps {
                daily_usd: row.try_get("spend_daily_usd")?,
                daily_tokens: row
                    .try_get::<Option<i64>, _>("spend_daily_tokens")?
                    .map(|n| n as u64),
            },
            use_global_instructions: row.try_get::<i64, _>("use_global_instructions")? != 0,
            check_results: row.try_get::<i64, _>("check_results")? != 0,
            context_rules: row
                .try_get::<Option<String>, _>("context_rules")?
                .map(|s| serde_json::from_str(&s))
                .transpose()?,
            use_graph: row.try_get::<i64, _>("use_graph")? != 0,
            making: Making {
                on: row.try_get::<i64, _>("making_on")? != 0,
                per_run: row.try_get::<i64, _>("making_per_run")? as u32,
                total: row.try_get::<i64, _>("making_total")? as u32,
            },
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
    // A model id is stored without the spaces around it (model switcher spec section 2.3).
    spec.model = spec.model.trim().to_string();
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

fn rules_json(spec: &DotSpec) -> Result<Option<String>> {
    Ok(spec
        .context_rules
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?)
}

fn map_unique(e: sqlx::Error, name: &str) -> Error {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return name_taken(name);
        }
    }
    e.into()
}

fn name_taken(name: &str) -> Error {
    Error::Conflict(format!("a dot named {name:?} already exists"))
}

/// Whether another dot (not `id`) has `name` in any letter case. The UNIQUE index on `name`
/// is case-sensitive, so this is checked apart.
async fn clashes_in_case(tx: &mut sqlx::SqliteConnection, name: &str, id: &str) -> Result<bool> {
    Ok(
        sqlx::query("SELECT 1 FROM dots WHERE name = ? COLLATE NOCASE AND id != ?")
            .bind(name)
            .bind(id)
            .fetch_optional(tx)
            .await?
            .is_some(),
    )
}

/// Keeps what the dot `id`, about to be deleted, spent in the runs started this local month, so
/// it still counts toward its makers' caps (dots-make-dots design section 6, Spend survives a
/// delete; migration 0053): one row per run that spent anything for each maker up the chain as
/// it is now, and one that counts toward the caps for every dot together, since the runs go
/// with the dot. The runs of the living dots below it are kept for the makers above it too:
/// those dots have no maker once it is gone, so without this a maker could shed their spend by
/// deleting the dot between them. A dot with no maker keeps nothing. Rows older than the month
/// count toward no cap and are removed here.
async fn keep_spend(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<()> {
    let (month_start, _) = crate::spend::month_bounds_in(Utc::now(), &Local);
    sqlx::query("DELETE FROM made_spend_kept WHERE started_at < ?")
        .bind(&month_start)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "WITH RECURSIVE up(maker, depth) AS ( \
         SELECT made_by_dot_id, 1 FROM dots WHERE id = ?1 AND made_by_dot_id IS NOT NULL \
         UNION ALL \
         SELECT d.made_by_dot_id, up.depth + 1 FROM dots d JOIN up ON d.id = up.maker \
         WHERE d.made_by_dot_id IS NOT NULL AND up.depth < ?3), \
         tree(dot, depth) AS ( \
         SELECT ?1, 0 \
         UNION ALL \
         SELECT d.id, tree.depth + 1 FROM dots d JOIN tree ON d.made_by_dot_id = tree.dot \
         WHERE tree.depth < ?3), \
         holders(maker) AS ( \
         SELECT maker FROM up UNION ALL SELECT NULL WHERE EXISTS (SELECT 1 FROM up)) \
         INSERT INTO made_spend_kept (dot_id, started_at, cost_usd, tokens) \
         SELECT holders.maker, r.started_at, COALESCE(r.cost_usd, 0.0), \
         r.tokens_in + r.tokens_out \
         FROM holders JOIN runs r ON (holders.maker IS NOT NULL OR r.dot_id = ?1) \
         WHERE r.dot_id IN (SELECT dot FROM tree) AND r.started_at >= ?2 \
         AND (COALESCE(r.cost_usd, 0.0) > 0 OR r.tokens_in + r.tokens_out > 0)",
    )
    .bind(id)
    .bind(&month_start)
    .bind(crate::spend::MAKER_DEPTH)
    .execute(&mut *tx)
    .await?;
    Ok(())
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
        self.insert_dot(&spec, org_id, workspace_id, None).await
    }

    /// Creates a dot a maker made, as `create_dot` does, and records `made` (dots-make-dots
    /// design sections 3 and 7.1). Its grants are stored exactly as `spec` gives them: a server
    /// it uses is not granted the variables of its config, since that could give the dot a name
    /// its maker lacks (section 6, Variables). What it may hold is the caller's check.
    pub async fn create_made_dot(
        &self,
        spec: &DotSpec,
        org_id: Option<&str>,
        workspace_id: &str,
        made: &Made,
    ) -> Result<Dot> {
        let spec = prepared(spec, None)?;
        self.insert_dot(&spec, org_id, workspace_id, Some(made))
            .await
    }

    /// Stores a new dot from `spec`, which is prepared and holds its grants as they are stored.
    async fn insert_dot(
        &self,
        spec: &DotSpec,
        org_id: Option<&str>,
        workspace_id: &str,
        made: Option<&Made>,
    ) -> Result<Dot> {
        let id = new_id();
        let ts = now();
        // The write lock up front: the clash check reads before the insert, and a deferred
        // transaction that reads and then writes fails at once (busy) when another writer
        // committed in between.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if clashes_in_case(&mut tx, &spec.name, &id).await? {
            return Err(name_taken(&spec.name));
        }
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, uses, variables, browser, \
             quiet_limit_secs, use_global_instructions, use_user_settings, max_turns, \
             timeout_secs, approval_wait_secs, enabled, created_at, updated_at, org_id, \
             workspace_id, spend_daily_usd, spend_daily_tokens, check_results, context_rules, \
             use_graph, making_on, making_per_run, making_total, made_by_dot_id, made_by_name, \
             made_in_run, made_purpose) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
             ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(spec.quiet_limit_secs.map(|n| n as i64))
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
        .bind(spec.spend_cap.daily_usd)
        .bind(spec.spend_cap.daily_tokens.map(|n| n as i64))
        .bind(spec.check_results as i64)
        .bind(rules_json(spec)?)
        .bind(spec.use_graph as i64)
        .bind(spec.making.on as i64)
        .bind(spec.making.per_run as i64)
        .bind(spec.making.total as i64)
        .bind(made.and_then(|m| m.by_dot_id.as_deref()))
        .bind(made.map(|m| m.by_name.as_str()))
        .bind(made.map(|m| m.run_id.as_str()))
        .bind(made.map(|m| m.purpose.as_str()))
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        check_uses(&mut tx, &spec.uses).await?;
        tx.commit().await?;
        self.get_dot(&id).await
    }

    /// The dots `maker_id` made that exist now, by name (dots-make-dots design section 9).
    pub async fn list_made_dots(&self, maker_id: &str) -> Result<Vec<Dot>> {
        let rows = sqlx::query(&format!(
            "SELECT {DOT_COLUMNS} FROM dots WHERE made_by_dot_id = ? ORDER BY name"
        ))
        .bind(maker_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(dot_from_row).collect()
    }

    /// The makers above `dot_id` that exist now, nearest first: the dot that made it, the dot
    /// that made that one, and so on (dots-make-dots design section 6, Spend). The chain ends at
    /// a dot a person made or one whose maker is deleted; no chain is followed further than
    /// `spend::MAKER_DEPTH`.
    pub async fn makers_of(&self, dot_id: &str) -> Result<Vec<Dot>> {
        let rows = sqlx::query(&format!(
            "WITH RECURSIVE up(maker, depth) AS ( \
             SELECT made_by_dot_id, 1 FROM dots WHERE id = ? AND made_by_dot_id IS NOT NULL \
             UNION ALL \
             SELECT d.made_by_dot_id, up.depth + 1 FROM dots d JOIN up ON d.id = up.maker \
             WHERE d.made_by_dot_id IS NOT NULL AND up.depth < ?) \
             SELECT {DOT_COLUMNS} FROM dots JOIN up ON dots.id = up.maker ORDER BY up.depth"
        ))
        .bind(dot_id)
        .bind(crate::spend::MAKER_DEPTH)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(dot_from_row).collect()
    }

    /// How many dots `maker_id` made that exist now: what its total limit counts (dots-make-dots
    /// design section 10).
    pub async fn count_made_dots(&self, maker_id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dots WHERE made_by_dot_id = ?")
            .bind(maker_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(n as u32)
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
    /// its variables; one it no longer uses loses its grants. Who made the dot stays as it is,
    /// and a new name is written to the dots it made (dots-make-dots design section 9).
    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        self.save_dot(id, spec, true).await
    }

    /// Updates a dot as `update_dot` does, but stores its grants exactly as `spec` gives them:
    /// for a change a maker makes to a dot it made, where the grant of a newly used server's
    /// variables could give the dot a name its maker lacks (dots-make-dots design section 6,
    /// Variables). What the dot may hold is the caller's check.
    pub async fn update_dot_exact(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        self.save_dot(id, spec, false).await
    }

    /// `update_dot` with `sync_grants`, `update_dot_exact` without.
    async fn save_dot(&self, id: &str, spec: &DotSpec, sync_grants: bool) -> Result<Dot> {
        let stored = self.get_dot(id).await?.spec;
        let mut spec = prepared(spec, stored.schedule.as_ref())?;
        if sync_grants {
            spec.variables = self.synced_grants(Some(&stored.uses), &spec).await?;
        }
        let spec = &spec;
        // The write lock up front, as in `create_dot`: the clash check reads before the update.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if spec.name != stored.name && clashes_in_case(&mut tx, &spec.name, id).await? {
            return Err(name_taken(&spec.name));
        }
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, uses = ?, variables = ?, \
             browser = ?, quiet_limit_secs = ?, spend_daily_usd = ?, spend_daily_tokens = ?, \
             use_global_instructions = ?, use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, check_results = ?, context_rules = ?, use_graph = ?, making_on = ?, \
             making_per_run = ?, making_total = ?, updated_at = ? \
             WHERE id = ? \
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
        .bind(spec.quiet_limit_secs.map(|n| n as i64))
        .bind(spec.spend_cap.daily_usd)
        .bind(spec.spend_cap.daily_tokens.map(|n| n as i64))
        .bind(spec.use_global_instructions as i64)
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
        .bind(spec.approval_wait_secs as i64)
        .bind(spec.enabled as i64)
        .bind(spec.check_results as i64)
        .bind(rules_json(spec)?)
        .bind(spec.use_graph as i64)
        .bind(spec.making.on as i64)
        .bind(spec.making.per_run as i64)
        .bind(spec.making.total as i64)
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
        if spec.name != stored.name {
            // The dots it made show its name (dots-make-dots design section 9).
            sqlx::query("UPDATE dots SET made_by_name = ? WHERE made_by_dot_id = ?")
                .bind(&spec.name)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_dot(id).await
    }

    /// Deletes the dot. What a dot with a maker spent this month is kept first, in the same
    /// transaction (`keep_spend`).
    pub async fn delete_dot(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        keep_spend(&mut tx, id).await?;
        let res = sqlx::query("DELETE FROM dots WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the dot only if it has no queued/running/awaiting runs, atomically. What a dot
    /// with a maker spent this month is kept first, in the same transaction (`keep_spend`).
    pub async fn delete_dot_if_idle(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        keep_spend(&mut tx, id).await?;
        let res = sqlx::query(
            "DELETE FROM dots WHERE id = ? AND NOT EXISTS (SELECT 1 FROM runs WHERE dot_id = ?              AND status IN ('queued','running','awaiting_approval'))",
        )
        .bind(id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            // Nothing was deleted, so nothing is kept.
            tx.rollback().await?;
            self.get_dot(id).await?;
            return Err(Error::Conflict(
                "dot has queued, running or awaiting runs; cancel them first".into(),
            ));
        }
        tx.commit().await?;
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
    /// its open questions, pending approvals, paused watchers, app proposals waiting for review
    /// and the workflow runs its chats started that wait for the person (teams spec section
    /// 10); and those plus its runs that ended and its notes posted after its `seen_at` (none
    /// without one). Items the person snoozed or marked done are left out.
    /// `NotFound` for an unknown dot.
    pub async fn dot_counts(&self, id: &str) -> Result<(u32, u32)> {
        // An item the person snoozed or marked done is left out of every term that is an
        // Inbox item (snooze and done spec section 4.2). Quiet runs are not Inbox items.
        let sql = format!(
            "SELECT \
             (SELECT COUNT(*) FROM messages m WHERE m.dot_id = d.id AND m.kind = 'question' \
              AND m.answered_at IS NULL AND m.dismissed_at IS NULL AND {question}) \
             + (SELECT COUNT(*) FROM approvals a JOIN runs r ON r.id = a.run_id \
              WHERE r.dot_id = d.id AND a.status = 'pending' AND {approval}) \
             + (SELECT COUNT(*) FROM watchers w WHERE w.dot_id = d.id \
              AND w.paused_reason IS NOT NULL AND {watcher}) \
             + (SELECT COUNT(*) FROM app_proposals p WHERE p.dot_id = d.id \
              AND p.status = 'pending' AND {app_review}) \
             + (SELECT COUNT(*) FROM runs q WHERE q.dot_id = d.id AND q.status = 'running' \
              AND q.quiet_since IS NOT NULL) \
             + (SELECT COUNT(*) FROM runs u WHERE u.dot_id = d.id AND u.trigger_kind = 'chat' \
              AND u.undelivered_at IS NOT NULL AND u.dismissed_at IS NULL \
              AND u.delivered_at IS NULL AND u.workflow_step_run_id IS NULL \
              AND {not_delivered}) \
             + (SELECT COUNT(*) FROM workflow_runs w JOIN runs o ON o.id = w.origin_run_id \
              WHERE o.dot_id = d.id AND w.status = 'waiting' AND w.waiting_on IS NOT NULL) \
             AS needs_you, \
             CASE WHEN d.seen_at IS NULL THEN 0 ELSE \
             (SELECT COUNT(*) FROM runs r WHERE r.dot_id = d.id \
              AND r.status IN ('succeeded', 'failed', 'cancelled') AND r.ended_at > d.seen_at \
              AND {review} AND {ended_not_delivered}) \
             + (SELECT COUNT(*) FROM messages m WHERE m.dot_id = d.id AND m.kind = 'note' \
              AND m.created_at > d.seen_at AND {note}) END AS fresh \
             FROM dots d WHERE d.id = ?",
            question = hidden_sql(InboxKind::Message, "m.id"),
            approval = hidden_sql(InboxKind::Approval, "a.id"),
            watcher = hidden_sql(InboxKind::Watcher, "w.id || '@' || w.paused_at"),
            app_review = hidden_sql(InboxKind::AppReview, "p.id"),
            not_delivered = hidden_sql(InboxKind::NotDelivered, "u.id"),
            review = hidden_sql(InboxKind::Review, "r.id"),
            ended_not_delivered = hidden_sql(InboxKind::NotDelivered, "r.id"),
            note = hidden_sql(InboxKind::Message, "m.id"),
        );
        let row = sqlx::query(&sql)
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

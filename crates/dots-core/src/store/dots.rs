use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::{opt_json, Store};
use crate::model::{Dot, DotSpec, EngineKind, WorkspaceMode};
use crate::util::{new_id, new_token, now};
use crate::{Error, Result};

const DOT_COLUMNS: &str = "id, name, instructions, engine, model, endpoint_url, workdir, \
    workspace_mode, schedule, webhook_token, policy, mcp_servers, use_user_settings, max_turns, \
    timeout_secs, approval_wait_secs, enabled, created_at, updated_at";

fn dot_from_row(row: &SqliteRow) -> Result<Dot> {
    Ok(Dot {
        id: row.try_get("id")?,
        webhook_token: row.try_get("webhook_token")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        spec: DotSpec {
            name: row.try_get("name")?,
            instructions: row.try_get("instructions")?,
            engine: EngineKind::parse(&row.try_get::<String, _>("engine")?)?,
            model: row.try_get("model")?,
            endpoint_url: row.try_get("endpoint_url")?,
            workdir: row.try_get("workdir")?,
            workspace_mode: WorkspaceMode::parse(&row.try_get::<String, _>("workspace_mode")?)?,
            schedule: row.try_get("schedule")?,
            policy: serde_json::from_str(&row.try_get::<String, _>("policy")?)?,
            mcp_servers: opt_json(row.try_get("mcp_servers")?)?,
            use_user_settings: row.try_get::<i64, _>("use_user_settings")? != 0,
            max_turns: row.try_get::<i64, _>("max_turns")? as u32,
            timeout_secs: row.try_get::<i64, _>("timeout_secs")? as u64,
            approval_wait_secs: row.try_get::<i64, _>("approval_wait_secs")? as u64,
            enabled: row.try_get::<i64, _>("enabled")? != 0,
        },
    })
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
    pub async fn create_dot(&self, spec: &DotSpec) -> Result<Dot> {
        spec.validate()?;
        let id = new_id();
        let ts = now();
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, endpoint_url, workdir, \
             workspace_mode, schedule, webhook_token, policy, mcp_servers, use_user_settings, \
             max_turns, timeout_secs, approval_wait_secs, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&spec.name)
        .bind(&spec.instructions)
        .bind(spec.engine.as_str())
        .bind(&spec.model)
        .bind(&spec.endpoint_url)
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(&spec.schedule)
        .bind(new_token())
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(spec.mcp_servers.as_ref().map(|v| v.to_string()))
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
        .bind(spec.approval_wait_secs as i64)
        .bind(spec.enabled as i64)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        self.get_dot(&id).await
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

    pub async fn update_dot(&self, id: &str, spec: &DotSpec) -> Result<Dot> {
        spec.validate()?;
        let res = sqlx::query(
            "UPDATE dots SET name = ?, instructions = ?, engine = ?, model = ?, endpoint_url = ?, \
             workdir = ?, workspace_mode = ?, schedule = ?, policy = ?, mcp_servers = ?, \
             use_user_settings = ?, max_turns = ?, timeout_secs = ?, approval_wait_secs = ?, \
             enabled = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&spec.name)
        .bind(&spec.instructions)
        .bind(spec.engine.as_str())
        .bind(&spec.model)
        .bind(&spec.endpoint_url)
        .bind(&spec.workdir)
        .bind(spec.workspace_mode.as_str())
        .bind(&spec.schedule)
        .bind(serde_json::to_string(&spec.policy)?)
        .bind(spec.mcp_servers.as_ref().map(|v| v.to_string()))
        .bind(spec.use_user_settings as i64)
        .bind(spec.max_turns as i64)
        .bind(spec.timeout_secs as i64)
        .bind(spec.approval_wait_secs as i64)
        .bind(spec.enabled as i64)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_unique(e, &spec.name))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {id}")));
        }
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

    pub async fn regenerate_webhook_token(&self, id: &str) -> Result<String> {
        let token = new_token();
        let res = sqlx::query("UPDATE dots SET webhook_token = ?, updated_at = ? WHERE id = ?")
            .bind(&token)
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

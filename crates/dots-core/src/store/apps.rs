//! The apps' call log (app library spec section 5).

use sqlx::Row;

use super::Store;
use crate::apps::AppCall;
use crate::Result;

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

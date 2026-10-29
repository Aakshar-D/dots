//! The API keys made on this PC: only a key's prefix and hash are kept (accounts spec §10).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::util::{ct_eq, token_hash};
use crate::{Error, Result};

/// One API key's record. Times are RFC 3339 UTC, given by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRow {
    pub id: String,
    pub scope: String,
    pub workspace_id: String,
    pub name: String,
    pub prefix: String,
    pub hash: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

const API_KEY_COLUMNS: &str =
    "id, scope, workspace_id, name, prefix, hash, created_at, last_used_at, revoked_at";

fn key_from_row(row: &SqliteRow) -> Result<ApiKeyRow> {
    Ok(ApiKeyRow {
        id: row.try_get("id")?,
        scope: row.try_get("scope")?,
        workspace_id: row.try_get("workspace_id")?,
        name: row.try_get("name")?,
        prefix: row.try_get("prefix")?,
        hash: row.try_get("hash")?,
        created_at: row.try_get("created_at")?,
        last_used_at: row.try_get("last_used_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}

impl Store {
    /// Adds a key's record. An id or hash already used is a `Conflict`.
    pub async fn add_api_key(&self, key: &ApiKeyRow) -> Result<()> {
        let added = sqlx::query(&format!(
            "INSERT INTO api_keys ({API_KEY_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&key.id)
        .bind(&key.scope)
        .bind(&key.workspace_id)
        .bind(&key.name)
        .bind(&key.prefix)
        .bind(&key.hash)
        .bind(&key.created_at)
        .bind(&key.last_used_at)
        .bind(&key.revoked_at)
        .execute(&self.pool)
        .await;
        if let Err(sqlx::Error::Database(db)) = &added {
            if db.is_unique_violation() {
                return Err(Error::Conflict(format!(
                    "the API key '{}' already exists",
                    key.id
                )));
            }
        }
        added?;
        Ok(())
    }

    /// The account's keys, newest first.
    pub async fn api_keys(&self, scope: &str) -> Result<Vec<ApiKeyRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {API_KEY_COLUMNS} FROM api_keys WHERE scope = ? \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(scope)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(key_from_row).collect()
    }

    /// The key record `id`, if there is one.
    pub async fn api_key(&self, id: &str) -> Result<Option<ApiKeyRow>> {
        let row = sqlx::query(&format!(
            "SELECT {API_KEY_COLUMNS} FROM api_keys WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(key_from_row).transpose()
    }

    /// The record of the key whose text is `key`: found by its hash, then confirmed with a
    /// constant-time comparison.
    pub async fn api_key_by_secret(&self, key: &str) -> Result<Option<ApiKeyRow>> {
        let hash = token_hash(key);
        let row = sqlx::query(&format!(
            "SELECT {API_KEY_COLUMNS} FROM api_keys WHERE hash = ?"
        ))
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;
        let found = row.as_ref().map(key_from_row).transpose()?;
        Ok(found.filter(|k| ct_eq(&k.hash, &hash)))
    }

    /// Records that the key was revoked at `at`. A key already revoked keeps its first time; one
    /// that is not there is `NotFound`.
    pub async fn revoke_api_key(&self, id: &str, at: &str) -> Result<()> {
        self.stamp_api_key("revoked_at = COALESCE(revoked_at, ?)", id, at)
            .await
    }

    /// Records that the key was used at `at`. A key that is not there is `NotFound`.
    pub async fn touch_api_key(&self, id: &str, at: &str) -> Result<()> {
        self.stamp_api_key("last_used_at = ?", id, at).await
    }

    async fn stamp_api_key(&self, assignment: &str, id: &str, at: &str) -> Result<()> {
        let done = sqlx::query(&format!("UPDATE api_keys SET {assignment} WHERE id = ?"))
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("no API key '{id}'")));
        }
        Ok(())
    }
}

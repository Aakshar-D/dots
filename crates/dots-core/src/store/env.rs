//! Personal variables (env vault spec §5.2). A secret's row holds no value.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::util::now;
use crate::vault::entries::{EntryKind, EnvEntry};
use crate::{Error, Result};

const ENTRY_COLUMNS: &str = "name, kind, value, created_at, updated_at";

fn entry_from_row(row: &SqliteRow) -> Result<EnvEntry> {
    let kind = EntryKind::parse(&row.try_get::<String, _>("kind")?)?;
    Ok(EnvEntry {
        name: row.try_get("name")?,
        kind,
        // A secret's row holds no value; never hand one out even if a row did.
        value: match kind {
            EntryKind::Secret => None,
            EntryKind::Plain => row.try_get("value")?,
        },
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn not_found(name: &str) -> Error {
    Error::NotFound(format!("no variable named {name}"))
}

impl Store {
    /// Every personal variable, by name.
    pub async fn list_env_entries(&self) -> Result<Vec<EnvEntry>> {
        let rows = sqlx::query(&format!(
            "SELECT {ENTRY_COLUMNS} FROM env_entries ORDER BY name COLLATE NOCASE"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(entry_from_row).collect()
    }

    /// The variable named `name`, in any case.
    pub async fn find_env_entry(&self, name: &str) -> Result<Option<EnvEntry>> {
        let row = sqlx::query(&format!(
            "SELECT {ENTRY_COLUMNS} FROM env_entries WHERE name = ?"
        ))
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(entry_from_row).transpose()
    }

    /// Adds a variable; `value` is `None` for a secret. A name taken in any case is a `Conflict`.
    pub async fn add_env_entry(
        &self,
        name: &str,
        kind: EntryKind,
        value: Option<&str>,
    ) -> Result<EnvEntry> {
        let ts = now();
        let added = sqlx::query(
            "INSERT INTO env_entries (name, kind, value, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(name)
        .bind(kind.as_str())
        .bind(value)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await;
        if let Err(sqlx::Error::Database(db)) = &added {
            if db.is_unique_violation() {
                return Err(Error::Conflict(format!(
                    "A variable named {name} is already in Personal."
                )));
            }
        }
        added?;
        self.find_env_entry(name)
            .await?
            .ok_or_else(|| not_found(name))
    }

    /// Sets a plain variable's value, or marks a secret's as replaced (`value` `None`).
    pub async fn set_env_value(&self, name: &str, value: Option<&str>) -> Result<EnvEntry> {
        let done = sqlx::query("UPDATE env_entries SET value = ?, updated_at = ? WHERE name = ?")
            .bind(value)
            .bind(now())
            .bind(name)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_found(name));
        }
        self.find_env_entry(name)
            .await?
            .ok_or_else(|| not_found(name))
    }

    pub async fn remove_env_entry(&self, name: &str) -> Result<()> {
        let done = sqlx::query("DELETE FROM env_entries WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(not_found(name));
        }
        Ok(())
    }
}

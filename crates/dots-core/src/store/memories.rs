//! The `memories` rows: what each dot remembers (dot memory spec sections 2 and 6). A new memory
//! is checked against the dot's limit and its other memories in the statement that inserts it,
//! and a changed text against the others in the statement that changes it, so a run and the
//! person writing at once cannot pass the limit or store one fact twice.

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::model::{Memory, MemorySource};
use crate::util::{new_id, now};
use crate::{Error, Result};

const COLUMNS: &str = "id, dot_id, text, pinned, source, run_id, created_at, updated_at";

/// The text of `column` as `memory::same_key` compares it: without spaces, tabs and line ends,
/// ASCII letters in lower case.
fn key_of(column: &str) -> String {
    format!(
        "lower(replace(replace(replace(replace({column}, ' ', ''), char(9), ''), char(10), ''), \
         char(13), ''))"
    )
}

/// What `add_memory` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    Saved(Memory),
    /// The dot remembers the same text already.
    Duplicate,
    /// The dot has `max` memories.
    Full,
}

fn memory_from_row(row: &SqliteRow) -> Result<Memory> {
    Ok(Memory {
        id: row.try_get("id")?,
        dot_id: row.try_get("dot_id")?,
        text: row.try_get("text")?,
        pinned: row.try_get::<i64, _>("pinned")? != 0,
        source: MemorySource::parse(&row.try_get::<String, _>("source")?)?,
        run_id: row.try_get("run_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

impl Store {
    /// The dot's memories, pinned first, then newest first.
    pub async fn list_memories(&self, dot_id: &str) -> Result<Vec<Memory>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM memories WHERE dot_id = ? \
             ORDER BY pinned DESC, created_at DESC, id DESC"
        ))
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(memory_from_row).collect()
    }

    pub async fn get_memory(&self, id: &str) -> Result<Memory> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM memories WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("memory {id}")))?;
        memory_from_row(&row)
    }

    /// Stores a memory of the dot, unless the dot has `max` already or remembers the same text
    /// (`key` is `memory::same_key` of `text`). Both are checked in the insert's own statement.
    pub async fn add_memory(
        &self,
        dot_id: &str,
        text: &str,
        key: &str,
        source: MemorySource,
        run_id: Option<&str>,
        max: usize,
    ) -> Result<Added> {
        let id = new_id();
        let ts = now();
        let res = sqlx::query(&format!(
            "INSERT INTO memories (id, dot_id, text, pinned, source, run_id, created_at, \
             updated_at) SELECT ?, ?, ?, 0, ?, ?, ?, ? WHERE \
             (SELECT COUNT(*) FROM memories WHERE dot_id = ?) < ? AND NOT EXISTS \
             (SELECT 1 FROM memories WHERE dot_id = ? AND {} = ?)",
            key_of("text")
        ))
        .bind(&id)
        .bind(dot_id)
        .bind(text)
        .bind(source.as_str())
        .bind(run_id)
        .bind(&ts)
        .bind(&ts)
        .bind(dot_id)
        .bind(max as i64)
        .bind(dot_id)
        .bind(key)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 1 {
            return self.get_memory(&id).await.map(Added::Saved);
        }
        let duplicate: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM memories WHERE dot_id = ? AND {} = ?",
            key_of("text")
        ))
        .bind(dot_id)
        .bind(key)
        .fetch_one(&self.pool)
        .await?;
        Ok(if duplicate > 0 {
            Added::Duplicate
        } else {
            Added::Full
        })
    }

    /// Changes the memory's text (`text` with its `key`) and/or its pin. `None` when the new
    /// text is one of the dot's other memories already; checked in the update's own statement.
    pub async fn update_memory(
        &self,
        id: &str,
        text: Option<(&str, &str)>,
        pinned: Option<bool>,
    ) -> Result<Option<Memory>> {
        let res = sqlx::query(&format!(
            "UPDATE memories SET text = COALESCE(?, text), pinned = COALESCE(?, pinned), \
             updated_at = ? WHERE id = ? AND (? IS NULL OR NOT EXISTS \
             (SELECT 1 FROM memories m WHERE m.dot_id = memories.dot_id AND m.id <> memories.id \
             AND {} = ?))",
            key_of("m.text")
        ))
        .bind(text.map(|(t, _)| t))
        .bind(pinned.map(i64::from))
        .bind(now())
        .bind(id)
        .bind(text.map(|(_, k)| k))
        .bind(text.map(|(_, k)| k))
        .execute(&self.pool)
        .await?;
        let memory = self.get_memory(id).await?;
        Ok((res.rows_affected() == 1).then_some(memory))
    }

    /// Deletes the memory; the row as it was. `NotFound` for an unknown id.
    pub async fn delete_memory(&self, id: &str) -> Result<Memory> {
        let memory = self.get_memory(id).await?;
        sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(memory)
    }
}

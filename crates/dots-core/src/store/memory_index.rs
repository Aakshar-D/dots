//! The index memories are ranked with (memory spec section 7; migration 0060): `memory_docs`
//! and its full-text table `memory_fts`. A cache: `sync_memory_index` brings the rows of one
//! owner (a dot, or a project) up to its memories as they are now, and writes nothing when
//! nothing changed. Changes are made under a write lock taken first, as `store/search.rs`
//! does, so reading the index's settings cannot fail on another writer.

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};
use sqlx::Row;

use super::Store;
use crate::model::{Memory, MemoryScope};
use crate::util::now;
use crate::Result;

/// The most rows one ranking reads: more than any dot and project have together.
const RANK_ROWS: i64 = 600;

/// What a memory was indexed with: a change of its text or its files changes it.
fn stamp(memory: &Memory) -> String {
    let mut hasher = Sha256::new();
    hasher.update(memory.text.as_bytes());
    for file in &memory.files {
        hasher.update([0]);
        hasher.update(file.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Store {
    /// Brings the index's rows of `owner` in `scope` up to `memories`: a memory that is new
    /// or changed is indexed, one that is gone leaves the index. A memory that only changed
    /// keeps when it was last recalled. Nothing is written when nothing changed.
    pub async fn sync_memory_index(
        &self,
        scope: MemoryScope,
        owner: &str,
        memories: &[Memory],
    ) -> Result<()> {
        let rows = sqlx::query(
            "SELECT doc_id, memory_id, stamp FROM memory_docs WHERE scope = ? AND owner = ?",
        )
        .bind(scope.as_str())
        .bind(owner)
        .fetch_all(&self.pool)
        .await?;
        let mut indexed: HashMap<String, (i64, String)> = HashMap::new();
        for row in &rows {
            indexed.insert(
                row.try_get("memory_id")?,
                (row.try_get("doc_id")?, row.try_get("stamp")?),
            );
        }
        let current: HashSet<&str> = memories.iter().map(|m| m.id.as_str()).collect();
        let gone: Vec<i64> = indexed
            .iter()
            .filter(|(id, _)| !current.contains(id.as_str()))
            .map(|(_, (doc, _))| *doc)
            .collect();
        // (the memory, the row it has already)
        let changed: Vec<(&Memory, Option<i64>)> = memories
            .iter()
            .filter_map(|m| match indexed.get(&m.id) {
                Some((_, old)) if *old == stamp(m) => None,
                Some((doc, _)) => Some((m, Some(*doc))),
                None => Some((m, None)),
            })
            .collect();
        if gone.is_empty() && changed.is_empty() {
            return Ok(());
        }
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let applied = async {
            for doc in &gone {
                sqlx::query("DELETE FROM memory_fts WHERE rowid = ?")
                    .bind(doc)
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("DELETE FROM memory_docs WHERE doc_id = ?")
                    .bind(doc)
                    .execute(&mut *conn)
                    .await?;
            }
            for (memory, doc) in &changed {
                let doc = match doc {
                    Some(doc) => {
                        sqlx::query("DELETE FROM memory_fts WHERE rowid = ?")
                            .bind(doc)
                            .execute(&mut *conn)
                            .await?;
                        sqlx::query("UPDATE memory_docs SET stamp = ? WHERE doc_id = ?")
                            .bind(stamp(memory))
                            .bind(doc)
                            .execute(&mut *conn)
                            .await?;
                        *doc
                    }
                    None => sqlx::query(
                        "INSERT INTO memory_docs (scope, owner, memory_id, stamp) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(scope.as_str())
                    .bind(owner)
                    .bind(&memory.id)
                    .bind(stamp(memory))
                    .execute(&mut *conn)
                    .await?
                    .last_insert_rowid(),
                };
                sqlx::query("INSERT INTO memory_fts (rowid, text, files) VALUES (?, ?, ?)")
                    .bind(doc)
                    .bind(&memory.text)
                    .bind(memory.files.join("\n"))
                    .execute(&mut *conn)
                    .await?;
            }
            // The rows of dots that are gone go with any change: nothing else removes them.
            sqlx::query(
                "DELETE FROM memory_fts WHERE rowid IN (SELECT doc_id FROM memory_docs \
                 WHERE scope = 'sol' AND owner NOT IN (SELECT id FROM dots))",
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "DELETE FROM memory_docs WHERE scope = 'sol' AND owner NOT IN \
                 (SELECT id FROM dots)",
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query("COMMIT").execute(&mut *conn).await
        }
        .await;
        if let Err(e) = applied {
            // The connection goes back to the pool: it must not keep the transaction open.
            if sqlx::query("ROLLBACK").execute(&mut *conn).await.is_err() {
                conn.close_on_drop();
            }
            return Err(e.into());
        }
        Ok(())
    }

    /// The indexed memories of the dot `dot_id` and of the project `project` that match the
    /// full-text query `matching` (`memory::rank::fts_query`), each with its scope, its id
    /// and its bm25 score, higher for a better match; a word in the text counts twice a word
    /// in a file's path.
    pub async fn rank_memories(
        &self,
        matching: &str,
        dot_id: &str,
        project: Option<&str>,
    ) -> Result<Vec<(MemoryScope, String, f64)>> {
        let rows = sqlx::query(
            "SELECT d.scope AS scope, d.memory_id AS memory_id, \
             -bm25(memory_fts, 1.0, 0.5) AS score \
             FROM memory_fts JOIN memory_docs d ON d.doc_id = memory_fts.rowid \
             WHERE memory_fts MATCH ? AND ((d.scope = 'sol' AND d.owner = ?) \
                 OR (d.scope = 'project' AND d.owner = ?)) \
             ORDER BY bm25(memory_fts, 1.0, 0.5) LIMIT ?",
        )
        .bind(matching)
        .bind(dot_id)
        .bind(project)
        .bind(RANK_ROWS)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    MemoryScope::parse(&row.try_get::<String, _>("scope")?)?,
                    row.try_get("memory_id")?,
                    row.try_get("score")?,
                ))
            })
            .collect()
    }

    /// Notes that `recall` answered with the memories `ids` of `owner` in `scope`, now. A
    /// memory the index does not know yet (it was read by its id, or listed, before any
    /// search) gets its row here, with an empty stamp, so the next search indexes it and the
    /// recall is not lost.
    pub async fn mark_memories_recalled(
        &self,
        scope: MemoryScope,
        owner: &str,
        ids: &[String],
    ) -> Result<()> {
        let ts = now();
        for id in ids {
            sqlx::query(
                "INSERT INTO memory_docs (scope, owner, memory_id, stamp, last_recalled_at) \
                 VALUES (?, ?, ?, '', ?) ON CONFLICT (scope, owner, memory_id) \
                 DO UPDATE SET last_recalled_at = excluded.last_recalled_at",
            )
            .bind(scope.as_str())
            .bind(owner)
            .bind(id)
            .bind(&ts)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// The ids of the memories of `owner` in `scope` that `recall` ever answered with.
    pub async fn recalled_memories(
        &self,
        scope: MemoryScope,
        owner: &str,
    ) -> Result<HashSet<String>> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT memory_id FROM memory_docs \
             WHERE scope = ? AND owner = ? AND last_recalled_at IS NOT NULL",
        )
        .bind(scope.as_str())
        .bind(owner)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().collect())
    }
}

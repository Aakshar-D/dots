//! Search over `search_docs` and its full-text index `search_fts` (migration 0037, search spec
//! section 3): the hits of one account's workspace, best first by bm25, with a snippet each.
//! A run's result and the replies it wrote are one hit: the best of them. The changes waiting
//! in `search_queue` go into the index before each search and when the store opens.

use std::collections::HashSet;

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::search::{
    fts_query, SearchHit, SearchKind, SearchQuery, SearchTarget, HIT_END, HIT_START,
};
use crate::Result;

/// How many words of context a snippet has, the matches included.
const SNIPPET_WORDS: i64 = 16;
/// The rows read for each hit given: a run's replies fold into one hit, so a few spare rows
/// keep the limit filled.
const ROWS_PER_HIT: u32 = 4;

fn hit_from_row(row: &SqliteRow) -> Result<SearchHit> {
    let kind = SearchKind::parse(&row.try_get::<String, _>("kind")?)?;
    let ref_id: String = row.try_get("ref_id")?;
    let dot_id: Option<String> = row.try_get("dot_id")?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let root_run_id: Option<String> = row.try_get("root_run_id")?;
    let root_chat = row.try_get::<Option<String>, _>("root_trigger")?.as_deref() == Some("chat");
    let target = match kind {
        SearchKind::Memory => SearchTarget::Memory {
            dot_id: dot_id.clone().unwrap_or_default(),
            memory_id: ref_id,
        },
        SearchKind::Intent => SearchTarget::Intent { intent_id: ref_id },
        SearchKind::Topic => SearchTarget::Topic {
            dot_id: dot_id.clone().unwrap_or_default(),
            root_run_id: root_run_id.unwrap_or(ref_id),
            run_id: None,
        },
        // A note of Dots' own has no run: its dot's conversation.
        _ => match (run_id, root_run_id) {
            (Some(run_id), Some(root)) if root_chat => SearchTarget::Topic {
                dot_id: dot_id.clone().unwrap_or_default(),
                root_run_id: root,
                run_id: Some(run_id),
            },
            (Some(run_id), Some(_)) => SearchTarget::Run { run_id },
            _ => SearchTarget::Dot {
                dot_id: dot_id.clone().unwrap_or_default(),
            },
        },
    };
    Ok(SearchHit {
        kind,
        dot_id,
        dot_name: row.try_get("dot_name")?,
        target,
        at: row.try_get("at")?,
        snippet: row.try_get("snippet")?,
    })
}

impl Store {
    /// The hits of `query` among the dots and goals of the workspace `workspace_id` of
    /// `org_id` (`None` is Personal), best first. Nothing for a query with no word.
    pub async fn search(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
        query: &SearchQuery,
    ) -> Result<Vec<SearchHit>> {
        let Some(matching) = fts_query(&query.text) else {
            return Ok(Vec::new());
        };
        self.sync_search_index().await?;
        let limit = query.limit();
        let kinds = if query.kinds.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = query
                .kinds
                .iter()
                .map(|k| format!("'{}'", k.as_str()))
                .collect();
            format!("AND s.kind IN ({})", list.join(", "))
        };
        let sql = format!(
            "SELECT s.kind AS kind, s.ref_id AS ref_id, s.run_id AS run_id, s.at AS at, \
             COALESCE(s.dot_id, i.dot_id) AS dot_id, d.name AS dot_name, \
             r.root_run_id AS root_run_id, root.trigger_kind AS root_trigger, \
             snippet(search_fts, 0, ?, ?, '…', {SNIPPET_WORDS}) AS snippet \
             FROM search_fts \
             JOIN search_docs s ON s.doc_id = search_fts.rowid \
             LEFT JOIN intents i ON s.kind = 'intent' AND i.id = s.ref_id \
             LEFT JOIN dots d ON d.id = COALESCE(s.dot_id, i.dot_id) \
             LEFT JOIN runs r ON r.id = s.run_id \
             LEFT JOIN runs root ON root.id = r.root_run_id \
             WHERE search_fts MATCH ? \
             AND CASE WHEN s.kind = 'intent' THEN i.org_id IS ? AND i.workspace_id = ? \
                 ELSE d.org_id IS ? AND d.workspace_id = ? END \
             AND (? IS NULL OR COALESCE(s.dot_id, i.dot_id) = ?) \
             {kinds} \
             ORDER BY bm25(search_fts), s.at DESC LIMIT ?"
        );
        let rows = sqlx::query(&sql)
            .bind(HIT_START.to_string())
            .bind(HIT_END.to_string())
            .bind(&matching)
            .bind(org_id)
            .bind(workspace_id)
            .bind(org_id)
            .bind(workspace_id)
            .bind(&query.dot_id)
            .bind(&query.dot_id)
            .bind(i64::from(limit * ROWS_PER_HIT))
            .fetch_all(&self.pool)
            .await?;
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for row in &rows {
            let hit = hit_from_row(row)?;
            // A run's result and its replies: the best one stands for the run.
            if hit.kind == SearchKind::Reply {
                let run: Option<String> = row.try_get("run_id")?;
                if !seen.insert(run) {
                    continue;
                }
            }
            hits.push(hit);
            if hits.len() == limit as usize {
                break;
            }
        }
        Ok(hits)
    }

    /// Brings the index up to the copies (migration 0037): what waits in `search_queue` goes
    /// into `search_fts`, removals first (a removed copy's id may be a new copy's now), then
    /// the new copies, and the queue is emptied. The write lock is taken before anything is
    /// read, so the index's settings are read under it and another writer cannot make the
    /// write fail at once.
    pub(crate) async fn sync_search_index(&self) -> Result<()> {
        let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM search_queue)")
            .fetch_one(&self.pool)
            .await?;
        if !waiting {
            return Ok(());
        }
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let applied = async {
            sqlx::query(
                "INSERT INTO search_fts (search_fts, rowid, body) \
                 SELECT 'delete', doc_id, body FROM search_queue WHERE op = 'delete' ORDER BY seq",
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "INSERT INTO search_fts (rowid, body) SELECT d.doc_id, d.body \
                 FROM search_queue q JOIN search_docs d ON d.doc_id = q.doc_id \
                 WHERE q.op = 'insert' ORDER BY q.seq",
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query("DELETE FROM search_queue")
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
}

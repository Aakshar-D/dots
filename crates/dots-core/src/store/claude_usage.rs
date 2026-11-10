//! What Claude Code's own transcripts say (usage tab spec sections 5 and 8): each transcript
//! file read so far, and each assistant message once.

use std::collections::{HashMap, HashSet};

use sqlx::Row;

use super::Store;
use crate::claude_usage::{ClaudeProject, ClaudeTotals, TranscriptMessage};
use crate::Result;

/// How far a transcript file has been read: `offset` is the byte after its last complete line
/// read; `mtime_ms` is its modification time in Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFile {
    pub path: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub offset: u64,
}

/// Whether a message is Dots': marked so when it was read, or its session is on a run now
/// (usage tab spec section 4).
const WITH_DOTS: &str = "WITH m AS (SELECT *, (dots = 1 OR session_id IN \
     (SELECT session_id FROM runs WHERE session_id IS NOT NULL)) AS is_dots \
     FROM claude_messages WHERE ts >= ?)";

const SUMS: &str = "COUNT(*) AS messages, COUNT(DISTINCT session_id) AS sessions, \
     COALESCE(SUM(input_tokens), 0) AS input_tokens, \
     COALESCE(SUM(cache_creation_tokens), 0) AS cache_creation_tokens, \
     COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens, \
     COALESCE(SUM(output_tokens), 0) AS output_tokens";

fn totals(row: &sqlx::sqlite::SqliteRow) -> Result<ClaudeTotals> {
    Ok(ClaudeTotals {
        messages: row.try_get::<i64, _>("messages")? as u32,
        sessions: row.try_get::<i64, _>("sessions")? as u32,
        input_tokens: row.try_get("input_tokens")?,
        cache_creation_tokens: row.try_get("cache_creation_tokens")?,
        cache_read_tokens: row.try_get("cache_read_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
    })
}

impl Store {
    /// Every transcript file read so far, by path.
    pub async fn transcript_files(&self) -> Result<HashMap<String, TranscriptFile>> {
        let rows = sqlx::query("SELECT path, size, mtime_ms, offset FROM claude_transcript_files")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                let file = TranscriptFile {
                    path: row.try_get("path")?,
                    size: row.try_get::<i64, _>("size")? as u64,
                    mtime_ms: row.try_get("mtime_ms")?,
                    offset: row.try_get::<i64, _>("offset")? as u64,
                };
                Ok((file.path.clone(), file))
            })
            .collect()
    }

    /// Every Claude session id Dots stored on a run.
    pub async fn run_session_ids(&self) -> Result<HashSet<String>> {
        let rows = sqlx::query("SELECT DISTINCT session_id FROM runs WHERE session_id IS NOT NULL")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| Ok(row.try_get("session_id")?))
            .collect()
    }

    /// Stores what one read of `file` found, in one transaction: each message once per
    /// (message id, request id), keeping the largest output count seen and a Dots mark once
    /// set; then how far the file has been read.
    pub async fn save_transcript_read(
        &self,
        file: &TranscriptFile,
        messages: &[(TranscriptMessage, bool)],
        scanned_at: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for (m, dots) in messages {
            sqlx::query(
                "INSERT INTO claude_messages (message_id, request_id, ts, session_id, cwd, model, \
                 input_tokens, cache_creation_tokens, cache_read_tokens, output_tokens, dots) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(message_id, request_id) DO UPDATE SET \
                 output_tokens = MAX(output_tokens, excluded.output_tokens), \
                 dots = MAX(dots, excluded.dots)",
            )
            .bind(&m.message_id)
            .bind(&m.request_id)
            .bind(&m.ts)
            .bind(&m.session_id)
            .bind(&m.cwd)
            .bind(&m.model)
            .bind(m.input_tokens)
            .bind(m.cache_creation_tokens)
            .bind(m.cache_read_tokens)
            .bind(m.output_tokens)
            .bind(*dots as i64)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO claude_transcript_files (path, size, mtime_ms, offset, scanned_at) \
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(path) DO UPDATE SET size = excluded.size, \
             mtime_ms = excluded.mtime_ms, offset = excluded.offset, \
             scanned_at = excluded.scanned_at",
        )
        .bind(&file.path)
        .bind(file.size as i64)
        .bind(file.mtime_ms)
        .bind(file.offset as i64)
        .bind(scanned_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Forgets the files no longer there (their messages stay).
    pub async fn forget_transcript_files(&self, paths: &[String]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for p in paths {
            sqlx::query("DELETE FROM claude_transcript_files WHERE path = ?")
                .bind(p)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the messages sent before `before`.
    pub async fn prune_claude_messages(&self, before: &str) -> Result<u64> {
        Ok(sqlx::query("DELETE FROM claude_messages WHERE ts < ?")
            .bind(before)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    /// The person's own messages since `since`, then Dots'.
    pub async fn claude_totals(&self, since: &str) -> Result<(ClaudeTotals, ClaudeTotals)> {
        let rows = sqlx::query(&format!(
            "{WITH_DOTS} SELECT is_dots, {SUMS} FROM m GROUP BY is_dots"
        ))
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        let (mut own, mut dots) = (ClaudeTotals::default(), ClaudeTotals::default());
        for row in &rows {
            if row.try_get::<i64, _>("is_dots")? != 0 {
                dots = totals(row)?;
            } else {
                own = totals(row)?;
            }
        }
        Ok((own, dots))
    }

    /// Each project's messages since `since` (the person's by cwd, all Dots' as one row with no
    /// cwd), heaviest by tokens first, at most `limit`.
    pub async fn claude_projects(&self, since: &str, limit: u32) -> Result<Vec<ClaudeProject>> {
        let rows = sqlx::query(&format!(
            "{WITH_DOTS} SELECT CASE WHEN is_dots THEN NULL ELSE cwd END AS project, {SUMS}, \
             SUM(input_tokens + cache_creation_tokens + output_tokens) AS tokens \
             FROM m GROUP BY project ORDER BY tokens DESC, project LIMIT ?"
        ))
        .bind(since)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let cwd: Option<String> = row.try_get("project")?;
                Ok(ClaudeProject {
                    name: cwd
                        .as_deref()
                        .map(crate::claude_usage::folder_name)
                        .unwrap_or_else(|| crate::claude_usage::DOTS_ROW.to_string()),
                    cwd,
                    totals: totals(row)?,
                })
            })
            .collect()
    }

    /// Every message since `since`, Dots' and the person's: the count and the tokens (input,
    /// cache creation and output).
    pub async fn claude_messages_since(&self, since: &str) -> Result<(u32, i64)> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n, \
             COALESCE(SUM(input_tokens + cache_creation_tokens + output_tokens), 0) AS t \
             FROM claude_messages WHERE ts >= ?",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok((row.try_get::<i64, _>("n")? as u32, row.try_get("t")?))
    }
}

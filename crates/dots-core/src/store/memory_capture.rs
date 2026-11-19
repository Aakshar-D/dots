//! What the capture step reads and writes of a run (memory spec section 8; migration 0061):
//! the claim that makes a run's capture happen once, and the compact list of what the run
//! did, counted from its stored tool calls. The transcript's text is never read here.

use sqlx::Row;

use super::Store;
use crate::Result;

/// The most tools and the most files the list of a run's doings names.
const MAX_TOOLS: i64 = 12;
const MAX_FILES: i64 = 20;

/// What a run did, in short: how many tool calls it made, the tools it called most with their
/// counts, and the paths of the files it wrote or edited, as the calls named them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doings {
    pub calls: i64,
    pub tools: Vec<(String, i64)>,
    pub files: Vec<String>,
}

impl Store {
    /// Takes the run for the capture step: `true` the first time, for a run that succeeded,
    /// and `false` ever after. One statement, so two ends of a run cannot both take it.
    pub async fn claim_memory_capture(&self, run_id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE runs SET memory_capture = 'started' \
             WHERE id = ? AND memory_capture IS NULL AND status = 'succeeded'",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Notes that the run's capture ended.
    pub async fn finish_memory_capture(&self, run_id: &str) -> Result<()> {
        sqlx::query("UPDATE runs SET memory_capture = 'done' WHERE id = ?")
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Whether the run was taken for the capture step: `None`, `started` or `done`.
    pub async fn memory_capture_state(&self, run_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT memory_capture FROM runs WHERE id = ?")
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    /// What the run did, from its stored tool calls: their names and counts, and the paths
    /// the file-writing ones were given. Nothing else of an event is read.
    pub async fn run_doings(&self, run_id: &str) -> Result<Doings> {
        // A row whose data is not JSON has no tool: it is not counted.
        let tool = "CASE WHEN json_valid(data) THEN json_extract(data, '$.tool') END";
        let rows = sqlx::query(&format!(
            "SELECT {tool} AS tool, COUNT(*) AS n FROM run_events \
             WHERE run_id = ? AND kind = 'tool_call' AND {tool} IS NOT NULL \
             GROUP BY tool ORDER BY n DESC, tool"
        ))
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        let mut doings = Doings::default();
        for row in &rows {
            let n: i64 = row.try_get("n")?;
            doings.calls += n;
            if (doings.tools.len() as i64) < MAX_TOOLS {
                doings.tools.push((row.try_get("tool")?, n));
            }
        }
        let path = "CASE WHEN json_valid(data) THEN COALESCE(\
                    json_extract(data, '$.input.file_path'), \
                    json_extract(data, '$.input.notebook_path')) END";
        doings.files = sqlx::query_scalar(&format!(
            "SELECT path FROM (SELECT {path} AS path, MIN(seq) AS first FROM run_events \
             WHERE run_id = ? AND kind = 'tool_call' \
             AND {tool} IN ('Edit', 'Write', 'MultiEdit', 'NotebookEdit') \
             GROUP BY path) WHERE typeof(path) = 'text' ORDER BY first LIMIT ?"
        ))
        .bind(run_id)
        .bind(MAX_FILES)
        .fetch_all(&self.pool)
        .await?;
        Ok(doings)
    }
}

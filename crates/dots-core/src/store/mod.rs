use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use crate::Result;

mod approvals;
mod dots;
mod runs;

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
}

impl Store {
    /// Opens (creating if needed) the database file and applies migrations.
    pub async fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Store { pool })
    }
}

pub(crate) fn opt_json(s: Option<String>) -> Result<Option<Value>> {
    Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
}

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection};

use crate::Result;

mod approvals;
mod dots;
mod library;
mod runs;
mod schedules;
mod settings;
mod upgrade;

/// The database file inside the data directory.
pub const DB_FILE: &str = "dots.db";

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
}

impl Store {
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006 and 0007 that need code (`upgrade.rs`).
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
        upgrade::finish_0006(&pool).await?;
        upgrade::finish_0007(&pool).await?;
        Ok(Store { pool })
    }

    /// Closes every connection of this store.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// Writes a consistent copy of the database `db` to `dest` with `VACUUM INTO`, through a
/// connection of its own, so it works after the store has closed (updates spec §7). The
/// database is in WAL mode: a copy of the file alone could miss what the WAL still holds.
/// Fails when `db` does not exist or `dest` does, and creates neither.
pub async fn backup_database(db: &Path, dest: &Path) -> Result<()> {
    let opts = SqliteConnectOptions::new()
        .filename(db)
        .busy_timeout(Duration::from_secs(5));
    let mut conn = opts.connect().await?;
    let done = sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().into_owned())
        .execute(&mut conn)
        .await;
    conn.close().await?;
    done?;
    Ok(())
}

pub(crate) fn opt_json(s: Option<String>) -> Result<Option<Value>> {
    Ok(s.map(|s| serde_json::from_str(&s)).transpose()?)
}

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection};

use crate::Result;

mod approvals;
mod apps;
mod browser;
mod dots;
mod env;
mod follow_ups;
mod instructions;
mod invites;
mod library;
mod limits;
mod memories;
mod messages;
mod orgs;
mod runs;
mod schedules;
mod settings;
mod teams;
#[doc(hidden)]
pub use teams::TEAM_ORG_BACKFILL_SQL;
mod upgrade;
mod watchers;
mod workflow_runs;
mod workspaces;

pub(crate) use upgrade::{auth_scheme, var_name};

/// The database file inside the data directory.
pub const DB_FILE: &str = "dots.db";

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
}

pub use invites::{DraftRow, InviteRow};
pub use limits::LimitReading;
pub use memories::Added;
pub use workspaces::WorkspaceRow;

impl Store {
    /// Opens (creating if needed) the database file and applies migrations, with the parts of
    /// migrations 0006 to 0008 and 0011 that need code (`upgrade.rs`).
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
        upgrade::finish_0008(&pool).await?;
        let store = Store { pool };
        upgrade::finish_0011(&store).await?;
        upgrade::finish_0012(&store.pool).await?;
        upgrade::finish_0015(&store.pool).await?;
        upgrade::finish_0016(&store.pool).await?;
        upgrade::finish_0018(&store.pool).await?;
        Ok(store)
    }

    /// The part of migration 0013 that needs Personal's values (`upgrade::finish_0013`): each
    /// http MCP server made before it whose URL reads a variable gets the origin it reaches now
    /// confirmed in Personal. Runs once, when the runtime starts with its vault; later calls do
    /// nothing. Returns how many origins were recorded.
    pub async fn finish_0013(&self, vault: &crate::vault::Vault) -> Result<usize> {
        upgrade::finish_0013(self, vault).await
    }

    /// Rewrites the database so freed pages lose what they held, and empties the WAL.
    pub(crate) async fn vacuum(&self) -> Result<()> {
        sqlx::query("VACUUM").execute(&self.pool).await?;
        // The result row is (busy, log, checkpointed); busy is 1 when a reader kept the WAL.
        for _ in 0..20 {
            let row = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                .fetch_one(&self.pool)
                .await?;
            if sqlx::Row::try_get::<i64, _>(&row, 0)? == 0 {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Err(crate::Error::Other(
            "the database's write-ahead log could not be emptied".into(),
        ))
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

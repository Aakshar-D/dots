//! App settings, stored in the `settings` table as one JSON value per key (spec §3).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::check_endpoint_url;
use crate::store::Store;
use crate::{Error, Result};

/// The `claude` CLI version the stream-json parser and the permission flow were last checked
/// against. A different installed version shows a warning until it is marked as tested.
pub const TESTED_CLAUDE_VERSION: &str = "2.1.295";
pub const MAX_CONCURRENT_RUNS: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub port: u16,
    pub max_concurrent_runs: usize,
    /// `None` searches PATH.
    pub claude_path: Option<String>,
    pub tested_claude_version: String,
    pub local_endpoints: Vec<String>,
    pub autostart: bool,
    pub schedules_paused: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 47321,
            max_concurrent_runs: 2,
            claude_path: None,
            tested_claude_version: TESTED_CLAUDE_VERSION.to_string(),
            local_endpoints: Vec::new(),
            autostart: true,
            schedules_paused: false,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        let invalid = |m: &str| Err(Error::Invalid(m.to_string()));
        if self.port == 0 {
            return invalid("port must be between 1 and 65535");
        }
        if !(1..=MAX_CONCURRENT_RUNS).contains(&self.max_concurrent_runs) {
            return invalid("max_concurrent_runs must be between 1 and 16");
        }
        if self
            .claude_path
            .as_deref()
            .is_some_and(|p| p.trim().is_empty())
        {
            return invalid("claude_path must not be empty (leave it unset to search PATH)");
        }
        if self.tested_claude_version.trim().is_empty() {
            return invalid("tested_claude_version must not be empty");
        }
        for url in &self.local_endpoints {
            check_endpoint_url(url)?;
        }
        Ok(())
    }

    /// Whether moving from `self` (what the running app started with) to `next` needs a
    /// restart: the port, concurrency and `claude` path are fixed when the runtime starts.
    pub fn restart_required(&self, next: &Settings) -> bool {
        self.port != next.port
            || self.max_concurrent_runs != next.max_concurrent_runs
            || self.claude_path != next.claude_path
    }
}

/// Reads the settings of the database in `data_dir` (created if missing) and closes that
/// store again, so the app can build its start configuration before the runtime opens it.
pub async fn load(data_dir: &Path) -> Result<Settings> {
    let store = Store::open(&data_dir.join(crate::store::DB_FILE)).await?;
    let settings = store.get_settings().await;
    store.close().await;
    settings
}

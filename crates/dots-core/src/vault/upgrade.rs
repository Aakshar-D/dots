//! Bringing a setup from before the vault in (env vault spec §11): secret values stored in the
//! Library move into Personal, the dots that use their servers are granted the new names, and
//! the Env tab offers, once each, to import variables from Dots' Windows environment and to
//! delete update backups made before the vault.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::entries::EntryKind;
use super::grants::Consumer;
use super::literals::has_literals;
use super::service::Vault;
use crate::library::LibraryConfig;
use crate::library_service::process_env;
use crate::{Error, Result};

/// When this data folder first ran with the vault; backups older than that may hold values.
pub const VAULT_SINCE: &str = "vault_since";
/// The import offer was answered.
pub const IMPORT_ANSWERED: &str = "vault_import_answered";
/// The old backups offer was answered.
pub const BACKUPS_ANSWERED: &str = "vault_backups_answered";
/// The data folder's backups folder (the app's `update::BACKUPS_DIR`).
pub const BACKUPS_DIR: &str = "backups";

impl Vault {
    /// Moves every secret value stored in a Library server's config into Personal, and
    /// grants the dots that use the server the names it moved under. The database is vacuumed
    /// after a move, so its freed pages lose the old values. Returns how many servers changed.
    pub async fn move_stored_literals(&self) -> Result<usize> {
        let store = self.store();
        let mut changed = 0;
        for item in store.list_library_items().await? {
            let LibraryConfig::McpServer(config) = &item.config else {
                continue;
            };
            if !has_literals(&item.name, config) {
                continue;
            }
            let (moved_config, names) = self.move_literals(&item.name, config).await?;
            if names.is_empty() {
                continue;
            }
            store
                .update_library_item(&item.id, &LibraryConfig::McpServer(moved_config))
                .await?;
            let key = Consumer::Mcp(item.name.clone()).key();
            for dot in store.dots_using(&item).await? {
                let mut grants = dot.spec.variables.clone();
                let list = grants.0.entry(key.clone()).or_default();
                for name in &names {
                    if !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                        list.push(name.clone());
                    }
                }
                store.set_dot_variables(&dot.id, &grants).await?;
            }
            changed += 1;
        }
        if changed > 0 {
            store.vacuum().await?;
        }
        Ok(changed)
    }

    /// Records when this data folder first ran with the vault, once.
    pub async fn mark_vault_since(&self) -> Result<()> {
        self.store()
            .set_marker(VAULT_SINCE, &json!(crate::util::now()), true)
            .await
    }

    /// The `${VAR}` names Library servers read that Dots' Windows environment sets and Personal
    /// does not have (`env_import_candidates`), until the offer is answered.
    pub async fn import_candidates(&self) -> Result<Vec<String>> {
        let store = self.store();
        if store.marker(IMPORT_ANSWERED).await?.is_some() {
            return Ok(Vec::new());
        }
        let mut names: Vec<String> = Vec::new();
        for item in store.list_library_items().await? {
            if let LibraryConfig::McpServer(config) = &item.config {
                for name in config.variables() {
                    let new = !names.iter().any(|n| n.eq_ignore_ascii_case(&name));
                    if new && process_env(&name).is_some() {
                        names.push(name);
                    }
                }
            }
        }
        let personal = self.personal_values(&names).await?;
        names.retain(|n| !personal.contains(n));
        names.sort();
        Ok(names)
    }

    /// Copies `names` from Dots' Windows environment into Personal as secrets and answers the
    /// offer (`env_import`). Only names `import_candidates` offers are taken.
    pub async fn import(&self, names: &[String]) -> Result<Vec<String>> {
        let offered = self.import_candidates().await?;
        let mut imported = Vec::new();
        for name in names {
            if !offered.contains(name) {
                return Err(Error::Invalid(format!("{name} is not offered for import")));
            }
            if let Some(value) = process_env(name) {
                self.add(name, EntryKind::Secret, &value).await?;
                imported.push(name.clone());
            }
        }
        self.answer(IMPORT_ANSWERED).await?;
        Ok(imported)
    }

    /// Answers the import offer without importing (`env_import_dismiss`).
    pub async fn dismiss_import(&self) -> Result<()> {
        self.answer(IMPORT_ANSWERED).await
    }

    /// The update backups in `data_dir` made before this data folder first ran with the vault
    /// (`env_old_backups`), by file name, until the offer is answered.
    pub async fn old_backups(&self, data_dir: &Path) -> Result<Vec<String>> {
        Ok(self
            .old_backup_paths(data_dir)
            .await?
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect())
    }

    /// Deletes the old backups when `delete`, and answers the offer either way
    /// (`env_old_backups_answer`).
    pub async fn answer_old_backups(&self, data_dir: &Path, delete: bool) -> Result<()> {
        if delete {
            for path in self.old_backup_paths(data_dir).await? {
                std::fs::remove_file(&path).map_err(|e| {
                    Error::Other(format!("could not delete {}: {e}", path.display()))
                })?;
            }
        }
        self.answer(BACKUPS_ANSWERED).await
    }

    async fn old_backup_paths(&self, data_dir: &Path) -> Result<Vec<PathBuf>> {
        let store = self.store();
        if store.marker(BACKUPS_ANSWERED).await?.is_some() {
            return Ok(Vec::new());
        }
        let Some(Value::String(since)) = store.marker(VAULT_SINCE).await? else {
            return Ok(Vec::new());
        };
        let Ok(since) = chrono::DateTime::parse_from_rfc3339(&since) else {
            return Ok(Vec::new());
        };
        let Ok(entries) = std::fs::read_dir(data_dir.join(BACKUPS_DIR)) else {
            return Ok(Vec::new());
        };
        let mut out: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "db"))
            .filter(|e| {
                e.metadata().and_then(|m| m.modified()).is_ok_and(|t| {
                    chrono::DateTime::<chrono::Utc>::from(t) < since.with_timezone(&chrono::Utc)
                })
            })
            .map(|e| e.path())
            .collect();
        out.sort();
        Ok(out)
    }

    async fn answer(&self, key: &str) -> Result<()> {
        self.store()
            .set_marker(key, &json!(crate::util::now()), false)
            .await
    }
}

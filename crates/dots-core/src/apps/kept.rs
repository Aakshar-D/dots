//! The data an app's removal kept (app library spec section 10: the data goes only after a
//! second confirmation). Removing an app that keeps its data leaves `<data>/apps/<app id>/` with
//! its `data.db` and a `removed.json` naming the app and the migrations that ran on the data.
//! The Apps screen lists it, and a new app of the same name whose migrations begin with those,
//! byte for byte, may start on it instead of on an empty database.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::registry::{blocking, remove_all, DATA_FILE};
use super::source::{AppSource, Migration};
use super::Apps;
use crate::library::{AppOrigin, LibraryItem};
use crate::{Error, Result};

/// The note a removal that kept the data leaves beside it.
pub const REMOVED_FILE: &str = "removed.json";

/// What `removed.json` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Removed {
    name: String,
    title: String,
    removed_at: String,
    /// The migrations that ran on the data; `None` when the removed code could not be read.
    migrations: Option<Vec<RanMigration>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RanMigration {
    file: String,
    sha256: String,
}

/// Data a removed app kept, as the Apps screen lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptData {
    /// The removed app's id: its folder below `<data>/apps/`.
    pub id: String,
    pub name: String,
    pub title: String,
    pub removed_at: String,
    /// The size of its database files.
    pub bytes: u64,
}

/// Kept data a new app may start on (the review card's offer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeptOffer {
    pub id: String,
    pub title: String,
    pub removed_at: String,
    pub bytes: u64,
    /// The new app's migrations that would run on it: those after the ones that ran.
    pub migrations: Vec<String>,
}

fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The files of an app's database, as SQLite leaves them.
fn data_files() -> [String; 3] {
    [
        DATA_FILE.to_string(),
        format!("{DATA_FILE}-wal"),
        format!("{DATA_FILE}-shm"),
    ]
}

/// Writes `removed.json` into `dir`: the app is being removed and its data stays. Blocks.
pub(super) fn note_removed(
    dir: &Path,
    name: &str,
    title: &str,
    src: Option<&AppSource>,
) -> Result<()> {
    let note = Removed {
        name: name.to_string(),
        title: title.to_string(),
        removed_at: crate::util::now(),
        migrations: src.map(|s| {
            s.migrations
                .iter()
                .map(|m| RanMigration {
                    file: m.file.clone(),
                    sha256: sha256(&m.sql),
                })
                .collect()
        }),
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(REMOVED_FILE), serde_json::to_vec_pretty(&note)?)?;
    Ok(())
}

/// Moves the database files from `from` to `to`. Blocks.
pub(super) fn move_data(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for file in data_files() {
        if from.join(&file).exists() {
            std::fs::rename(from.join(&file), to.join(&file))?;
        }
    }
    Ok(())
}

/// Every folder below `root` with a `removed.json`, and the migrations it names. Blocks.
fn read_kept(root: &Path) -> Vec<(KeptData, Option<Vec<RanMigration>>)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(text) = std::fs::read(dir.join(REMOVED_FILE)) else {
            continue;
        };
        let Ok(note) = serde_json::from_slice::<Removed>(&text) else {
            continue;
        };
        let bytes = data_files()
            .iter()
            .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
            .map(|m| m.len())
            .sum();
        let kept = KeptData {
            id: entry.file_name().to_string_lossy().into_owned(),
            name: note.name,
            title: note.title,
            removed_at: note.removed_at,
            bytes,
        };
        found.push((kept, note.migrations));
    }
    found
}

/// Whether the migrations `ran` begin `next`'s, file name and text alike.
fn fits(ran: &[RanMigration], next: &[Migration]) -> bool {
    ran.len() <= next.len()
        && ran
            .iter()
            .zip(next)
            .all(|(r, m)| r.file == m.file && r.sha256 == sha256(&m.sql))
}

impl Apps {
    async fn kept_notes(&self) -> Result<Vec<(KeptData, Option<Vec<RanMigration>>)>> {
        let live: HashSet<String> = self.list().await?.into_iter().map(|(i, _)| i.id).collect();
        let root = self.root().to_path_buf();
        let mut found = blocking(move || read_kept(&root)).await?;
        // A row that is still there is a removal under way, not kept data.
        found.retain(|(k, _)| !live.contains(&k.id));
        found.sort_by(|a, b| b.0.removed_at.cmp(&a.0.removed_at));
        Ok(found)
    }

    /// The data removed apps kept, newest first (spec section 10).
    pub async fn kept(&self) -> Result<Vec<KeptData>> {
        Ok(self
            .kept_notes()
            .await?
            .into_iter()
            .map(|(k, _)| k)
            .collect())
    }

    /// Deletes the kept data `id`; `NotFound` when there is none.
    pub async fn delete_kept(&self, id: &str) -> Result<()> {
        if !self.kept().await?.iter().any(|k| k.id == id) {
            return Err(Error::NotFound(format!("kept data {id}")));
        }
        let lock = self.lock(id);
        let _held = lock.lock().await;
        let dir = self.app_dir(id);
        blocking(move || remove_all(&dir)).await?
    }

    /// The newest kept data of an app named as `src` is whose migrations `src` keeps: what the
    /// review card offers to start the new app on.
    pub async fn kept_fit(&self, src: &AppSource) -> Result<Option<KeptOffer>> {
        for (kept, ran) in self.kept_notes().await? {
            let Some(ran) = ran else {
                continue;
            };
            if kept.name == src.manifest.name && fits(&ran, &src.migrations) {
                return Ok(Some(KeptOffer {
                    migrations: src.migrations[ran.len()..]
                        .iter()
                        .map(|m| m.file.clone())
                        .collect(),
                    id: kept.id,
                    title: kept.title,
                    removed_at: kept.removed_at,
                    bytes: kept.bytes,
                }));
            }
        }
        Ok(None)
    }

    /// Adds `src` as a new app on the kept data `kept_id`, which must still fit it: the data
    /// moves to the new app, and the migrations after those that ran on it run. On a failure the
    /// data goes back where it was.
    pub async fn install_kept(&self, src: AppSource, kept_id: &str) -> Result<LibraryItem> {
        let lock = self.lock(kept_id);
        let _held = lock.lock().await;
        match self.kept_fit(&src).await? {
            Some(offer) if offer.id == kept_id => {}
            _ => {
                return Err(Error::Conflict(format!(
                    "the data kept from the removed {} is gone or no longer fits; add the app \
                     without it",
                    src.manifest.title
                )))
            }
        }
        self.install_on(src, AppOrigin::Proposed, Some(kept_id))
            .await
    }
}

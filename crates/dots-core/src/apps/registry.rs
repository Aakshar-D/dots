//! The installed apps (app library spec sections 4, 6, 8 and 10): each is a Library item of
//! kind `app` whose row says which version is active, and a folder in the data folder that
//! holds every accepted version's code and the app's database:
//!
//! ```text
//! <data>/apps/<app id>/code/<version>/   the files of an accepted version
//! <data>/apps/<app id>/data.db           the app's database
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::apps::data;
use crate::apps::source::{self, AppSource};
use crate::events::{Bus, RuntimeEvent};
use crate::library::{AppConfig, AppOrigin, LibraryConfig, LibraryItem, LibraryKind};
use crate::store::Store;
use crate::{Error, Result};

/// The folder below the data folder that holds the apps.
pub const APPS_DIR: &str = "apps";
pub const DATA_FILE: &str = "data.db";

/// An app version: (app id, version).
type VersionKey = (String, u32);

/// The apps. Cheap to clone.
#[derive(Clone)]
pub struct Apps {
    pub(super) store: Store,
    pub(super) bus: Bus,
    root: PathBuf,
    /// One lock per app id: a call, an update and a removal of one app never overlap.
    locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Each loaded version's files, by (app id, version).
    cache: Arc<Mutex<HashMap<VersionKey, Arc<AppSource>>>>,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("blocking task failed: {e}")))
}

impl Apps {
    /// The apps below `root` (`<data>/apps`).
    pub fn new(store: Store, bus: Bus, root: PathBuf) -> Apps {
        Apps {
            store,
            bus,
            root,
            locks: Arc::default(),
            cache: Arc::default(),
        }
    }

    pub fn app_dir(&self, app_id: &str) -> PathBuf {
        self.root.join(app_id)
    }

    pub fn code_dir(&self, app_id: &str, version: u32) -> PathBuf {
        self.app_dir(app_id).join("code").join(version.to_string())
    }

    pub fn data_path(&self, app_id: &str) -> PathBuf {
        self.app_dir(app_id).join(DATA_FILE)
    }

    pub(super) fn lock(&self, app_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .expect("app locks")
            .entry(app_id.to_string())
            .or_default()
            .clone()
    }

    fn changed(&self, item_id: &str) {
        let _ = self.bus.send(RuntimeEvent::LibraryUpdated {
            item_id: item_id.to_string(),
        });
    }

    /// The installed app `name`: `NotFound` when there is none.
    pub async fn get(&self, name: &str) -> Result<(LibraryItem, AppConfig)> {
        let item = self
            .store
            .find_library_item(LibraryKind::App, name)
            .await?
            .ok_or_else(|| Error::NotFound(format!("app {name}")))?;
        let LibraryConfig::App(config) = item.config.clone() else {
            return Err(Error::Other(format!("app {name} has no app config")));
        };
        Ok((item, config))
    }

    /// Every installed app, by name.
    pub async fn list(&self) -> Result<Vec<(LibraryItem, AppConfig)>> {
        Ok(self
            .store
            .list_library_items()
            .await?
            .into_iter()
            .filter_map(|item| match item.config.clone() {
                LibraryConfig::App(c) => Some((item, c)),
                _ => None,
            })
            .collect())
    }

    /// The active version's files of the app `item` (read once per version, then kept).
    pub async fn load(&self, item: &LibraryItem, config: &AppConfig) -> Result<Arc<AppSource>> {
        let key = (item.id.clone(), config.version);
        if let Some(found) = self.cache.lock().expect("app cache").get(&key) {
            return Ok(found.clone());
        }
        let dir = self.code_dir(&item.id, config.version);
        let read = blocking(move || source::read_folder(&dir)).await??;
        if read.manifest.name != item.name {
            return Err(Error::Invalid(format!(
                "the app {}'s code names itself {}",
                item.name, read.manifest.name
            )));
        }
        let read = Arc::new(read);
        self.cache
            .lock()
            .expect("app cache")
            .insert(key, read.clone());
        Ok(read)
    }

    /// Adds a new app at version 1 (spec section 6.2, "Add to library"; section 8, a bundled app):
    /// its code is written, its migrations run on a new database, then its row is stored with
    /// both switches off. A name the Library has is a `Conflict`. On a failure nothing stays.
    pub async fn install(&self, src: AppSource, origin: AppOrigin) -> Result<LibraryItem> {
        let name = src.manifest.name.clone();
        if self
            .store
            .find_library_item(LibraryKind::App, &name)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "the Library already has an app named {name:?}"
            )));
        }
        let config = AppConfig {
            title: src.manifest.title.clone(),
            description: src.manifest.description.clone(),
            version: 1,
            previous_version: None,
            every_dot: false,
            can_start_dots: false,
            origin,
            changed: false,
        };
        let item = self
            .store
            .add_library_item(&name, &LibraryConfig::App(config), None)
            .await?;
        let code = self.code_dir(&item.id, 1);
        let data_path = self.data_path(&item.id);
        let app_dir = self.app_dir(&item.id);
        let written = blocking(move || -> Result<()> {
            write_version(&code, &src)?;
            let mut conn = data::open(&data_path)?;
            data::migrate(&mut conn, &src.migrations)?;
            Ok(())
        })
        .await?;
        if let Err(e) = written {
            let _ = self.store.remove_library_item(&item.id).await;
            let _ = blocking(move || std::fs::remove_dir_all(app_dir)).await;
            return Err(e);
        }
        self.changed(&item.id);
        Ok(item)
    }

    /// Makes `src` the app `name`'s next version (spec sections 5 and 6.2, "Update"): the
    /// migrations newer than its data run first, in one transaction; if one fails, nothing
    /// changes. The version before is kept for "Roll back" only when no migration ran (spec
    /// section 4). `by_proposal` marks a bundled app as the person's (spec section 8). "Can
    /// start dots" goes off when the new code no longer asks for it.
    pub async fn update(
        &self,
        name: &str,
        src: AppSource,
        by_proposal: bool,
    ) -> Result<LibraryItem> {
        if src.manifest.name != name {
            return Err(Error::Invalid(format!(
                "the new code names the app {}, not {name}",
                src.manifest.name
            )));
        }
        let (item, old) = self.get(name).await?;
        let lock = self.lock(&item.id);
        let _held = lock.lock().await;
        let next = old.version + 1;
        let code = self.code_dir(&item.id, next);
        let data_path = self.data_path(&item.id);
        let keep = src.clone();
        let ran = blocking(move || -> Result<usize> {
            let mut conn = data::open(&data_path)?;
            let ran = data::migrate(&mut conn, &keep.migrations)?;
            write_version(&code, &keep)?;
            Ok(ran)
        })
        .await??;
        let config = AppConfig {
            title: src.manifest.title.clone(),
            description: src.manifest.description.clone(),
            version: next,
            previous_version: (ran == 0).then_some(old.version),
            can_start_dots: old.can_start_dots && src.manifest.can_start_dots(),
            changed: old.changed
                || (by_proposal && matches!(old.origin, AppOrigin::Bundled { .. })),
            ..old.clone()
        };
        let item = self
            .store
            .update_library_item(&item.id, &LibraryConfig::App(config.clone()))
            .await?;
        self.sweep(&item.id, &config).await;
        self.changed(&item.id);
        Ok(item)
    }

    /// Goes back to the version before (spec section 10, "Roll back"), offered only while the
    /// update that replaced it ran no migrations.
    pub async fn roll_back(&self, name: &str) -> Result<LibraryItem> {
        let (item, old) = self.get(name).await?;
        let lock = self.lock(&item.id);
        let _held = lock.lock().await;
        let Some(previous) = old.previous_version else {
            return Err(Error::Invalid(format!(
                "the app {name} has no version to roll back to: roll back is offered only when \
                 the last update changed no data schema"
            )));
        };
        let earlier = self.load_version(&item, previous).await?;
        let config = AppConfig {
            title: earlier.manifest.title.clone(),
            description: earlier.manifest.description.clone(),
            version: previous,
            previous_version: None,
            can_start_dots: old.can_start_dots && earlier.manifest.can_start_dots(),
            ..old
        };
        let item = self
            .store
            .update_library_item(&item.id, &LibraryConfig::App(config.clone()))
            .await?;
        self.sweep(&item.id, &config).await;
        self.changed(&item.id);
        Ok(item)
    }

    async fn load_version(&self, item: &LibraryItem, version: u32) -> Result<Arc<AppSource>> {
        let LibraryConfig::App(config) = &item.config else {
            return Err(Error::Other("not an app".into()));
        };
        let config = AppConfig {
            version,
            ..config.clone()
        };
        self.load(item, &config).await
    }

    /// Deletes the code folders of versions the row no longer names.
    async fn sweep(&self, app_id: &str, config: &AppConfig) {
        let keep: Vec<String> = std::iter::once(config.version)
            .chain(config.previous_version)
            .map(|v| v.to_string())
            .collect();
        let code = self.app_dir(app_id).join("code");
        let _ = blocking(move || {
            if let Ok(entries) = std::fs::read_dir(&code) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !keep.contains(&name) {
                        let _ = std::fs::remove_dir_all(entry.path());
                    }
                }
            }
        })
        .await;
        self.cache.lock().expect("app cache").retain(|(id, v), _| {
            id != app_id || *v == config.version || Some(*v) == config.previous_version
        });
    }

    /// Switches "Every dot" (spec section 6).
    pub async fn set_every_dot(&self, name: &str, on: bool) -> Result<LibraryItem> {
        let (item, config) = self.get(name).await?;
        self.store_config(
            &item,
            AppConfig {
                every_dot: on,
                ..config
            },
        )
        .await
    }

    /// Switches "Can start dots" (spec section 7): on only for an app whose code asks for it.
    pub async fn set_can_start_dots(&self, name: &str, on: bool) -> Result<LibraryItem> {
        let (item, config) = self.get(name).await?;
        if on && !self.load(&item, &config).await?.manifest.can_start_dots() {
            return Err(Error::Invalid(format!(
                "the app {name} does not ask to start dots"
            )));
        }
        self.store_config(
            &item,
            AppConfig {
                can_start_dots: on,
                ..config
            },
        )
        .await
    }

    async fn store_config(&self, item: &LibraryItem, config: AppConfig) -> Result<LibraryItem> {
        let item = self
            .store
            .update_library_item(&item.id, &LibraryConfig::App(config))
            .await?;
        self.changed(&item.id);
        Ok(item)
    }

    /// Removes the app `name` (spec section 10): refused (`Conflict`, naming them) while dots
    /// tick it, unless `confirm`; its code goes, and its data too when `delete_data` (the second
    /// confirmation). Kept data is found again by nothing: a new app gets a new id.
    pub async fn remove(&self, name: &str, confirm: bool, delete_data: bool) -> Result<()> {
        let (item, _) = self.get(name).await?;
        let users = self.store.dots_using(&item).await?;
        if !users.is_empty() && !confirm {
            let names: Vec<String> = users.into_iter().map(|d| d.spec.name).collect();
            return Err(Error::Conflict(format!(
                "the app {name:?} is used by {}; removing it takes it out of them",
                names.join(", ")
            )));
        }
        let lock = self.lock(&item.id);
        let _held = lock.lock().await;
        self.store.remove_library_item(&item.id).await?;
        let dir = self.app_dir(&item.id);
        blocking(move || {
            let _ = std::fs::remove_dir_all(dir.join("code"));
            if delete_data {
                let _ = std::fs::remove_dir_all(&dir);
            }
        })
        .await?;
        self.cache
            .lock()
            .expect("app cache")
            .retain(|(id, _), _| *id != item.id);
        self.changed(&item.id);
        Ok(())
    }
}

/// Writes `src`'s files to `dir`, through a sibling `.tmp` folder renamed at the end, so a version
/// folder is whole or absent.
fn write_version(dir: &Path, src: &AppSource) -> Result<()> {
    let tmp = dir.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    for (path, bytes) in src.files() {
        let file = tmp.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, bytes)?;
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&tmp, dir)?;
    Ok(())
}

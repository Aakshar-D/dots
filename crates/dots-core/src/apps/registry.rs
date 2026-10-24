//! The installed apps (app library spec sections 4, 6, 8 and 10): each is a Library item of
//! kind `app` whose row says which version is active, and a folder in the data folder that
//! holds every accepted version's code and the app's database:
//!
//! ```text
//! <data>/apps/<app id>/code/<version>/   the files of an accepted version
//! <data>/apps/<app id>/data.db           the app's database
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use chrono::{Local, SecondsFormat};
use serde::Serialize;
use serde_json::Value;

use crate::apps::call::{self, Actor, Call, CallError, DotRef};
use crate::apps::data;
use crate::apps::engine::Limits;
use crate::apps::source::{self, AppSource};
use crate::events::{Bus, RuntimeEvent};
use crate::library::{AppConfig, AppOrigin, LibraryConfig, LibraryItem, LibraryKind};
use crate::scheduler::{Clock, SystemClock};
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
    /// What `ctx.now` reads.
    clock: Arc<dyn Clock>,
}

/// A logged call (spec section 5): every call of a tool that writes, and of `on_run_end`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppCall {
    pub id: String,
    pub app_id: String,
    pub tool: String,
    pub actor: Actor,
    pub run_id: Option<String>,
    pub input: Value,
    /// The call's `ctx.now`.
    pub now: String,
    /// The ids `ctx.id()` gave.
    pub ids: Vec<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub changes: u64,
    pub took_ms: u64,
    pub at: String,
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
            clock: Arc::new(SystemClock),
        }
    }

    /// `ctx.now` reads `clock` (tests).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Apps {
        self.clock = clock;
        self
    }

    /// Calls `tool` of the app `name` with `input` as `actor` (spec section 5): one call at a
    /// time per app, all or nothing. A call of a tool that writes is logged, and a call that
    /// changed data sends `AppDataChanged`. `Err` is the sentence the caller gets: the rule the
    /// input breaks, the error the code threw, a bound.
    pub async fn call(
        &self,
        name: &str,
        tool: &str,
        input: Value,
        actor: Actor,
    ) -> std::result::Result<Value, String> {
        let (item, config) = match self.get(name).await {
            Ok(found) => found,
            Err(Error::NotFound(_)) => return Err(format!("there is no app {name}")),
            Err(e) => return Err(e.to_string()),
        };
        let src = self.load(&item, &config).await.map_err(|e| e.to_string())?;
        let dots: Vec<DotRef> = self
            .store
            .dots_with_app(&item)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|d| DotRef {
                id: d.id,
                name: d.spec.name,
            })
            .collect();
        let logged =
            matches!(actor, Actor::Dots) || src.manifest.tool(tool).is_some_and(|t| t.writes);
        let now = self
            .clock
            .now()
            .with_timezone(&Local)
            .to_rfc3339_opts(SecondsFormat::Secs, false);
        let lock = self.lock(&item.id);
        let _held = lock.lock().await;
        let path = self.data_path(&item.id);
        let started = std::time::Instant::now();
        let (function, call_input, call_actor, call_now) =
            (tool.to_string(), input.clone(), actor.clone(), now.clone());
        let ran = blocking(move || {
            let conn = data::open(&path).map_err(|e| CallError::Db(e.to_string()))?;
            let conn = Rc::new(RefCell::new(conn));
            call::call(
                &conn,
                Call {
                    manifest: &src.manifest,
                    logic: &src.logic,
                    function: &function,
                    input: &call_input,
                    actor: call_actor,
                    now: call_now,
                    dots,
                    start: None,
                    limits: Limits::default(),
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?;
        let took_ms = started.elapsed().as_millis() as u64;
        if logged {
            let run_id = match &actor {
                Actor::Dot { run_id, .. } => Some(run_id.clone()),
                _ => None,
            };
            let (result, error, ids, changes) = match &ran {
                Ok(out) => (Some(out.result.clone()), None, out.ids.clone(), out.changes),
                Err(e) => (None, Some(e.to_string()), Vec::new(), 0),
            };
            let record = AppCall {
                id: crate::util::new_id(),
                app_id: item.id.clone(),
                tool: tool.to_string(),
                actor: actor.clone(),
                run_id,
                input,
                now,
                ids,
                result,
                error,
                changes,
                took_ms,
                at: crate::util::now(),
            };
            if let Err(e) = self.store.record_app_call(&record).await {
                tracing::warn!(app = %name, tool, "the app call could not be logged: {e}");
            }
        }
        match ran {
            Ok(out) => {
                if out.changes > 0 {
                    let _ = self.bus.send(RuntimeEvent::AppDataChanged {
                        app: name.to_string(),
                    });
                }
                Ok(out.result)
            }
            Err(e) => Err(e.to_string()),
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

    /// The app `name` read again under its lock, which the returned guard holds: a change
    /// computed from a read taken before the lock could undo a change made meanwhile.
    async fn get_locked(
        &self,
        name: &str,
    ) -> Result<(tokio::sync::OwnedMutexGuard<()>, LibraryItem, AppConfig)> {
        let (first, _) = self.get(name).await?;
        let held = self.lock(&first.id).lock_owned().await;
        let (item, config) = self.get(name).await?;
        if item.id != first.id {
            return Err(Error::Conflict(format!(
                "the app {name} was replaced meanwhile; try again"
            )));
        }
        Ok((held, item, config))
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
    /// its row is stored with both switches off, then its code is written and its migrations run
    /// on a new database. A name the Library has is a `Conflict`. On a failure nothing stays.
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

    /// Makes `src` the app `name`'s next version (spec sections 5 and 6.2, "Update"): its code
    /// is written, then the migrations newer than its data run, in one transaction; if either
    /// fails, nothing
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
        let (_held, item, old) = self.get_locked(name).await?;
        let next = old.version + 1;
        let code = self.code_dir(&item.id, next);
        let data_path = self.data_path(&item.id);
        let keep = src.clone();
        // The code first, then the data: a write that fails leaves the data as it was, and a
        // migration that fails takes the new folder away again.
        let ran = blocking(move || -> Result<usize> {
            write_version(&code, &keep)?;
            let migrated = data::open(&data_path)
                .and_then(|mut conn| data::migrate(&mut conn, &keep.migrations));
            if migrated.is_err() {
                let _ = std::fs::remove_dir_all(&code);
            }
            migrated
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
        let (_held, item, old) = self.get_locked(name).await?;
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
        let (_held, item, config) = self.get_locked(name).await?;
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
        let (_held, item, config) = self.get_locked(name).await?;
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
        let (_held, item, _) = self.get_locked(name).await?;
        // "Every dot" gives the app to dots that do not name it: they use it too.
        let users = self.store.dots_with_app(&item).await?;
        if !users.is_empty() && !confirm {
            let names: Vec<String> = users.into_iter().map(|d| d.spec.name).collect();
            return Err(Error::Conflict(format!(
                "the app {name:?} is used by {}; removing it takes it out of them",
                names.join(", ")
            )));
        }
        // The files first: when one cannot be deleted the row stays, and removing again retries.
        let dir = self.app_dir(&item.id);
        blocking(move || -> Result<()> {
            remove_all(&dir.join("code"))?;
            if delete_data {
                remove_all(&dir)?;
            }
            Ok(())
        })
        .await??;
        self.store.remove_library_item(&item.id).await?;
        self.cache
            .lock()
            .expect("app cache")
            .retain(|(id, _), _| *id != item.id);
        self.changed(&item.id);
        Ok(())
    }
}

/// Deletes `path` and all under it; one already gone is not an error.
fn remove_all(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
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

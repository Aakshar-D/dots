//! The apps that come with Dots (app library spec section 8): Projects, whose files are built into
//! Dots from `crates/dots-core/apps/projects/`. A file added to that folder is listed in
//! `PROJECTS_FILES` too; a test checks that the two agree.
//!
//! Dots installs Projects the first time it starts with it, for every dot, and updates it with
//! each release while it is still the bundled one. Once the person accepts a dot's change to it,
//! it is the person's and Dots leaves it alone. Once the person removes it, Dots does not put it
//! back: `<data>/apps/bundled.json` names the bundled apps Dots has installed, and the Apps
//! screen offers to install one again.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::kept::KeptOffer;
use super::propose::check_migrations_kept;
use super::registry::blocking;
use super::source::{self, AppSource};
use super::Apps;
use crate::library::{AppConfig, AppOrigin, LibraryItem};
use crate::{Error, Result};

/// The note of the bundled apps Dots has installed, below `<data>/apps/`.
pub const BUNDLED_FILE: &str = "bundled.json";

/// The name of the Projects app.
pub const PROJECTS: &str = "projects";

/// Projects' files, by their path in the app folder.
const PROJECTS_FILES: &[(&str, &[u8])] = &[
    ("README.md", include_bytes!("../../apps/projects/README.md")),
    ("app.json", include_bytes!("../../apps/projects/app.json")),
    ("logic.js", include_bytes!("../../apps/projects/logic.js")),
    (
        "migrations/0001_projects.sql",
        include_bytes!("../../apps/projects/migrations/0001_projects.sql"),
    ),
    (
        "sample.sql",
        include_bytes!("../../apps/projects/sample.sql"),
    ),
    (
        "tests/assign.test.js",
        include_bytes!("../../apps/projects/tests/assign.test.js"),
    ),
    (
        "tests/tasks.test.js",
        include_bytes!("../../apps/projects/tests/tasks.test.js"),
    ),
    (
        "tests/undo.test.js",
        include_bytes!("../../apps/projects/tests/undo.test.js"),
    ),
    (
        "ui/index.html",
        include_bytes!("../../apps/projects/ui/index.html"),
    ),
];

/// The Dots release the bundled apps come with: `AppOrigin::Bundled`'s `release`.
pub fn release() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Projects' files as Dots holds them.
pub fn projects_files() -> BTreeMap<String, Vec<u8>> {
    PROJECTS_FILES
        .iter()
        .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
        .collect()
}

/// Projects, read and checked as a proposal's folder is (`source::read_files`). Blocks.
pub fn projects() -> Result<AppSource> {
    source::read_files(&projects_files())
}

/// A bundled app that is not installed, as the Apps screen offers it ("Install Projects"),
/// with the data a removal kept when the app's migrations fit it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundledOffer {
    pub name: String,
    pub title: String,
    pub description: String,
    pub kept: Option<KeptOffer>,
}

/// What `bundled.json` holds.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Installed {
    installed: Vec<String>,
}

/// The bundled apps Dots has installed. Blocks.
fn read_installed(root: &Path) -> Installed {
    std::fs::read(root.join(BUNDLED_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Notes that Dots has installed the bundled app `name`. Blocks.
fn note_installed(root: &Path, name: &str) -> Result<()> {
    let mut note = read_installed(root);
    if !note.installed.iter().any(|n| n == name) {
        note.installed.push(name.to_string());
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join(BUNDLED_FILE), serde_json::to_vec_pretty(&note)?)?;
    }
    Ok(())
}

impl Apps {
    /// Brings the bundled Projects in line with this Dots release, at start: installed the first
    /// time, updated while it is the bundled one and the person has not changed it, and left
    /// out once the person removed it. An app of the name that a dot made is left alone.
    pub async fn sync_bundled(&self) -> Result<()> {
        let src = blocking(projects).await??;
        let root = self.root().to_path_buf();
        let note = {
            let root = root.clone();
            blocking(move || read_installed(&root)).await?
        };
        match self.get(PROJECTS).await {
            Ok((item, config)) => self.update_bundled(&item, &config, src).await?,
            Err(Error::NotFound(_)) if note.installed.iter().any(|n| n == PROJECTS) => {
                return Ok(())
            }
            Err(Error::NotFound(_)) => {
                self.install(src, AppOrigin::Bundled { release: release() })
                    .await?;
            }
            Err(e) => return Err(e),
        }
        blocking(move || note_installed(&root, PROJECTS)).await?
    }

    /// Updates the installed bundled app `item` to `src`, this release's, while it is bundled,
    /// unchanged by the person and from another release. The new version keeps the migrations
    /// that ran, as a dot's change must; one that does not is refused and nothing changes.
    async fn update_bundled(
        &self,
        item: &LibraryItem,
        config: &AppConfig,
        src: AppSource,
    ) -> Result<()> {
        let AppOrigin::Bundled { release: had } = &config.origin else {
            return Ok(());
        };
        // A roll back keeps the release, so the version it left is not brought back.
        if config.changed || *had == release() {
            return Ok(());
        }
        let active = self.load(item, config).await?;
        if active.files() == src.files() {
            return Ok(());
        }
        check_migrations_kept(&active, &src, config.version).map_err(|e| {
            Error::Invalid(format!("the bundled {} cannot update: {e}", config.title))
        })?;
        self.update_as(
            &item.name,
            src,
            false,
            Some((&item.id, config.version)),
            Some(AppOrigin::Bundled { release: release() }),
        )
        .await?;
        Ok(())
    }

    /// The bundled apps that are not installed: what the Apps screen offers to install.
    pub async fn bundled_offers(&self) -> Result<Vec<BundledOffer>> {
        match self.get(PROJECTS).await {
            Ok(_) => return Ok(Vec::new()),
            Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        let src = blocking(projects).await??;
        Ok(vec![BundledOffer {
            name: src.manifest.name.clone(),
            title: src.manifest.title.clone(),
            description: src.manifest.description.clone(),
            kept: self.kept_fit(&src).await?,
        }])
    }

    /// Installs the bundled app `name` again ("Install Projects"): on the kept data `kept` when
    /// it names data that still fits it, else with fresh data.
    pub async fn install_bundled(&self, name: &str, kept: Option<&str>) -> Result<LibraryItem> {
        if name != PROJECTS {
            return Err(Error::NotFound(format!(
                "no app named {name} comes with Dots"
            )));
        }
        let src = blocking(projects).await??;
        let origin = AppOrigin::Bundled { release: release() };
        let item = match kept {
            Some(id) => self.install_kept_as(src, id, origin).await?,
            None => self.install(src, origin).await?,
        };
        let root = self.root().to_path_buf();
        blocking(move || note_installed(&root, PROJECTS)).await??;
        Ok(item)
    }
}

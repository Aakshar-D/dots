//! The apps that come with Dots (app library spec section 8): Projects, whose files are built into
//! Dots from `crates/dots-core/apps/projects/`. A file added to that folder is listed in
//! `PROJECTS_FILES` too; a test checks that the two agree.
//!
//! Dots installs Projects the first time it starts with it, for every dot, and updates it with
//! each release while it is still the bundled one. Once the person accepts a dot's change to it,
//! it is the person's and Dots leaves it alone. Once the person removes it, Dots does not put it
//! back: `<data>/apps/bundled.json` names the bundled apps Dots has installed, and the Apps
//! screen offers to install one again.
//!
//! The note also holds a digest of the bundled files Dots last applied to each app. The digest
//! decides whether a release brings anything new: a roll back of a bundled update stays rolled
//! back until the bundled files change, whatever the release string says.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    (
        "ui/main.js",
        include_bytes!("../../apps/projects/ui/main.js"),
    ),
    (
        "ui/projects.css",
        include_bytes!("../../apps/projects/ui/projects.css"),
    ),
    (
        "ui/projects.js",
        include_bytes!("../../apps/projects/ui/projects.js"),
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
    /// The digest (`digest`) of the bundled files Dots last installed or updated each app to.
    #[serde(default)]
    applied: BTreeMap<String, String>,
}

/// The digest of an app's bundled files: a SHA-256 over each path, a NUL, the length of the
/// file's bytes (8 little-endian bytes) and the bytes, in the sorted order of `files()`.
fn digest(src: &AppSource) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in src.files() {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    format!("{:x}", hasher.finalize())
}

/// The note in `root`. Only a missing file is an empty note; any other read error, or a note
/// that does not parse, is an error, so nothing is put back on a note Dots cannot read. Blocks.
fn read_installed(root: &Path) -> Result<Installed> {
    match std::fs::read(root.join(BUNDLED_FILE)) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Installed::default()),
        Err(e) => Err(e.into()),
    }
}

/// Writes the note through a sibling temporary file, renamed over the note, so a crash never
/// leaves half a note. Blocks.
fn write_installed(root: &Path, note: &Installed) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let tmp = root.join(format!("{BUNDLED_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(note)?)?;
    std::fs::rename(&tmp, root.join(BUNDLED_FILE))?;
    Ok(())
}

/// Notes in `root` that Dots has installed the bundled app `name`, and, when given, the digest
/// of the bundled files it applied to it. Blocks.
fn note(root: &Path, name: &str, applied: Option<String>) -> Result<()> {
    let mut note = read_installed(root)?;
    if !note.installed.iter().any(|n| n == name) {
        note.installed.push(name.to_string());
    }
    if let Some(digest) = applied {
        note.applied.insert(name.to_string(), digest);
    }
    write_installed(root, &note)
}

impl Apps {
    /// Brings the bundled Projects in line with this Dots release, at start: installed the first
    /// time, updated while it is the bundled one and the person has not changed it, and left
    /// out once the person removed it. The digest of the bundled files Dots last applied decides
    /// whether a release brings anything new. An app of the name that a dot made is left alone.
    /// A note that cannot be read is an error, and nothing changes.
    pub async fn sync_bundled(&self) -> Result<()> {
        let src = blocking(projects).await??;
        let root = self.root().to_path_buf();
        let known = {
            let root = root.clone();
            blocking(move || read_installed(&root)).await??
        };
        match self.get(PROJECTS).await {
            Ok((item, config)) => {
                // Noted before the update, so an installed app is always noted, even when the
                // update is refused.
                {
                    let root = root.clone();
                    blocking(move || note(&root, PROJECTS, None)).await??;
                }
                let applied = known.applied.get(PROJECTS).cloned();
                self.update_bundled(&item, &config, src, applied).await?
            }
            Err(Error::NotFound(_)) if known.installed.iter().any(|n| n == PROJECTS) => {
                return Ok(())
            }
            Err(Error::NotFound(_)) => {
                let digest = digest(&src);
                self.install(src, AppOrigin::Bundled { release: release() })
                    .await?;
                blocking(move || note(&root, PROJECTS, Some(digest))).await??;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// Updates the installed bundled app `item` to `src`, this release's, while it is bundled
    /// and the person has not changed it. `applied` is the digest of the bundled files Dots last
    /// applied to it: when `src` has that digest, this release brings nothing new and nothing
    /// changes. Without a digest (an install from before digests), `src` is compared with the
    /// active version and, when equal, only the digest is recorded. The new version keeps the
    /// migrations that ran, as a dot's change must; one that does not is refused and nothing
    /// changes.
    async fn update_bundled(
        &self,
        item: &LibraryItem,
        config: &AppConfig,
        src: AppSource,
        applied: Option<String>,
    ) -> Result<()> {
        let AppOrigin::Bundled { .. } = &config.origin else {
            return Ok(());
        };
        if config.changed {
            return Ok(());
        }
        let mine = digest(&src);
        // This release's files are the ones Dots last applied: a roll back stays.
        if applied.as_deref() == Some(mine.as_str()) {
            return Ok(());
        }
        let active = self.load(item, config).await?;
        if active.files() == src.files() {
            return self.record_applied(&item.name, mine).await;
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
        self.record_applied(&item.name, mine).await
    }

    /// Records in the note that Dots applied the bundled files with `digest` to `name`. The note
    /// is written on a blocking thread.
    async fn record_applied(&self, name: &str, digest: String) -> Result<()> {
        let root = self.root().to_path_buf();
        let name = name.to_string();
        blocking(move || note(&root, &name, Some(digest))).await?
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
        let mine = digest(&src);
        let origin = AppOrigin::Bundled { release: release() };
        let item = match kept {
            Some(id) => self.install_kept_as(src, id, origin).await?,
            None => self.install(src, origin).await?,
        };
        // The app is installed either way: a note that cannot be written is logged, not returned.
        if let Err(e) = self.record_applied(PROJECTS, mine).await {
            tracing::warn!(
                app = PROJECTS,
                "installed, but its note could not be written: {e}"
            );
        }
        Ok(item)
    }
}

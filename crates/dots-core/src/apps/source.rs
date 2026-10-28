//! An app's files as a folder holds them (app library spec section 4), read whole and checked
//! before anything runs them: a proposal's folder, an accepted version's, the bundled Projects.

use std::collections::BTreeMap;
use std::path::Path;

use super::engine::{self, Limits};
use super::manifest::Manifest;
use crate::{Error, Result};

/// The most bytes an app's files may hold together (spec section 4).
pub const MAX_APP_BYTES: u64 = 2 * 1024 * 1024;
/// The most files an app may hold.
pub const MAX_APP_FILES: usize = 500;

pub const MANIFEST_FILE: &str = "app.json";
pub const LOGIC_FILE: &str = "logic.js";
pub const UI_DIR: &str = "ui";
pub const UI_INDEX: &str = "ui/index.html";
pub const MIGRATIONS_DIR: &str = "migrations";
pub const SAMPLE_FILE: &str = "sample.sql";
pub const TESTS_DIR: &str = "tests";
pub const README_FILE: &str = "README.md";

/// An app's files, checked by `read_folder`.
#[derive(Debug, Clone, PartialEq)]
pub struct AppSource {
    pub manifest: Manifest,
    pub logic: String,
    /// The files below `ui/`, by their path relative to `ui/` with `/`.
    pub ui: BTreeMap<String, Vec<u8>>,
    /// `migrations/NNNN_name.sql` in order: (number, file name, SQL).
    pub migrations: Vec<Migration>,
    pub sample: Option<String>,
    /// `tests/*.test.js`: (file name, source), sorted by name.
    pub tests: Vec<(String, String)>,
    pub readme: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    pub number: u32,
    pub file: String,
    pub sql: String,
}

impl AppSource {
    /// Every file with its path relative to the app folder, sorted: what the Code tab shows and
    /// what a version folder is written from.
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let manifest = serde_json::to_string_pretty(&self.manifest).unwrap_or_default();
        out.insert(MANIFEST_FILE.to_string(), manifest.into_bytes());
        out.insert(LOGIC_FILE.to_string(), self.logic.clone().into_bytes());
        for (path, bytes) in &self.ui {
            out.insert(format!("{UI_DIR}/{path}"), bytes.clone());
        }
        for m in &self.migrations {
            out.insert(
                format!("{MIGRATIONS_DIR}/{}", m.file),
                m.sql.clone().into_bytes(),
            );
        }
        if let Some(s) = &self.sample {
            out.insert(SAMPLE_FILE.to_string(), s.clone().into_bytes());
        }
        for (name, src) in &self.tests {
            out.insert(format!("{TESTS_DIR}/{name}"), src.clone().into_bytes());
        }
        if let Some(r) = &self.readme {
            out.insert(README_FILE.to_string(), r.clone().into_bytes());
        }
        out
    }

    /// The newest migration's number, 0 for none.
    pub fn schema_version(&self) -> u32 {
        self.migrations.last().map_or(0, |m| m.number)
    }
}

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// Reads the app folder `dir` and checks it (spec section 6.2): the layout, `app.json`, the
/// migrations' names and order, the bounds, and that `logic.js` loads in the sandbox and exports
/// every listed tool and `on_run_end`. `Err(Invalid)` names the first problem. Blocks: call it
/// from `spawn_blocking`.
pub fn read_folder(dir: &Path) -> Result<AppSource> {
    read_files(&walk(dir)?)
}

/// `read_folder` for an app's files held in memory, by their path relative to the app folder
/// with `/` (the apps that come with Dots, `apps::bundled`). The same checks, the bounds
/// included. Blocks.
pub fn read_files(files: &BTreeMap<String, Vec<u8>>) -> Result<AppSource> {
    let total: u64 = files.values().map(|b| b.len() as u64).sum();
    if total > MAX_APP_BYTES {
        return invalid(format!(
            "the app's files hold more than {} MB",
            MAX_APP_BYTES / (1024 * 1024)
        ));
    }
    if files.len() > MAX_APP_FILES {
        return invalid(format!("the app holds more than {MAX_APP_FILES} files"));
    }
    let text = |path: &str| -> Result<String> {
        let bytes = files
            .get(path)
            .ok_or_else(|| Error::Invalid(format!("the app has no {path}")))?;
        String::from_utf8(bytes.clone())
            .map_err(|_| Error::Invalid(format!("{path} is not UTF-8 text")))
    };
    let manifest = Manifest::parse(&text(MANIFEST_FILE)?)?;
    let logic = text(LOGIC_FILE)?;
    if !files.contains_key(UI_INDEX) {
        return invalid(format!("the app has no {UI_INDEX}"));
    }
    let mut ui = BTreeMap::new();
    let mut migrations = Vec::new();
    let mut tests = Vec::new();
    for (path, bytes) in files {
        if let Some(rest) = path.strip_prefix("ui/") {
            ui.insert(rest.to_string(), bytes.clone());
        } else if let Some(name) = path.strip_prefix("migrations/") {
            let number = migration_number(name).ok_or_else(|| {
                Error::Invalid(format!(
                    "migrations/{name}: name migrations NNNN_name.sql, from 0001 (a-z, 0-9, _)"
                ))
            })?;
            migrations.push(Migration {
                number,
                file: name.to_string(),
                sql: text(path)?,
            });
        } else if let Some(name) = path.strip_prefix("tests/") {
            if !name.ends_with(".test.js") || name.contains('/') {
                return invalid(format!("tests/{name}: tests are tests/<name>.test.js"));
            }
            tests.push((name.to_string(), text(path)?));
        } else if ![MANIFEST_FILE, LOGIC_FILE, SAMPLE_FILE, README_FILE].contains(&path.as_str()) {
            return invalid(format!(
                "{path}: an app holds app.json, logic.js, ui/, migrations/, sample.sql, tests/ \
                 and README.md only"
            ));
        }
    }
    migrations.sort_by_key(|m| m.number);
    for (i, m) in migrations.iter().enumerate() {
        if m.number != i as u32 + 1 {
            return invalid(format!(
                "migrations/{}: migrations are numbered 0001, 0002, … with no gap or repeat",
                m.file
            ));
        }
    }
    let sample = files
        .contains_key(SAMPLE_FILE)
        .then(|| text(SAMPLE_FILE))
        .transpose()?;
    let readme = files
        .contains_key(README_FILE)
        .then(|| text(README_FILE))
        .transpose()?;
    check_exports(&manifest, &logic)?;
    Ok(AppSource {
        manifest,
        logic,
        ui,
        migrations,
        sample,
        tests,
        readme,
    })
}

/// `NNNN_name.sql` → NNNN.
fn migration_number(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".sql")?;
    let (digits, rest) = stem.split_at_checked(4)?;
    let label = rest.strip_prefix('_')?;
    let ok = digits.chars().all(|c| c.is_ascii_digit())
        && !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let n: u32 = digits.parse().ok()?;
    (ok && n > 0).then_some(n)
}

/// Every file below `dir` by its relative path with `/`, sorted. Refuses links, hidden names and
/// the bounds.
fn walk(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let meta = std::fs::symlink_metadata(dir)
        .map_err(|_| Error::Invalid(format!("{} is not a folder", dir.display())))?;
    if !meta.is_dir() {
        return invalid(format!("{} is not a folder", dir.display()));
    }
    let mut out = BTreeMap::new();
    let mut total: u64 = 0;
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("{prefix}{name}");
            let kind = entry.file_type()?;
            if name.starts_with('.') {
                return invalid(format!("{rel}: hidden files are not allowed in an app"));
            }
            if kind.is_symlink() {
                return invalid(format!("{rel}: links are not allowed in an app"));
            }
            if kind.is_dir() {
                stack.push((entry.path(), format!("{rel}/")));
                continue;
            }
            let len = entry.metadata()?.len();
            total += len;
            if total > MAX_APP_BYTES {
                return invalid(format!(
                    "the app's files hold more than {} MB",
                    MAX_APP_BYTES / (1024 * 1024)
                ));
            }
            if out.len() >= MAX_APP_FILES {
                return invalid(format!("the app holds more than {MAX_APP_FILES} files"));
            }
            out.insert(rel, std::fs::read(entry.path())?);
        }
    }
    Ok(out)
}

/// Loads `logic` in the sandbox and checks it exports a function for each listed tool and for
/// `on_run_end`.
fn check_exports(manifest: &Manifest, logic: &str) -> Result<()> {
    let wanted: Vec<String> = manifest
        .tools
        .iter()
        .map(|t| t.name.clone())
        .chain(manifest.on_run_end.clone())
        .collect();
    let missing = engine::with_sandbox(Limits::default(), |ctx, judge| {
        let exports = engine::load(ctx, judge, engine::LOGIC_FILE, logic)?;
        Ok(wanted
            .iter()
            .filter(|name| {
                exports
                    .get::<_, rquickjs::Value>(name.as_str())
                    .map(|v| !v.is_function())
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>())
    })
    .map_err(|e| Error::Invalid(format!("logic.js: {e}")))?;
    match missing.first() {
        Some(name) => invalid(format!("logic.js exports no function {name}")),
        None => Ok(()),
    }
}

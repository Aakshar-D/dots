//! The plugin folders of one Claude-engine run (MCP/skills spec section 7): copies under
//! `<plugins_dir>/run-<run_id>/`, each passed to the CLI with `--plugin-dir`, deleted when the
//! run ends.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::frontmatter::remove_field;
use crate::library::LibraryKind;
use crate::{Error, Result};

/// A copy larger than this is left out.
pub const MAX_COPY_BYTES: u64 = 50 * 1024 * 1024;

/// A standalone skill the dot uses: its Library name and its folder.
#[derive(Debug, Clone)]
pub struct KitSkill {
    pub name: String,
    pub dir: PathBuf,
}

/// A plugin the dot uses: its Library name, its folder, and whether its hooks run.
#[derive(Debug, Clone)]
pub struct KitPlugin {
    pub name: String,
    pub dir: PathBuf,
    pub run_hooks: bool,
}

/// An item that did not get into the run, and why (for a run event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    pub kind: LibraryKind,
    pub name: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct RunKit {
    root: PathBuf,
    /// The folders for `--plugin-dir`: `dots-skills` first (if any skill went in), then each
    /// plugin, in the order given.
    pub plugin_dirs: Vec<PathBuf>,
    pub left_out: Vec<LeftOut>,
}

impl RunKit {
    /// Deletes the run's folder. Safe to call twice.
    pub fn cleanup(&self) {
        if let Err(e) = fs::remove_dir_all(&self.root) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("could not delete {}: {e}", self.root.display());
            }
        }
    }
}

/// The files of `src` to copy, as paths relative to it, and their total size. `None` when the
/// total goes past `MAX_COPY_BYTES`. Symbolic links are not followed or copied, and `.git` is
/// left out; `skip` names more paths to leave out.
fn collect(src: &Path, skip: &dyn Fn(&Path) -> bool) -> std::io::Result<Option<Vec<PathBuf>>> {
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in fs::read_dir(src.join(&rel))? {
            let entry = entry?;
            let path = rel.join(entry.file_name());
            if entry.file_name() == ".git" || skip(&path) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                total += entry.metadata()?.len();
                if total > MAX_COPY_BYTES {
                    return Ok(None);
                }
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(Some(files))
}

/// The cleaned text of a copied file. A `SKILL.md`, a command file and an agent file lose
/// `allowed-tools`, and `hooks` unless `run_hooks`; an agent file also loses `permissionMode`
/// and `mcpServers`, always (an agent must not change Dots' permission handling or bring
/// servers past the gateway).
fn cleaned(rel: &Path, bytes: Vec<u8>, run_hooks: bool) -> Vec<u8> {
    let is_md = rel.extension().is_some_and(|e| e == "md");
    let is_skill = rel.file_name().is_some_and(|n| n == "SKILL.md");
    let is_command = is_md && rel.starts_with("commands");
    let is_agent = is_md && rel.starts_with("agents");
    if !(is_skill || is_command || is_agent) {
        return bytes;
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    text = remove_field(&text, "allowed-tools");
    if !run_hooks {
        text = remove_field(&text, "hooks");
    }
    if is_agent {
        text = remove_field(&text, "permissionMode");
        text = remove_field(&text, "mcpServers");
    }
    text.into_bytes()
}

/// Writes the copy's manifest: the original (when it is a JSON object) without `mcpServers`,
/// `monitors` and, unless `run_hooks`, `hooks`; `name` set to the plugin's Library name, so
/// Claude Code calls its skills `<name>:<skill>`. A plugin with no manifest, or an unreadable
/// one, gets `{"name": ...}`.
fn write_manifest(dest: &Path, name: &str, run_hooks: bool) -> std::io::Result<()> {
    let path = dest.join(".claude-plugin").join("plugin.json");
    let mut manifest = match fs::read(&path).map(|b| serde_json::from_slice::<Value>(&b)) {
        Ok(Ok(Value::Object(m))) => m,
        _ => serde_json::Map::new(),
    };
    manifest.remove("mcpServers");
    manifest.remove("monitors");
    if !run_hooks {
        manifest.remove("hooks");
    }
    manifest.insert("name".into(), Value::String(name.into()));
    fs::create_dir_all(path.parent().expect("has a parent"))?;
    fs::write(
        path,
        serde_json::to_vec_pretty(&Value::Object(manifest)).expect("a JSON object"),
    )
}

/// Copies the files `files` of `src` into `dest`, cleaning those that need it.
fn copy_files(src: &Path, dest: &Path, files: &[PathBuf], run_hooks: bool) -> std::io::Result<()> {
    for rel in files {
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, cleaned(rel, fs::read(src.join(rel))?, run_hooks))?;
    }
    Ok(())
}

/// Copies one folder, or says why not. `Err(reason)` leaves nothing behind.
fn copy_folder(
    src: &Path,
    dest: &Path,
    run_hooks: bool,
    skip: &dyn Fn(&Path) -> bool,
) -> std::result::Result<(), String> {
    if !src.is_dir() {
        return Err("folder not found".into());
    }
    let files = match collect(src, skip) {
        Ok(Some(files)) => files,
        Ok(None) => return Err("larger than 50 MB".into()),
        Err(e) => return Err(format!("could not be read: {e}")),
    };
    copy_files(src, dest, &files, run_hooks).map_err(|e| {
        let _ = fs::remove_dir_all(dest);
        format!("could not be copied: {e}")
    })
}

/// Writes the run's plugin folders under `<plugins_dir>/run-<run_id>/` (an old folder of that
/// name is replaced):
///
/// - `dots-skills/`: a generated plugin whose `skills/<name>/` holds a copy of each skill;
/// - `<name>/`: a copy of each plugin, its manifest named `<name>`, without `.mcp.json` (its servers go through the gateway),
///   without `monitors/`, and without `hooks/` unless `run_hooks`.
///
/// Every copied `SKILL.md`, command and agent file loses `allowed-tools`, `hooks` (unless the
/// plugin's `run_hooks`) and, for an agent, `permissionMode` and `mcpServers`. A skill or plugin that is
/// missing, has no `SKILL.md` (a skill), or is over 50 MB is listed in `left_out`. Nothing is
/// created when nothing goes in. This blocks: call it from `spawn_blocking`.
pub fn prepare_run_kit(
    plugins_dir: &Path,
    run_id: &str,
    skills: &[KitSkill],
    plugins: &[KitPlugin],
) -> Result<RunKit> {
    let root = plugins_dir.join(format!("run-{run_id}"));
    let mut kit = RunKit {
        root: root.clone(),
        plugin_dirs: Vec::new(),
        left_out: Vec::new(),
    };
    if skills.is_empty() && plugins.is_empty() {
        return Ok(kit);
    }
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;

    let skills_plugin = root.join("dots-skills");
    let mut copied_skills = 0;
    for skill in skills {
        let mut leave = |reason: String| {
            kit.left_out.push(LeftOut {
                kind: LibraryKind::Skill,
                name: skill.name.clone(),
                reason,
            })
        };
        if skill.dir.is_dir() && !skill.dir.join("SKILL.md").is_file() {
            leave("SKILL.md not found".into());
            continue;
        }
        let dest = skills_plugin.join("skills").join(&skill.name);
        match copy_folder(&skill.dir, &dest, false, &|_| false) {
            Ok(()) => copied_skills += 1,
            Err(reason) => leave(reason),
        }
    }
    if copied_skills > 0 {
        let manifest = json!({
            "name": "dots-skills",
            "description": "The skills this Dots run uses",
            "version": "0.0.0",
        });
        let dir = skills_plugin.join(".claude-plugin");
        let write = fs::create_dir_all(&dir).and_then(|_| {
            fs::write(
                dir.join("plugin.json"),
                serde_json::to_vec_pretty(&manifest).expect("a literal"),
            )
        });
        if let Err(e) = write {
            kit.cleanup();
            return Err(Error::Io(e));
        }
        kit.plugin_dirs.push(skills_plugin);
    }

    for plugin in plugins {
        let dest = root.join(&plugin.name);
        let run_hooks = plugin.run_hooks;
        let skip = move |rel: &Path| {
            let first = rel.components().next().map(|c| c.as_os_str().to_owned());
            let top = rel.components().count() == 1;
            match first.as_deref().and_then(|f| f.to_str()) {
                Some(".mcp.json") => top,
                Some("monitors") => true,
                Some("hooks") => !run_hooks,
                _ => false,
            }
        };
        let copied = copy_folder(&plugin.dir, &dest, run_hooks, &skip).and_then(|()| {
            write_manifest(&dest, &plugin.name, run_hooks).map_err(|e| {
                let _ = fs::remove_dir_all(&dest);
                format!("could not be copied: {e}")
            })
        });
        match copied {
            Ok(()) => kit.plugin_dirs.push(dest),
            Err(reason) => kit.left_out.push(LeftOut {
                kind: LibraryKind::Plugin,
                name: plugin.name.clone(),
                reason,
            }),
        }
    }
    Ok(kit)
}

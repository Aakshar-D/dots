//! Files a watcher stages for a run (watchers spec section 4.2): an email's attachments, saved
//! under Dots' data folder at fetch time (`<root>/<watcher_id>/<run_id>/`) with safe names, at
//! most `MAX_ATTACHMENTS_BYTES` per email in all. An event held outside its watcher's hours
//! (watch hours spec section 4.2) has its files under `<root>/<watcher_id>/held/<held_id>/`
//! until the opening run takes them.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::model::{Run, RunStatus, TriggerKind};
use crate::store::Store;
use crate::Result;

/// Whether the thread rooted at `root_run_id` has a run still to come (queued, running or
/// waiting on an approval).
async fn thread_open(store: &Store, root_run_id: &str) -> Result<bool> {
    Ok(store.list_thread(root_run_id).await?.iter().any(|r| {
        matches!(
            r.status,
            RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval
        )
    }))
}

fn remove_dir(dir: &Path) {
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(dir) {
            tracing::warn!(dir = %dir.display(), "removing staged files failed: {e}");
        }
    }
}

/// Saves `files` into `dir`; nothing (not even the folder) when there are none.
fn stage_into(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes)?;
    }
    Ok(())
}

/// The most an email's attachments may weigh in all; larger ones are listed, not saved.
pub const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;

/// The longest safe name, in characters.
const MAX_NAME_CHARS: usize = 120;

/// The folder, beside a watcher's run folders, of its held events' files.
const HELD: &str = "held";

const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Characters that change the order a name reads in (`invoice\u{202E}fdp.exe` would show as
/// `invoiceexe.pdf`): dropped.
fn reorders(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Whether the name is a reserved Windows device name: its part before the first dot is.
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

/// The name split at its last dot: (stem, ".ext" or "").
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// A name that is safe as a file name on Windows: the part after the last `/` or `\`, without
/// control characters, `<>:"|?*` as `_`, no spaces at the start nor spaces or dots at the end, a reserved device name
/// prefixed with `_`, at most 120 characters keeping the extension; `attachment` when nothing
/// is left.
pub fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out: String = last
        .chars()
        .filter(|c| !c.is_control() && !reorders(*c))
        .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    while out.ends_with([' ', '.']) {
        out.pop();
    }
    let mut out = out.trim_start().to_string();
    if out.is_empty() {
        return "attachment".to_string();
    }
    if reserved(&out) {
        out.insert(0, '_');
    }
    if out.chars().count() > MAX_NAME_CHARS {
        let (stem, ext) = split_extension(&out);
        let keep = MAX_NAME_CHARS.saturating_sub(ext.chars().count());
        out = stem.chars().take(keep).collect::<String>() + ext;
    }
    out
}

/// The names, with ` (2)`, ` (3)`… before the extension of a name equal in any case to an
/// earlier one.
pub fn unique_names(names: &[String]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::new();
    for name in names {
        let mut candidate = name.clone();
        let mut n = 2;
        while taken.iter().any(|t| t.eq_ignore_ascii_case(&candidate)) {
            // The stem gives way, so the name stays within 120 characters.
            let (stem, ext) = split_extension(name);
            let suffix = format!(" ({n}){ext}");
            let keep = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
            candidate = stem.chars().take(keep).collect::<String>() + &suffix;
            n += 1;
        }
        taken.push(candidate);
    }
    taken
}

/// Where a watcher's staged files live.
#[derive(Debug, Clone)]
pub struct WatcherFiles {
    root: PathBuf,
}

impl WatcherFiles {
    pub fn new(root: PathBuf) -> WatcherFiles {
        WatcherFiles { root }
    }

    pub fn stage_dir(&self, watcher_id: &str, run_id: &str) -> PathBuf {
        self.root.join(watcher_id).join(run_id)
    }

    /// Saves `files` (safe names already) for the run.
    pub fn stage(&self, watcher_id: &str, run_id: &str, files: &[(String, Vec<u8>)]) -> Result<()> {
        stage_into(&self.stage_dir(watcher_id, run_id), files)
    }

    /// Where the files of a held event live.
    pub fn held_dir(&self, watcher_id: &str, held_id: &str) -> PathBuf {
        self.root.join(watcher_id).join(HELD).join(held_id)
    }

    /// Saves `files` (safe names already) for a held event.
    pub fn stage_held(
        &self,
        watcher_id: &str,
        held_id: &str,
        files: &[(String, Vec<u8>)],
    ) -> Result<()> {
        stage_into(&self.held_dir(watcher_id, held_id), files)
    }

    /// Removes a held event's files.
    pub fn remove_held(&self, watcher_id: &str, held_id: &str) {
        remove_dir(&self.held_dir(watcher_id, held_id));
    }

    /// Moves the files of the `n`th event of an opening run (`names`, as its held row lists
    /// them) into the run's staging as `<n>-<name>`, and answers each moved file's attachment
    /// entry. A file that is gone, or cannot be moved, is left out with a warning: the held
    /// row's list is best-effort, and the event still goes into the run.
    pub fn take_held_files(
        &self,
        watcher_id: &str,
        held_id: &str,
        names: &[String],
        run_id: &str,
        n: usize,
    ) -> Vec<Value> {
        let from_dir = self.held_dir(watcher_id, held_id);
        let to_dir = self.stage_dir(watcher_id, run_id);
        let mut moved = Vec::new();
        for name in names {
            let from = from_dir.join(name);
            let staged_name = format!("{n}-{name}");
            let outcome = std::fs::metadata(&from).and_then(|meta| {
                std::fs::create_dir_all(&to_dir)?;
                std::fs::rename(&from, to_dir.join(&staged_name))?;
                Ok(meta.len())
            });
            match outcome {
                Ok(size) => moved.push(serde_json::json!({
                    "name": staged_name, "size": size, "path": null, "event": n,
                })),
                Err(e) => tracing::warn!(
                    watcher = watcher_id, held = held_id, file = %name,
                    "a held file could not be moved to its opening run, left out: {e}"
                ),
            }
        }
        moved
    }

    /// Undoes `take_held_files` for a run that was not made: each staged file (`<n>-<name>`)
    /// goes back under its held event, and the run's staging goes.
    pub fn return_held_files(&self, watcher_id: &str, run_id: &str, held: &[(String, Value)]) {
        let staged = self.stage_dir(watcher_id, run_id);
        for (held_id, attachment) in held {
            let Some(staged_name) = attachment["name"].as_str() else {
                continue;
            };
            let n = attachment["event"].as_u64().unwrap_or_default();
            let name = staged_name
                .strip_prefix(&format!("{n}-"))
                .unwrap_or(staged_name);
            let back = self.held_dir(watcher_id, held_id).join(name);
            if let Err(e) = std::fs::rename(staged.join(staged_name), &back) {
                tracing::warn!(watcher = watcher_id, held = %held_id, "returning a held file failed: {e}");
            }
        }
        remove_dir(&staged);
    }

    /// Removes everything staged for the watcher (it was deleted).
    pub fn forget_watcher(&self, watcher_id: &str) {
        remove_dir(&self.root.join(watcher_id));
    }

    /// Copies the staged attachments of a watcher thread's root run into
    /// `<workspace>/attachments/<root id>/`, and answers the root's payload with each copied
    /// attachment's `path`; `None` when the root has nothing staged.
    pub fn copy_into(&self, root: &Run, workspace: &Path) -> Result<Option<Value>> {
        if root.trigger != TriggerKind::Watcher {
            return Ok(None);
        }
        let Some(mut payload) = root.payload.clone() else {
            return Ok(None);
        };
        let Some(watcher_id) = payload["watcher_id"].as_str().map(str::to_string) else {
            return Ok(None);
        };
        let staged = self.stage_dir(&watcher_id, &root.id);
        let Some(attachments) = payload["attachments"].as_array_mut() else {
            return Ok(None);
        };
        if !staged.is_dir() || !attachments.iter().any(|a| a.get("path").is_some()) {
            return Ok(None);
        }
        let dest = workspace.join("attachments").join(&root.id);
        std::fs::create_dir_all(&dest)?;
        for attachment in attachments.iter_mut() {
            if attachment.get("path").is_none() {
                continue;
            }
            let Some(name) = attachment["name"].as_str().map(str::to_string) else {
                continue;
            };
            let from = staged.join(&name);
            if !from.is_file() {
                continue;
            }
            let to = dest.join(&name);
            std::fs::copy(&from, &to)?;
            attachment["path"] = Value::String(to.to_string_lossy().to_string());
        }
        Ok(Some(payload))
    }

    /// Removes a run's copy from its workspace, and `attachments` when nothing else is in it.
    pub fn remove_copy(workspace: &Path, root_run_id: &str) {
        let all = workspace.join("attachments");
        let copy = all.join(root_run_id);
        if !copy.exists() {
            return;
        }
        remove_dir(&copy);
        let empty = std::fs::read_dir(&all).is_ok_and(|mut d| d.next().is_none());
        if empty {
            if let Err(e) = std::fs::remove_dir(&all) {
                tracing::warn!(dir = %all.display(), "removing the attachments folder failed: {e}");
            }
        }
    }

    /// Removes a watcher thread's staged copy once no run of the thread is left to come.
    pub async fn release(&self, store: &Store, run_id: &str) {
        let outcome: Result<()> = async {
            let run = store.get_run(run_id).await?;
            let root = store.get_run(&run.root_run_id).await?;
            if root.trigger != TriggerKind::Watcher || thread_open(store, &root.id).await? {
                return Ok(());
            }
            let payload = root.payload.unwrap_or_default();
            if let Some(watcher_id) = payload["watcher_id"].as_str() {
                remove_dir(&self.stage_dir(watcher_id, &root.id));
            }
            Ok(())
        }
        .await;
        if let Err(e) = outcome {
            tracing::warn!(run = %run_id, "releasing staged files failed: {e}");
        }
    }

    /// Removes what a crash left: the folders of watchers that are gone, of runs that are gone
    /// or whose thread has no run left to come, and of held events no longer held.
    pub async fn sweep(&self, store: &Store) {
        let Ok(watchers) = std::fs::read_dir(&self.root) else {
            return;
        };
        for watcher in watchers.flatten() {
            let id = watcher.file_name().to_string_lossy().to_string();
            if store.get_watcher(&id).await.is_err() {
                remove_dir(&watcher.path());
                continue;
            }
            let Ok(runs) = std::fs::read_dir(watcher.path()) else {
                continue;
            };
            for run in runs.flatten() {
                let run_id = run.file_name().to_string_lossy().to_string();
                if run_id == HELD {
                    self.sweep_held(store, &id, &run.path()).await;
                    continue;
                }
                let keep = match store.get_run(&run_id).await {
                    Ok(r) => thread_open(store, &r.root_run_id).await.unwrap_or(true),
                    Err(_) => false,
                };
                if !keep {
                    remove_dir(&run.path());
                }
            }
        }
    }

    /// Removes the folders under a watcher's `held` whose event is no longer held.
    async fn sweep_held(&self, store: &Store, watcher_id: &str, dir: &Path) {
        let Ok(held) = store.held_events(watcher_id).await else {
            return;
        };
        let Ok(folders) = std::fs::read_dir(dir) else {
            return;
        };
        for folder in folders.flatten() {
            let held_id = folder.file_name().to_string_lossy().to_string();
            if !held.iter().any(|e| e.id == held_id) {
                remove_dir(&folder.path());
            }
        }
    }
}

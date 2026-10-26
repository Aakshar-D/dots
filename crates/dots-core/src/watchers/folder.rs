//! Folder watchers (watchers spec section 4.1): the OS's notifications on a folder, each path
//! settled for `SETTLE` before it wakes the dot once, as created, changed or deleted.
//!
//! What a change counts as is decided at report time and at the settle:
//! - a change reported while the dot acts (`ActingTracker`) is the dot's own and is dropped in
//!   the OS's callback, even when it settles after the dot stopped acting;
//! - partial downloads, temporary files and any path with a part starting with `.` never count
//!   (`skipped`), nor do folders, which are remembered so their removal is not a deletion;
//! - a "change" whose size and modified time are those last reported for the path is a sync
//!   engine's touch (OneDrive re-saving a settled file) and is dropped.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecursiveMode, Watcher as _};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{folder_gone, ActingTracker, Failure, SourceEvent, Watchers};
use crate::model::{FolderConfig, FolderEvent, Watcher};
use crate::scheduler::Clock;

/// How long a path must stay unchanged before it wakes the dot.
pub const SETTLE: Duration = Duration::from_secs(2);

/// How many entries the walk at start remembers as folders, at most.
const MAX_WALK: usize = 20_000;

const SKIPPED_ENDINGS: [&str; 4] = [".crdownload", ".part", ".tmp", ".partial"];
const SKIPPED_STARTS: [&str; 2] = ["~$", ".~lock."];

/// Whether a path under the watched folder never counts: a part starting with `.`, a partial
/// download or a temporary file.
pub fn skipped(relative: &Path) -> bool {
    if relative
        .components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    {
        return true;
    }
    let Some(name) = relative.file_name() else {
        return true;
    };
    let name = name.to_string_lossy().to_lowercase();
    SKIPPED_ENDINGS.iter().any(|e| name.ends_with(e))
        || SKIPPED_STARTS.iter().any(|s| name.starts_with(s))
}

/// Whether the file name matches the watcher's pattern, in any case; an empty pattern matches
/// every name.
pub fn matches_pattern(pattern: &str, name: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    globset::GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map(|g| g.compile_matcher().is_match(name))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Created,
    Changed,
    Removed,
}

/// The kinds a notify event stands for, per path.
fn seen(kind: &EventKind, paths: &[PathBuf]) -> Vec<(Seen, PathBuf)> {
    let all = |s: Seen| paths.iter().map(|p| (s, p.clone())).collect();
    match kind {
        EventKind::Access(_) => Vec::new(),
        EventKind::Create(_) => all(Seen::Created),
        EventKind::Remove(_) => all(Seen::Removed),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => all(Seen::Removed),
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => all(Seen::Created),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            let mut out = Vec::new();
            if let Some(from) = paths.first() {
                out.push((Seen::Removed, from.clone()));
            }
            if let Some(to) = paths.get(1) {
                out.push((Seen::Created, to.clone()));
            }
            out
        }
        _ => all(Seen::Changed),
    }
}

/// The folders under `root` (at most `MAX_WALK` entries looked at).
fn walk_folders(root: &Path, recursive: bool) -> HashSet<PathBuf> {
    let mut folders = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    let mut looked = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            looked += 1;
            if looked > MAX_WALK {
                return folders;
            }
            let path = entry.path();
            if path.is_dir() {
                folders.insert(path.clone());
                if recursive {
                    stack.push(path);
                }
            }
        }
    }
    folders
}

fn modified(meta: &std::fs::Metadata) -> Option<SystemTime> {
    meta.modified().ok()
}

/// Starts watching the folder; the watch lives until `token` is cancelled. An error is the
/// failure to report: a folder that is gone, or anything else the OS refused.
pub(crate) fn start(
    service: Weak<Watchers>,
    watcher: Watcher,
    config: FolderConfig,
    acting: Arc<ActingTracker>,
    clock: Arc<dyn Clock>,
    token: CancellationToken,
) -> Result<(), Failure> {
    let root = PathBuf::from(&config.path);
    if !root.is_dir() {
        return Err(Failure::Permanent(folder_gone(&config.path)));
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<(Seen, PathBuf)>();
    let dot_id = watcher.dot_id.clone();
    let mut os = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            return;
        };
        // Stamped when the OS reports it: the dot's own changes are dropped here.
        if acting.is_acting(&dot_id, clock.now()) {
            return;
        }
        for item in seen(&event.kind, &event.paths) {
            let _ = tx.send(item);
        }
    })
    .map_err(|e| Failure::Transient(format!("Could not watch {}: {e}", config.path)))?;
    let mode = if config.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    if let Err(e) = os.watch(&root, mode) {
        return Err(if root.is_dir() {
            Failure::Transient(format!("Could not watch {}: {e}", config.path))
        } else {
            Failure::Permanent(folder_gone(&config.path))
        });
    }
    tokio::spawn(async move {
        // Kept alive here: dropping it stops the OS's notifications.
        let _os = os;
        let mut folders = walk_folders(&root, config.recursive);
        let mut pending: HashMap<PathBuf, (Seen, Instant)> = HashMap::new();
        let mut reported: HashMap<PathBuf, (u64, Option<SystemTime>)> = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut last_check = Instant::now();
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                Some((kind, path)) = rx.recv() => {
                    let entry = pending.entry(path).or_insert((kind, Instant::now()));
                    entry.1 = Instant::now() + SETTLE;
                }
                _ = tick.tick() => {
                    if last_check.elapsed() >= Duration::from_secs(1) {
                        last_check = Instant::now();
                        if !root.is_dir() {
                            if let Some(service) = service.upgrade() {
                                let failure = Failure::Permanent(folder_gone(&config.path));
                                if let Err(e) = service.note_failure(&watcher.id, failure).await {
                                    tracing::warn!(watcher = %watcher.id, "pausing failed: {e}");
                                }
                            }
                            break;
                        }
                    }
                    let now = Instant::now();
                    let due: Vec<(PathBuf, Seen)> = pending
                        .iter()
                        .filter(|(_, (_, deadline))| *deadline <= now)
                        .map(|(p, (k, _))| (p.clone(), *k))
                        .collect();
                    for (path, first) in due {
                        pending.remove(&path);
                        let Some(event) = settle(&path, first, &mut folders, &mut reported) else {
                            continue;
                        };
                        let Some(payload) = payload(&root, &config, &path, event) else {
                            continue;
                        };
                        let Some(service) = service.upgrade() else {
                            return;
                        };
                        if let Err(e) = service.fire(&watcher.id, SourceEvent::new(payload)).await {
                            tracing::warn!(watcher = %watcher.id, "folder event lost: {e}");
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

/// What a settled path is, or `None` when it counts for nothing.
fn settle(
    path: &Path,
    first: Seen,
    folders: &mut HashSet<PathBuf>,
    reported: &mut HashMap<PathBuf, (u64, Option<SystemTime>)>,
) -> Option<(FolderEvent, Option<u64>)> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            folders.insert(path.to_path_buf());
            None
        }
        Ok(meta) => {
            let now = (meta.len(), modified(&meta));
            let event = if first == Seen::Created {
                FolderEvent::Created
            } else {
                FolderEvent::Changed
            };
            // Unchanged since it was last reported, whatever the OS called the event: a sync
            // engine's touch or replace (Review Focus 1).
            if reported.get(path) == Some(&now) {
                return None;
            }
            reported.insert(path.to_path_buf(), now);
            Some((event, Some(meta.len())))
        }
        Err(_) => {
            reported.remove(path);
            if folders.remove(path) || first == Seen::Created {
                return None;
            }
            Some((FolderEvent::Deleted, None))
        }
    }
}

/// The run's payload for a settled path, unless the skip list, the subfolder rule, the pattern
/// or the watcher's events leave it out.
fn payload(
    root: &Path,
    config: &FolderConfig,
    path: &Path,
    (event, size): (FolderEvent, Option<u64>),
) -> Option<Value> {
    let relative = path.strip_prefix(root).ok()?;
    if skipped(relative) {
        return None;
    }
    if !config.recursive && relative.components().count() > 1 {
        return None;
    }
    let name = relative.file_name()?.to_string_lossy();
    if !matches_pattern(&config.pattern, &name) || !config.events.contains(&event) {
        return None;
    }
    Some(json!({
        "source": "folder",
        "event": event.as_str(),
        "path": root.join(relative).to_string_lossy(),
        "relative": relative.to_string_lossy(),
        "size": size,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deferred minor: a settled file reported again unchanged is dropped whatever the OS
    /// called it (a sync engine's replace can read as a creation).
    #[test]
    fn a_settled_file_reported_again_unchanged_is_dropped_whatever_the_os_called_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"abc").unwrap();
        let mut folders = HashSet::new();
        let mut reported = HashMap::new();
        assert!(settle(&path, Seen::Created, &mut folders, &mut reported).is_some());
        assert_eq!(
            settle(&path, Seen::Created, &mut folders, &mut reported),
            None
        );
        assert_eq!(
            settle(&path, Seen::Changed, &mut folders, &mut reported),
            None
        );
        std::fs::write(&path, b"abcd").unwrap();
        assert!(settle(&path, Seen::Created, &mut folders, &mut reported).is_some());
    }
}

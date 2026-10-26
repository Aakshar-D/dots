//! Files a watcher stages for a run (watchers spec section 4.2): an email's attachments, saved
//! under Dots' data folder at fetch time (`<root>/<watcher_id>/<run_id>/`) with safe names, at
//! most `MAX_ATTACHMENTS_BYTES` per email in all.

use std::path::PathBuf;

use crate::Result;

/// The most an email's attachments may weigh in all; larger ones are listed, not saved.
pub const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;

/// The longest safe name, in characters.
const MAX_NAME_CHARS: usize = 120;

const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

/// Whether `stem` is a reserved Windows device name.
fn reserved(stem: &str) -> bool {
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
/// control characters, `<>:"|?*` as `_`, no spaces or dots at the end, a reserved device name
/// prefixed with `_`, at most 120 characters keeping the extension; `attachment` when nothing
/// is left.
pub fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    while out.ends_with([' ', '.']) {
        out.pop();
    }
    if out.trim().is_empty() {
        return "attachment".to_string();
    }
    if reserved(split_extension(&out).0) {
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
            let (stem, ext) = split_extension(name);
            candidate = format!("{stem} ({n}){ext}");
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
        if files.is_empty() {
            return Ok(());
        }
        let dir = self.stage_dir(watcher_id, run_id);
        std::fs::create_dir_all(&dir)?;
        for (name, bytes) in files {
            std::fs::write(dir.join(name), bytes)?;
        }
        Ok(())
    }

    /// Removes everything staged for the watcher (it was deleted).
    pub fn forget_watcher(&self, watcher_id: &str) {
        let dir = self.root.join(watcher_id);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!(watcher = %watcher_id, "removing staged files failed: {e}");
            }
        }
    }
}

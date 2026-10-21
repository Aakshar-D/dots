//! The skill creator (skill creator spec): what a drafted `SKILL.md` must hold to be saved, and
//! the folder Dots keeps the skills it makes in.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::kit::parse_frontmatter;
use crate::library::{check_name, LibraryKind};
use crate::{Error, Result};

/// The most a `SKILL.md` the creator saves may hold, in bytes.
pub const MAX_SKILL_BYTES: usize = 64 * 1024;

/// What the Library will show for a draft that can be saved (`skill_check`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftSummary {
    pub name: String,
    pub description: String,
}

/// Names Windows keeps for devices, also with an extension: no folder can have them.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

/// A name that is a valid Library name but no folder name on Windows: a device name, or one
/// ending in `.`, which Windows drops.
fn not_a_folder_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    name.ends_with('.') || RESERVED.contains(&stem.as_str())
}

/// Whether `text` can be saved as a skill (skill creator spec §6, step 1): it opens with a
/// frontmatter that the Library can read, with a `name` the Library takes for a skill and that
/// can be a folder name, and a `description`, and it has instructions below. `Invalid` says
/// what to change.
pub fn check_draft(text: &str) -> Result<DraftSummary> {
    if text.len() > MAX_SKILL_BYTES {
        return invalid(format!(
            "The skill is longer than {} KB; shorten it.",
            MAX_SKILL_BYTES / 1024
        ));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.lines().next().is_none_or(|l| l.trim_end() != "---") {
        return invalid(
            "The skill must start with a --- line, then its name and description, then another \
             --- line.",
        );
    }
    let parsed = parse_frontmatter(text);
    if parsed.fields.is_empty() {
        return invalid(
            "The lines between the --- lines could not be read. Check for a missing closing --- \
             line or quote and a name used twice, and put a description that holds \": \" in \
             double quotes.",
        );
    }
    let field = |key: &str| {
        parsed
            .fields
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };
    let Some(name) = field("name") else {
        return invalid("Add a name: line between the --- lines.");
    };
    check_name(LibraryKind::Skill, name)?;
    if not_a_folder_name(name) {
        return invalid(format!(
            "The name {name:?} cannot be a folder name on Windows; pick another."
        ));
    }
    let Some(description) = field("description") else {
        return invalid("Add a description: line between the --- lines.");
    };
    if parsed.body.trim().is_empty() {
        return invalid("Add the skill's instructions below the second --- line.");
    }
    Ok(DraftSummary {
        name: name.to_string(),
        description: description.to_string(),
    })
}

/// Writes `text` to `<root>/<name>/SKILL.md` (spec §6, step 3): into a hidden folder beside it
/// first, then renamed into place, so a failed write leaves no half-written skill. `Conflict`
/// when `<root>/<name>` exists; nothing is overwritten.
pub(crate) fn write_skill_folder(root: &Path, name: &str, text: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    let dir = root.join(name);
    if std::fs::symlink_metadata(&dir).is_ok() {
        return Err(Error::Conflict(format!(
            "{name} already exists in Dots' skills folder ({}).",
            dir.display()
        )));
    }
    let tmp = root.join(format!(".{name}.{}", crate::util::new_id()));
    std::fs::create_dir(&tmp)?;
    let written =
        std::fs::write(tmp.join("SKILL.md"), text).and_then(|()| std::fs::rename(&tmp, &dir));
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    Ok(dir)
}

/// True when `dir` is a folder inside `root`, compared after resolving both: a skill there is
/// Dots' own, and removing it deletes its folder (spec §7). False when either cannot be resolved,
/// and when `dir` itself is a symlink or junction: a link is never deleted through.
pub(crate) fn is_inside(dir: &Path, root: &Path) -> bool {
    if is_link(dir) {
        return false;
    }
    match (dir.canonicalize(), root.canonicalize()) {
        (Ok(dir), Ok(root)) => dir != root && dir.starts_with(&root),
        _ => false,
    }
}

/// Whether `path` is a symlink, or on Windows a junction or other reparse point.
fn is_link(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    let reparse = std::os::windows::fs::MetadataExt::file_attributes(&meta) & 0x400 != 0;
    #[cfg(not(windows))]
    let reparse = false;
    meta.file_type().is_symlink() || reparse
}

/// Whether `name` is what `write_skill_folder` gives its hidden folder: `.<skill name>.<ULID>`.
fn is_unfinished_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((skill, id)) = rest.rsplit_once('.') else {
        return false;
    };
    id.len() == 26
        && id
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z'))
        && check_name(LibraryKind::Skill, skill).is_ok()
}

/// Deletes the folder of one of Dots' own skills. The error names the folder (spec §7).
pub(crate) fn delete_skill_folder(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Other(format!(
            "Could not delete {}: {e}. Close anything using it and try again.",
            dir.display()
        ))),
    }
}

/// Removes what a save that never finished left in `root`: the hidden folders named as
/// `write_skill_folder` names them, and no other (a person may keep a `.git` there). For startup,
/// when no save is running.
pub fn sweep_unfinished(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir()) && !is_link(&entry.path());
        if is_dir && is_unfinished_name(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

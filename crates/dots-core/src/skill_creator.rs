//! The skill creator (skill creator spec): drafting a `SKILL.md` with the `claude` CLI, what a
//! draft must hold to be saved, and the folder Dots keeps the skills it makes in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::{ask_once, Once, OnceFailure};
use crate::kit::parse_frontmatter;
use crate::library::{check_name, LibraryKind};
use crate::{Error, Result};

/// The system prompt of a draft (spec §5). It names the rules `check_draft` holds a draft to.
pub const DRAFTING_INSTRUCTIONS: &str = "\
You write skills for Claude Code: a SKILL.md file that an assistant reads when a task calls for it.

Answer with the complete SKILL.md and nothing else: no greeting, no explanation, and no code \
fence around it.

The file starts with a frontmatter block:
- the first line is ---
- then a line name: <name>, where the name uses only lowercase letters, digits and hyphens, \
is at most 64 characters, and says what the skill is for
- then a line description: \"<what the skill does and when to use it>\", on one line, in \
double quotes, with a double quote inside written as \\\"
- then a line ---
Use only these top-level key: value lines. No lists, no indented lines, no other keys.

After the frontmatter, write the instructions in Markdown, for an assistant that will follow \
them: what to do, step by step, what to check, and what to hand back. Keep them specific to \
the idea and short enough to read in a minute.

When you are given a current SKILL.md and a change, answer with the whole revised file: \
changed as asked, and otherwise kept as it was.
";

/// How long a draft may take before the CLI is ended (spec §5).
pub const DRAFT_LIMIT: Duration = Duration::from_secs(5 * 60);

/// The most an idea or a change request may hold, in characters.
pub const MAX_REQUEST_CHARS: usize = 4000;

/// The model a draft uses when the dialog names none.
pub const DEFAULT_MODEL: &str = "sonnet";

/// What the dialog asks for (`skill_draft`): a first draft of `idea`, or, with `current` and
/// `change`, the current text changed as asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRequest {
    pub idea: String,
    pub model: String,
    pub current: Option<String>,
    pub change: Option<String>,
}

/// The request's text, sent on the CLI's stdin. `Invalid` for an empty idea or model, an
/// idea or change over `MAX_REQUEST_CHARS`, a current text over `MAX_SKILL_BYTES`, and a
/// change without the text to change (or the reverse).
pub fn draft_prompt(req: &DraftRequest) -> Result<String> {
    let idea = req.idea.trim();
    if idea.is_empty() {
        return invalid("Describe the skill first.");
    }
    if req.model.trim().is_empty() {
        return invalid("Pick a model.");
    }
    let too_long = |s: &str| s.chars().count() > MAX_REQUEST_CHARS;
    if too_long(idea) {
        return invalid(format!(
            "Describe the skill in at most {MAX_REQUEST_CHARS} characters."
        ));
    }
    match (req.current.as_deref(), req.change.as_deref().map(str::trim)) {
        (None, None) => Ok(format!("Write a skill for this idea:\n\n{idea}\n")),
        (Some(_), Some("")) | (None, Some(_)) => invalid("Say what should change."),
        (Some(_), None) => invalid("Say what should change."),
        (Some(current), Some(change)) => {
            if too_long(change) {
                return invalid(format!(
                    "Say what should change in at most {MAX_REQUEST_CHARS} characters."
                ));
            }
            if current.len() > MAX_SKILL_BYTES {
                return invalid(format!(
                    "The skill is longer than {} KB; shorten it.",
                    MAX_SKILL_BYTES / 1024
                ));
            }
            Ok(format!(
                "The idea the skill was written for:\n\n{idea}\n\n\
                 The current SKILL.md, between the lines <<<SKILL.md and SKILL.md>>>:\n\n\
                 <<<SKILL.md\n{}\nSKILL.md>>>\n\n\
                 Change it like this:\n\n{change}\n\n\
                 Answer with the whole revised SKILL.md.\n",
                current.trim_end()
            ))
        }
    }
}

/// The answer without surrounding blank lines and, when the whole answer is one fenced code
/// block, without the fence. It ends with one newline.
pub fn unwrap_fence(answer: &str) -> String {
    let text = answer.trim();
    let inner = text
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .and_then(|inner| inner.split_once('\n'))
        .filter(|(info, body)| {
            info.trim().chars().all(|c| c.is_ascii_alphanumeric())
                && !body.lines().any(|l| l.trim_start().starts_with("```"))
        })
        .map(|(_, body)| body.trim());
    format!("{}\n", inner.unwrap_or(text))
}

/// The drafts being written, at most one per dialog.
pub struct SkillDrafts {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
    /// Each draft's CLI starts in a new empty folder in here.
    dir: PathBuf,
    limit: Duration,
    shutdown: CancellationToken,
    /// By dialog id: a number for the draft, and what cancels it.
    running: Mutex<HashMap<String, (u64, CancellationToken)>>,
    next: Mutex<u64>,
}

impl SkillDrafts {
    /// `program` is the CLI the runtime found, `None` when there is none.
    pub fn new(
        program: Option<PathBuf>,
        env: Vec<(String, String)>,
        dir: PathBuf,
        limit: Duration,
        shutdown: CancellationToken,
    ) -> SkillDrafts {
        SkillDrafts {
            program,
            env,
            dir,
            limit,
            shutdown,
            running: Mutex::new(HashMap::new()),
            next: Mutex::new(0),
        }
    }

    /// Writes a draft for the dialog `dialog` (`skill_draft`, spec §5) and returns its text. A
    /// draft the same dialog asked for before is cancelled first. Fails with the CLI's error,
    /// when the answer holds no text, when `cancel` is called, or after the time limit.
    pub async fn draft(&self, dialog: &str, req: &DraftRequest) -> Result<String> {
        let prompt = draft_prompt(req)?;
        let Some(program) = self.program.clone() else {
            return Err(Error::Other(
                "claude CLI not found: install Claude Code or set its path in Settings".into(),
            ));
        };
        let token = self.shutdown.child_token();
        let number = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            *next
        };
        let previous = self
            .running
            .lock()
            .unwrap()
            .insert(dialog.to_string(), (number, token.clone()));
        if let Some((_, previous)) = previous {
            previous.cancel();
        }
        let cwd = self.dir.join(crate::util::new_id());
        let asked = match std::fs::create_dir_all(&cwd) {
            Ok(()) => {
                let once = Once {
                    program: &program,
                    env: &self.env,
                    cwd: &cwd,
                    model: req.model.trim(),
                    system_prompt: DRAFTING_INSTRUCTIONS,
                    prompt: &prompt,
                    limit: self.limit,
                };
                let asked = ask_once(&once, &token).await;
                let _ = std::fs::remove_dir_all(&cwd);
                asked
            }
            Err(e) => Err(OnceFailure::Failed(format!(
                "could not make {}: {e}",
                cwd.display()
            ))),
        };
        {
            let mut running = self.running.lock().unwrap();
            if running.get(dialog).is_some_and(|(n, _)| *n == number) {
                running.remove(dialog);
            }
        }
        match asked {
            Ok(answer) if answer.trim().is_empty() => Err(Error::Other(
                "Claude answered with no text; try again.".into(),
            )),
            Ok(answer) => Ok(unwrap_fence(&answer)),
            Err(OnceFailure::Cancelled) => Err(Error::Other("The draft was cancelled.".into())),
            Err(OnceFailure::TimedOut) => Err(Error::Other(
                "Claude took too long to write the skill.".into(),
            )),
            Err(OnceFailure::Failed(e)) => Err(Error::Other(format!(
                "Claude could not write the skill: {e}"
            ))),
        }
    }

    /// Cancels the dialog's draft, if one is being written (`skill_draft_cancel`).
    pub fn cancel(&self, dialog: &str) {
        if let Some((_, token)) = self.running.lock().unwrap().remove(dialog) {
            token.cancel();
        }
    }
}

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

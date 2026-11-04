//! Files the person attaches to a chat message (attachments spec section 4.2). Each is staged
//! the moment it is added: Dots copies it to `<data>/chat-files/<id>/<file_name>`, its stored
//! copy, which is kept as long as its topic exists. Before each run of a topic, the runner copies
//! the topic's files that are missing into `<workspace>/attachments/<root>/` (`copy_into`).

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::model::ChatAttachment;
use crate::store::Store;
use crate::watchers::files::safe_name;
use crate::{Error, Result};

/// The most files one message carries.
pub const MAX_FILES: usize = 10;
/// The most bytes one file may have: 25 MB. Exactly this is accepted.
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// The folder of stored copies, in Dots' data folder.
pub const DIR: &str = "chat-files";
/// The file in a topic's attachments folder that keeps its files out of git.
pub const IGNORE_FILE: &str = ".gitignore";
/// The folder in a workspace that holds the topics' files.
pub const WORKSPACE_DIR: &str = "attachments";

pub const TOO_MANY: &str = "You can attach up to 10 files to a message.";
pub const GONE: &str = "An attached file is no longer there. Remove it and attach it again.";
pub const PROGRAM: &str = "Dots doesn't open programs or scripts. Use Show in folder.";
pub const NOT_THERE: &str = "this file is no longer there";
/// The start of a run's error when a file of its own message could not be put in place.
pub const COPY_FAILED: &str = "could not put the attached files in the workspace";

fn too_large(name: &str) -> Error {
    Error::Invalid(format!("{name} is larger than 25 MB."))
}

fn unreadable(name: &str, e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{name} could not be read: {e}"))
}

/// Extensions Dots shows but never opens: opening runs them (attachments spec section 4.5.1).
const PROGRAMS: [&str; 28] = [
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "msi",
    "msp",
    "scr",
    "hta",
    "lnk",
    "url",
    "cpl",
    "reg",
    "jar",
    "pif",
    "application",
    "gadget",
    "msc",
    "scf",
    "inf",
    "appref-ms",
];

/// Whether Dots may open the file with its default handler: its extension is not a program's
/// or a script's.
pub fn can_open(path: &Path) -> bool {
    match path.extension() {
        Some(ext) => {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            !PROGRAMS.contains(&ext.as_str())
        }
        None => true,
    }
}

/// A size as the UI's `formatBytes` writes it: `240 B`, `2.4 KB`, `30 MB`.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return one_place(kb, "KB");
    }
    one_place(kb / 1024.0, "MB")
}

fn one_place(n: f64, unit: &str) -> String {
    if n < 10.0 {
        format!("{n:.1} {unit}")
    } else {
        format!("{} {unit}", n.round() as u64)
    }
}

/// Whether the entry is a link of any kind: a symlink, a junction or another reparse point.
fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Whether `path` is missing or a real folder, never a link.
fn missing_or_real_dir(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(meta) if is_link(&meta) => Err(std::io::Error::other(format!(
            "{} is a link",
            path.display()
        ))),
        Ok(meta) if !meta.is_dir() => Err(std::io::Error::other(format!(
            "{} is not a folder",
            path.display()
        ))),
        Ok(_) => Ok(()),
    }
}

/// Makes `path` a real folder if it is missing; refuses a link or a file in its place.
fn real_dir(path: &Path) -> std::io::Result<()> {
    missing_or_real_dir(path)?;
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => missing_or_real_dir(path),
        Err(e) => Err(e),
    }
}

/// Whether the topic root id may name a folder: letters and digits only.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The block a message's prompt gets for its files (attachments spec section 4.4): each file's
/// absolute path and size.
pub fn prompt_block(files: &[(PathBuf, u64)]) -> String {
    let mut out = String::from(
        "## Attached files\n\nThe person attached these files to this message. They are in your \
         workspace and stay there for the rest of this conversation:\n\n",
    );
    for (path, size) in files {
        out.push_str(&format!("- {} ({})\n", path.display(), human_size(*size)));
    }
    out
}

/// Where a topic's files go in a workspace.
pub fn topic_dir(workspace: &Path, root_run_id: &str) -> PathBuf {
    workspace.join(WORKSPACE_DIR).join(root_run_id)
}

/// Where the stored copies live.
#[derive(Debug, Clone)]
pub struct ChatFiles {
    root: PathBuf,
}

impl ChatFiles {
    pub fn new(root: PathBuf) -> ChatFiles {
        ChatFiles { root }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// Dots' own copy of the file.
    pub fn stored_path(&self, a: &ChatAttachment) -> PathBuf {
        self.dir(&a.id).join(&a.file_name)
    }

    /// Copies the file at `from` (a picked or dropped path) as attachment `id`. Answers its
    /// safe name and size, or a refusal for a link, a folder, a file over 25 MB, or one that
    /// cannot be read. Nothing is left behind after a refusal.
    pub fn stage_path(&self, id: &str, from: &Path) -> Result<(String, u64)> {
        let shown = from
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| from.to_string_lossy().to_string());
        let name = safe_name(&shown);
        // Never followed: a link is refused as it is.
        let meta = std::fs::symlink_metadata(from).map_err(|e| unreadable(&name, e))?;
        if is_link(&meta) {
            return Err(Error::Invalid(format!(
                "{name} is a link. Attach the file it points to instead."
            )));
        }
        if meta.is_dir() {
            return Err(Error::Invalid(format!(
                "{name} is a folder. Attach the files in it instead."
            )));
        }
        if !meta.is_file() {
            return Err(unreadable(&name, "it is not a file"));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(too_large(&name));
        }
        let source = std::fs::File::open(from).map_err(|e| unreadable(&name, e))?;
        let opened = source.metadata().map_err(|e| unreadable(&name, e))?;
        if !opened.is_file() {
            return Err(unreadable(&name, "it is not a file"));
        }
        let size = self.write(id, &name, source.take(MAX_FILE_BYTES + 1))?;
        Ok((name, size))
    }

    /// Stores `bytes` (a pasted image) as attachment `id`, under the safe form of `name`.
    pub fn stage_bytes(&self, id: &str, name: &str, bytes: &[u8]) -> Result<(String, u64)> {
        let name = safe_name(name);
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(too_large(&name));
        }
        let size = self.write(id, &name, bytes)?;
        Ok((name, size))
    }

    /// Writes the stored copy from `from`, which yields at most `MAX_FILE_BYTES + 1` bytes.
    /// More than the limit (a file that grew while copied) is refused, and the folder goes.
    fn write(&self, id: &str, name: &str, mut from: impl Read) -> Result<u64> {
        if !plain_id(id) {
            return Err(Error::Invalid(format!("attachment id {id} is not valid")));
        }
        let dir = self.dir(id);
        let outcome = (|| -> Result<u64> {
            std::fs::create_dir_all(&dir).map_err(|e| unreadable(name, e))?;
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(name))
                .map_err(|e| unreadable(name, e))?;
            let copied = std::io::copy(&mut from, &mut out).map_err(|e| unreadable(name, e))?;
            out.flush().map_err(|e| unreadable(name, e))?;
            if copied > MAX_FILE_BYTES {
                return Err(too_large(name));
            }
            Ok(copied)
        })();
        if outcome.is_err() {
            self.remove(id);
        }
        outcome
    }

    /// Puts the topic's `files` that are missing into `<workspace>/attachments/<root>/`, with a
    /// `.gitignore` of `*` (attachments spec section 4.4). A file that is there is never
    /// overwritten: the dot may have edited it. Nothing is written through a link: `attachments`
    /// and the topic folder must each be missing or a real folder inside the workspace, and each
    /// file is created new. Answers the files it could not place, by id, with why.
    pub fn copy_into(
        &self,
        root_run_id: &str,
        files: &[ChatAttachment],
        workspace: &Path,
    ) -> Vec<(String, Error)> {
        let placed: Vec<&ChatAttachment> = files.iter().filter(|f| f.name.is_some()).collect();
        if placed.is_empty() {
            return Vec::new();
        }
        let dir = match self.topic_folder(root_run_id, workspace) {
            Ok(dir) => dir,
            Err(e) => {
                let text = e.to_string();
                return placed
                    .iter()
                    .map(|f| (f.id.clone(), Error::Other(text.clone())))
                    .collect();
            }
        };
        let mut failed = Vec::new();
        for f in placed {
            if let Err(e) = self.place(f, &dir) {
                failed.push((f.id.clone(), Error::Other(e.to_string())));
            }
        }
        failed
    }

    /// The topic's folder in the workspace, made safe to write into: both levels real folders
    /// (never links), inside the canonical workspace, with the `.gitignore` in place.
    fn topic_folder(&self, root_run_id: &str, workspace: &Path) -> std::io::Result<PathBuf> {
        if !plain_id(root_run_id) {
            return Err(std::io::Error::other(format!(
                "{root_run_id} is not a topic id"
            )));
        }
        let all = workspace.join(WORKSPACE_DIR);
        real_dir(&all)?;
        let dir = all.join(root_run_id);
        real_dir(&dir)?;
        let inside = std::fs::canonicalize(workspace)?;
        let canonical = std::fs::canonicalize(&dir)?;
        // Two levels below the workspace itself: no link anywhere on the way.
        if canonical.parent().and_then(Path::parent) != Some(inside.as_path()) {
            return Err(std::io::Error::other(format!(
                "{} is outside the workspace",
                dir.display()
            )));
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(canonical.join(IGNORE_FILE))
        {
            Ok(mut f) => f.write_all(b"*\n")?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        Ok(canonical)
    }

    /// Copies one stored file into `dir` when nothing is at its name.
    fn place(&self, f: &ChatAttachment, dir: &Path) -> std::io::Result<()> {
        let name = f.name.as_deref().unwrap_or_default();
        if Path::new(name).file_name().map(|n| n.to_string_lossy()) != Some(name.into())
            || name == IGNORE_FILE
        {
            return Err(std::io::Error::other(format!("{name} is not a file name")));
        }
        let to = dir.join(name);
        // There already, as a file, a folder or a link: left as it is.
        if std::fs::symlink_metadata(&to).is_ok() {
            return Ok(());
        }
        let mut from = std::fs::File::open(self.stored_path(f))?;
        let mut out = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&to)
        {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) => return Err(e),
        };
        let copied = std::io::copy(&mut from, &mut out).and_then(|_| out.flush());
        if let Err(e) = copied {
            drop(out);
            let _ = std::fs::remove_file(&to);
            return Err(e);
        }
        Ok(())
    }

    /// Removes attachment `id`'s stored copy. A failure is logged.
    pub fn remove(&self, id: &str) {
        if !plain_id(id) {
            return;
        }
        let dir = self.dir(id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(dir = %dir.display(), "removing an attached file failed: {e}"),
        }
    }

    /// At start (attachments spec section 4.7): staged rows go (no chat box survives a
    /// restart), and so does every folder that has no row.
    pub async fn sweep(&self, store: &Store) {
        match store.delete_unsent_attachments().await {
            Ok(ids) => {
                for id in ids {
                    self.remove(&id);
                }
            }
            Err(e) => tracing::warn!("removing staged files failed: {e}"),
        }
        let kept: HashSet<String> = match store.all_attachment_ids().await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("reading attached files failed: {e}");
                return;
            }
        };
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if kept.contains(&name) {
                continue;
            }
            let path = entry.path();
            let gone = if entry.file_type().is_ok_and(|t| t.is_dir()) {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = gone {
                tracing::warn!(path = %path.display(), "removing a stray attached file failed: {e}");
            }
        }
    }
}

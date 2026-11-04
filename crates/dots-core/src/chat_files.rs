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
pub const PROGRAM: &str = "Dots only opens documents, images and media. Use Show in folder.";
pub const NOT_THERE: &str = "this file is no longer there";
/// The start of a run's error when a file of its own message could not be put in place.
pub const COPY_FAILED: &str = "could not put the attached files in the workspace";

fn too_large(name: &str) -> Error {
    Error::Invalid(format!("{name} is larger than 25 MB."))
}

fn unreadable(name: &str, e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{name} could not be read: {e}"))
}

/// The extensions "Open" accepts (attachments spec section 4.5.1): documents, images and media
/// whose default program does not run code from the file. Anything else is listed, but only
/// shown in its folder.
const OPENABLE: [&str; 38] = [
    // Documents and text.
    "pdf", "txt", "md", "rtf", "csv", "tsv", "json", "xml", "yaml", "yml", "toml", "log",
    // Images.
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "heic",
    // Audio and video.
    "mp3", "wav", "m4a", "ogg", "flac", "mp4", "mov", "webm", "mkv", "avi",
    // Office documents without macros.
    "docx", "xlsx", "pptx", "odt", "ods", "odp",
];

/// Whether `name` is a file name Windows keeps as it is: no trailing dot or space (Windows drops
/// them, so `run.cmd.` names `run.cmd`) and no `:` (`a.pdf:x.exe` names a stream of `a.pdf`).
pub fn plain_name(name: &str) -> bool {
    !name.contains(':') && name.trim_end_matches(['.', ' ']) == name
}

/// Whether Dots may open the file at `path` with its default program, by its name alone: a
/// plain name (`plain_name`) whose extension, ignoring case, is in the allow-list.
pub fn can_open(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if !plain_name(name) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    match Path::new(&lower).extension().and_then(|e| e.to_str()) {
        Some(ext) => OPENABLE.contains(&ext),
        None => false,
    }
}

/// Whether Dots may open the file at `path` now: `can_open` by its name, and the file it really
/// is (its canonical path) has that same name, ignoring case on Windows, and is a file.
pub fn can_open_file(path: &Path) -> bool {
    if !can_open(path) {
        return false;
    }
    let Ok(real) = std::fs::canonicalize(path) else {
        return false;
    };
    if !real.is_file() || !can_open(&real) {
        return false;
    }
    let same = |a: &std::ffi::OsStr, b: &std::ffi::OsStr| {
        let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
        if cfg!(windows) {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        }
    };
    matches!((path.file_name(), real.file_name()), (Some(a), Some(b)) if same(a, b))
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

/// Opens `path` to read without following a link at its last part: on Windows the link itself
/// is opened (`FILE_FLAG_OPEN_REPARSE_POINT`), so its handle says it is one.
fn open_no_follow(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

/// Whether the opened file is the entry `checked` described. On Unix the device and inode must
/// match (a link swapped in opens its target, another inode); on Windows the handle of a link
/// is the link itself, which `is_link` refuses.
fn same_entry(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        checked.dev() == opened.dev() && checked.ino() == opened.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (checked, opened);
        true
    }
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

/// Whether `copy` is a real file (never a link) with the same bytes as `original`. Any failure
/// to read either answers false.
fn same_bytes(copy: &Path, original: &Path) -> bool {
    let compare = || -> std::io::Result<bool> {
        let meta = std::fs::symlink_metadata(copy)?;
        if is_link(&meta) || !meta.is_file() || meta.len() != std::fs::metadata(original)?.len() {
            return Ok(false);
        }
        let mut a = std::io::BufReader::new(std::fs::File::open(copy)?);
        let mut b = std::io::BufReader::new(std::fs::File::open(original)?);
        let (mut x, mut y) = ([0u8; 8192], [0u8; 8192]);
        loop {
            let n = a.read(&mut x)?;
            if n == 0 {
                // Same length, so the original is done too.
                return Ok(true);
            }
            b.read_exact(&mut y[..n])?;
            if x[..n] != y[..n] {
                return Ok(false);
            }
        }
    };
    compare().unwrap_or(false)
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

/// `path` with `.` dropped and `..` applied, without touching the disk. A `..` above the root
/// is dropped.
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir) | Some(Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A path's key for comparing: normalized, with `/` and `\` alike, and ignoring case on
/// Windows.
pub fn path_key(path: &Path) -> String {
    let text = normalize(path).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Whether `path` is inside `workspace`, by its normalized text and, when it exists, by where
/// it really is (a link inside that points out does not count).
pub fn is_inside(path: &Path, workspace: &Path) -> bool {
    let p = path_key(path);
    let w = path_key(workspace);
    let w = w.trim_end_matches('/');
    if !p.starts_with(&format!("{w}/")) {
        return false;
    }
    match (
        std::fs::canonicalize(path),
        std::fs::canonicalize(workspace),
    ) {
        (Ok(real), Ok(base)) => real.starts_with(&base),
        (Err(_), _) => true,
        (Ok(_), Err(_)) => false,
    }
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
        // Opened without following a link, and checked again on the handle: a file swapped for
        // a link after the check above is refused too.
        let source = open_no_follow(from).map_err(|e| unreadable(&name, e))?;
        let opened = source.metadata().map_err(|e| unreadable(&name, e))?;
        if is_link(&opened) || !same_entry(&meta, &opened) {
            return Err(Error::Invalid(format!(
                "{name} is a link. Attach the file it points to instead."
            )));
        }
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
    /// file is created new. A file of `run_id`'s own message whose name is taken by anything
    /// but Dots' unchanged copy is not placed: the dot would be told the wrong file is the
    /// attached one. Answers the files it could not place, by id, with why.
    pub fn copy_into(
        &self,
        root_run_id: &str,
        files: &[ChatAttachment],
        workspace: &Path,
        run_id: &str,
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
            let own = f.run_id.as_deref() == Some(run_id);
            if let Err(e) = self.place(f, &dir, own) {
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

    /// Copies one stored file into `dir` when nothing is at its name. For a file of the run's
    /// `own` message, something else at its name is an error; Dots' own unchanged copy (the run
    /// started again) is fine.
    fn place(&self, f: &ChatAttachment, dir: &Path, own: bool) -> std::io::Result<()> {
        let name = f.name.as_deref().unwrap_or_default();
        if Path::new(name).file_name().map(|n| n.to_string_lossy()) != Some(name.into())
            || name == IGNORE_FILE
        {
            return Err(std::io::Error::other(format!("{name} is not a file name")));
        }
        let to = dir.join(name);
        let taken = || {
            if own && !same_bytes(&to, &self.stored_path(f)) {
                Err(std::io::Error::other(format!(
                    "something else is already at {name} in the topic's folder"
                )))
            } else {
                Ok(())
            }
        };
        // There already, as a file, a folder or a link: left as it is.
        if std::fs::symlink_metadata(&to).is_ok() {
            return taken();
        }
        let mut from = std::fs::File::open(self.stored_path(f))?;
        let mut out = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&to)
        {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return taken(),
            Err(e) => return Err(e),
        };
        // The folder may have been swapped for a link since it was checked: the new file must
        // be in it, or it goes before anything is written to it.
        let inside = std::fs::canonicalize(&to).is_ok_and(|real| real.parent() == Some(dir));
        if !inside {
            drop(out);
            let _ = std::fs::remove_file(&to);
            return Err(std::io::Error::other(format!(
                "{} is outside the workspace",
                to.display()
            )));
        }
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

    /// Whether the topic's folder in `workspace` holds anything other than Dots' own copies of
    /// `files`, unchanged: an entry with no file of that name, a link or a folder, or a file
    /// whose bytes differ from its stored copy (attachments spec section 7). A worktree with
    /// such a folder is not removed as unchanged, though git ignores the folder. Anything that
    /// cannot be read counts as changed.
    pub fn changed_in(
        &self,
        root_run_id: &str,
        files: &[ChatAttachment],
        workspace: &Path,
    ) -> bool {
        if !plain_id(root_run_id) {
            return false;
        }
        let dir = topic_dir(workspace, root_run_id);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return true;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if name == IGNORE_FILE {
                continue;
            }
            let Some(file) = files
                .iter()
                .find(|f| f.name.as_deref() == Some(name.as_str()))
            else {
                return true;
            };
            if !same_bytes(&entry.path(), &self.stored_path(file)) {
                return true;
            }
        }
        false
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

//! Checkpoints of folder runs: what a run changed in its folder, and putting it back (undo spec,
//! `docs/superpowers/specs/2026-11-06-undo-run-design.md`).
//!
//! Before a folder run's engine starts, Dots takes a snapshot of the folder; when the run ends it
//! takes another. The two snapshots are git trees, so the change list and the diff of each file
//! come from `git diff-tree`, and undo writes the files of the first snapshot back.
//!
//! - A folder inside a git repository is snapshotted with the repository's own machinery and
//!   nothing the person uses changes: a temporary index (a copy of theirs, in Dots' data folder),
//!   `git add -A` of the folder into it, `write-tree` and `commit-tree`, kept under the private
//!   ref `refs/dots/checkpoints/<run id>` (the run's "after" commit, whose parent is its "before").
//!   Their index, branches, HEAD and stash are never touched. Files the repository ignores are
//!   not in the snapshot.
//! - Any other folder is snapshotted into a private repository of Dots in its data folder
//!   (`<data>/checkpoints/shadow/<folder key>.git`): Dots walks the folder itself, skips the
//!   folders that hold dependencies or build output, links, and files over [`MAX_FILE_BYTES`], and
//!   stores each file's exact bytes.
//!
//! A file too large to keep is still watched by its size and time: when it changes it is listed
//! as changed, and as one Dots can't put back.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::model::{FileStatus, WorktreeDiff};
use crate::{Error, Result};

/// The folder of checkpoints in Dots' data folder.
pub const DIR: &str = "checkpoints";
/// A file larger than this is not kept (20 MiB); a change to it is listed as one Dots can't undo.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// A folder with more files than this to keep is not snapshotted at all.
pub const MAX_FILES: usize = 20_000;
/// A folder whose files to keep add up to more than this (512 MiB) is not snapshotted at all.
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// Checkpoints older than this are removed.
pub const KEEP_DAYS: i64 = 30;
/// When Dots' private repositories hold more than this (5 GiB), the oldest checkpoints go first.
pub const MAX_STORE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
/// The longest one snapshot may take; past it the run goes on without a checkpoint.
pub const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(120);
/// Folders never kept in a private snapshot, wherever they are: dependencies, build output and
/// caches, which a run rebuilds rather than edits.
pub const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".gradle",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
];
/// The folder at the top of a workspace where Dots puts a topic's files for the run
/// (`chat_files::WORKSPACE_DIR`): copies, never part of a run's changes.
const OWN_DIR: &str = crate::chat_files::WORKSPACE_DIR;
/// The ref of a run's checkpoint: `refs/dots/checkpoints/<run id>`.
const REF_PREFIX: &str = "refs/dots/checkpoints/";
/// The identity of checkpoint commits, so no git config of the person's is needed.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Dots"),
    ("GIT_AUTHOR_EMAIL", "checkpoints@dots.invalid"),
    ("GIT_COMMITTER_NAME", "Dots"),
    ("GIT_COMMITTER_EMAIL", "checkpoints@dots.invalid"),
];
const TOO_LARGE_REASON: &str =
    "Larger than 20 MB: Sage did not keep a copy, so it can't be put back.";
const LINK_REASON: &str = "A link: Sage does not put links back.";
const REPO_REASON: &str = "A folder with its own git repository: Sage does not put it back.";

/// How a run's folder was snapshotted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// In the git repository the folder belongs to.
    Git,
    /// In Dots' private repository for the folder.
    Folder,
}

/// Where a run's change list stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangesState {
    /// The run is working: only its starting point is kept.
    Recording,
    /// The run ended and its changes are listed.
    Recorded,
    /// Dots could not snapshot the folder; `note` says why.
    Unavailable,
}

/// One file a run created, changed or deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Relative to the run's folder, with `/` between parts.
    pub path: String,
    pub status: FileStatus,
    /// Dots kept what it needs to put the file back.
    pub undoable: bool,
    /// Why it can't be put back, when it can't.
    pub reason: Option<String>,
}

/// What a folder run changed (the run's "Changes").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunChanges {
    pub run_id: String,
    pub state: ChangesState,
    /// The folder is in a git repository (files it ignores are not tracked).
    pub git: bool,
    pub files: Vec<ChangedFile>,
    /// Why there is no change list, or what it leaves out.
    pub note: Option<String>,
    /// When the person undid the run's changes; `None` while they are in place (or redone).
    pub undone_at: Option<String>,
}

/// What undo (or redo) would do to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    /// The file is as the run left it: it will be put back.
    Ready,
    /// The file changed after the run: skipped unless the person ticks "overwrite".
    Conflict,
    /// The file is already as undo would leave it.
    Done,
    /// Dots can't put this file back.
    Cannot,
}

/// One file of an undo or redo, with what would happen to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFile {
    pub path: String,
    pub status: FileStatus,
    pub action: FileAction,
    pub reason: Option<String>,
}

/// What the person sees before undo or redo: each file and what would happen to it, the run's
/// diff, and the later runs in the same folder that changed the same files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangesPreview {
    pub run_id: String,
    /// The run's changes were undone: the action is redo.
    pub redo: bool,
    pub files: Vec<PlannedFile>,
    pub diff: WorktreeDiff,
    /// Other runs in this folder, started after this one, that changed some of the same files.
    pub later_runs: Vec<String>,
}

/// What an undo or redo did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UndoOutcome {
    pub redo: bool,
    /// The files put back.
    pub restored: Vec<String>,
    /// The files left as they are, with why.
    pub skipped: Vec<PlannedFile>,
    pub changes: RunChanges,
}

/// A file too large to keep, as Dots last saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Big {
    path: String,
    size: u64,
    /// Milliseconds since 1970.
    mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Snapshot {
    commit: String,
    #[serde(default)]
    big: Vec<Big>,
    /// Some files could not be read and are not in it.
    #[serde(default)]
    partial: bool,
}

/// One changed path between the two snapshots, with both sides' git mode and blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Change {
    path: String,
    status: FileStatus,
    /// `None`: no file on this side.
    before: Option<Side>,
    after: Option<Side>,
    undoable: bool,
    reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Side {
    mode: String,
    blob: String,
}

/// A run's checkpoint, kept as `<data>/checkpoints/runs/<run id>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    run_id: String,
    folder: String,
    kind: Kind,
    /// The private repository (`Kind::Folder`); empty for `Kind::Git`.
    #[serde(default)]
    shadow: String,
    /// The folder's path inside its repository, `/`-terminated, or empty (`Kind::Git`).
    #[serde(default)]
    prefix: String,
    created_at: String,
    before: Option<Snapshot>,
    after: Option<Snapshot>,
    #[serde(default)]
    changes: Vec<Change>,
    note: Option<String>,
    undone_at: Option<String>,
}

impl Record {
    fn repo(&self) -> Repo {
        let folder = PathBuf::from(&self.folder);
        match self.kind {
            Kind::Git => Repo::Git { folder },
            Kind::Folder => Repo::Shadow {
                git_dir: PathBuf::from(&self.shadow),
                folder,
            },
        }
    }

    fn summary(&self) -> RunChanges {
        let state = match (&self.before, &self.after) {
            (None, _) => ChangesState::Unavailable,
            (Some(_), None) => ChangesState::Recording,
            (Some(_), Some(_)) => ChangesState::Recorded,
        };
        RunChanges {
            run_id: self.run_id.clone(),
            state,
            git: self.kind == Kind::Git,
            files: self
                .changes
                .iter()
                .map(|c| ChangedFile {
                    path: c.path.clone(),
                    status: c.status,
                    undoable: c.undoable,
                    reason: c.reason.clone(),
                })
                .collect(),
            note: self.note.clone(),
            undone_at: self.undone_at.clone(),
        }
    }
}

/// Where git runs for a checkpoint.
enum Repo {
    /// The person's repository, found from the folder.
    Git { folder: PathBuf },
    /// Dots' private repository of the folder.
    Shadow { git_dir: PathBuf, folder: PathBuf },
}

impl Repo {
    fn folder(&self) -> &Path {
        match self {
            Repo::Git { folder } | Repo::Shadow { folder, .. } => folder,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new("git");
        crate::proc::hide_window(&mut cmd);
        cmd.kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match self {
            Repo::Git { folder } => {
                cmd.arg("-C").arg(folder);
            }
            Repo::Shadow { git_dir, folder } => {
                cmd.arg("-C").arg(folder).arg("--git-dir").arg(git_dir);
            }
        }
        cmd.args(["-c", "core.quotePath=false", "-c", "gc.auto=0"]);
        cmd
    }

    async fn run(&self, args: &[&str], env: &[(&str, OsString)], input: Option<&[u8]>) -> Out {
        let mut cmd = self.command();
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        exec(cmd, input).await
    }

    /// `run`, as an error when git fails.
    async fn ok(
        &self,
        args: &[&str],
        env: &[(&str, OsString)],
        input: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let out = self.run(args, env, input).await;
        if out.ok {
            Ok(out.stdout)
        } else {
            Err(Error::Other(format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or_default(),
                out.stderr.trim()
            )))
        }
    }

    async fn text(&self, args: &[&str]) -> Result<String> {
        let out = self.ok(args, &[], None).await?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }
}

struct Out {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
}

async fn exec(mut cmd: Command, input: Option<&[u8]>) -> Out {
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Out {
                ok: false,
                stdout: Vec::new(),
                stderr: format!("could not start git: {e}"),
            }
        }
    };
    let mut child = child;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let input = input.to_vec();
        // Written beside the read of stdout, so a large answer never blocks a large question.
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        });
    }
    match child.wait_with_output().await {
        Ok(out) => Out {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) => Out {
            ok: false,
            stdout: Vec::new(),
            stderr: e.to_string(),
        },
    }
}

/// One lock per folder for the whole process: two snapshots, or a snapshot and an undo, of the
/// same folder never run at once.
static LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Default::default);

/// When `prune` last ran, in seconds since 1970.
static LAST_PRUNE: AtomicI64 = AtomicI64::new(0);

fn folder_key(folder: &Path) -> String {
    let text = folder.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/');
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

fn lock_for(folder: &Path) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS
        .lock()
        .unwrap()
        .entry(folder_key(folder))
        .or_default()
        .clone()
}

/// A run id as it may name a file and a ref: letters and digits only.
fn check_id(run_id: &str) -> Result<()> {
    if run_id.is_empty() || run_id.len() > 64 || !run_id.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(Error::Invalid(format!("not a run id: {run_id:?}")));
    }
    Ok(())
}

fn millis(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// The names Windows keeps for devices, which no file may take.
fn reserved_on_windows(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// The file `rel` (relative, `/` between parts) inside `folder`, for undo to write or delete,
/// refused when it could land anywhere else: an absolute path, `..`, a drive or stream (`:`), a
/// backslash, a part of `.git`, a name Windows changes or keeps for a device, or a link (a
/// symlink or a junction) anywhere from the folder down to the file itself.
pub fn safe_path(folder: &Path, rel: &str) -> Result<PathBuf> {
    let refuse = |why: &str| Error::Invalid(format!("refusing to write {rel:?}: {why}"));
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\0') || rel.contains('\\') {
        return Err(refuse("not a path inside the folder"));
    }
    let mut full = folder.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(refuse("not a path inside the folder"));
        }
        if part.contains(':') {
            return Err(refuse("names a drive or a stream"));
        }
        if part.eq_ignore_ascii_case(".git") {
            return Err(refuse("inside .git"));
        }
        if cfg!(windows)
            && (part.ends_with('.') || part.ends_with(' ') || reserved_on_windows(part))
        {
            return Err(refuse("a name Windows does not keep as written"));
        }
        full.push(part);
    }
    if let Some(link) = crate::workspace::link_on_path(folder, rel) {
        return Err(refuse(&format!("{link} is a link")));
    }
    Ok(full)
}

/// What is at a path now, as undo compares it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Current {
    Absent,
    Blob(String),
    /// A folder, a link, or a file that could not be read; the text says which.
    Other(String),
    /// A path undo must never write (`safe_path` refused it).
    Unsafe(String),
}

/// The sizes past which Dots does not keep a file, or does not snapshot a folder at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_FILE_BYTES,
            max_files: MAX_FILES,
            max_total_bytes: MAX_TOTAL_BYTES,
        }
    }
}

impl Limits {
    fn check(&self, count: usize, total: u64) -> Result<()> {
        if count > self.max_files {
            return Err(Error::Invalid(format!(
                "This folder has more than {} files to back up, so Sage did not back it up and this run can't be undone.",
                self.max_files
            )));
        }
        if total > self.max_total_bytes {
            return Err(Error::Invalid(format!(
                "This folder holds more than {} MB to back up, so Sage did not back it up and this run can't be undone.",
                self.max_total_bytes / (1024 * 1024)
            )));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Checkpoints {
    root: PathBuf,
    limits: Limits,
}

impl Checkpoints {
    /// Checkpoints kept in `root` (`<data>/checkpoints`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            limits: Limits::default(),
        }
    }

    /// The same checkpoints with other size limits (tests use small ones).
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    fn records_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    fn record_path(&self, run_id: &str) -> Result<PathBuf> {
        check_id(run_id)?;
        Ok(self.records_dir().join(format!("{run_id}.json")))
    }

    fn load(&self, run_id: &str) -> Result<Option<Record>> {
        let path = self.record_path(run_id)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, rec: &Record) -> Result<()> {
        let path = self.record_path(&rec.run_id)?;
        std::fs::create_dir_all(self.records_dir())?;
        let tmp = path.with_extension(format!("json.{}.tmp", crate::util::new_id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(rec)?)?;
        std::fs::rename(&tmp, &path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }

    fn all_records(&self) -> Vec<Record> {
        let Ok(dir) = std::fs::read_dir(self.records_dir()) else {
            return Vec::new();
        };
        dir.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<Record>(&bytes).ok())
            .collect()
    }

    fn shadow_dir(&self, folder: &Path) -> PathBuf {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(folder_key(folder).as_bytes());
        let key: String = digest[..10].iter().map(|b| format!("{b:02x}")).collect();
        self.root.join("shadow").join(format!("{key}.git"))
    }

    fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// The checkpoint before a run's engine starts in `folder`. Never fails the run: a folder
    /// Dots can't snapshot gets a record that says why, and the run goes on.
    pub async fn begin(&self, run_id: &str, folder: &Path) {
        if check_id(run_id).is_err() {
            return;
        }
        let lock = lock_for(folder);
        let _held = lock.lock().await;
        let rec = match tokio::time::timeout(SNAPSHOT_TIMEOUT, self.start_record(run_id, folder))
            .await
        {
            Ok(rec) => rec,
            Err(_) => Record {
                note: Some(format!(
                    "Sage took longer than {} seconds to back up this folder, so this run can't be undone.",
                    SNAPSHOT_TIMEOUT.as_secs()
                )),
                ..self.empty_record(run_id, folder, Kind::Folder)
            },
        };
        if let Err(e) = self.save(&rec) {
            tracing::warn!(run = %run_id, "saving the run's checkpoint failed: {e}");
        }
    }

    fn empty_record(&self, run_id: &str, folder: &Path, kind: Kind) -> Record {
        Record {
            run_id: run_id.to_string(),
            folder: folder.to_string_lossy().into_owned(),
            kind,
            shadow: String::new(),
            prefix: String::new(),
            created_at: crate::util::now(),
            before: None,
            after: None,
            changes: Vec::new(),
            note: None,
            undone_at: None,
        }
    }

    async fn start_record(&self, run_id: &str, folder: &Path) -> Record {
        let git_prefix = git_prefix(folder).await;
        let mut rec = match &git_prefix {
            Some(prefix) => Record {
                prefix: prefix.clone(),
                ..self.empty_record(run_id, folder, Kind::Git)
            },
            None => Record {
                shadow: self.shadow_dir(folder).to_string_lossy().into_owned(),
                ..self.empty_record(run_id, folder, Kind::Folder)
            },
        };
        match self.snapshot(&rec, None).await {
            Ok(snap) => {
                if snap.partial {
                    rec.note = Some(PARTIAL_NOTE.into());
                }
                rec.before = Some(snap);
            }
            Err(e) => {
                tracing::warn!(run = %run_id, "the run's checkpoint failed: {e}");
                rec.note = Some(match e {
                    Error::Invalid(why) => why,
                    other => format!("Sage could not back up this folder ({other}), so this run can't be undone."),
                });
            }
        }
        rec
    }

    /// The checkpoint after a run ends: the second snapshot and the change list. Errors are
    /// logged and noted on the record, never returned.
    pub async fn finish(&self, run_id: &str) {
        let Ok(Some(mut rec)) = self.load(run_id) else {
            return;
        };
        let Some(before) = rec.before.clone() else {
            return;
        };
        let folder = PathBuf::from(&rec.folder);
        {
            let lock = lock_for(&folder);
            let _held = lock.lock().await;
            let done = tokio::time::timeout(SNAPSHOT_TIMEOUT, async {
                let after = self.snapshot(&rec, Some(&before.commit)).await?;
                let changes = changes_between(&rec, &before, &after).await?;
                Ok::<_, Error>((after, changes))
            })
            .await;
            match done {
                Ok(Ok((after, changes))) => {
                    if after.partial && rec.note.is_none() {
                        rec.note = Some(PARTIAL_NOTE.into());
                    }
                    rec.after = Some(after);
                    rec.changes = changes;
                }
                Ok(Err(e)) => {
                    tracing::warn!(run = %run_id, "the run's closing checkpoint failed: {e}");
                    rec.before = None;
                    rec.note = Some(format!("Sage could not list this run's changes ({e})."));
                }
                Err(_) => {
                    rec.before = None;
                    rec.note = Some("Sage took too long to list this run's changes.".into());
                }
            }
            if let Err(e) = self.save(&rec) {
                tracing::warn!(run = %run_id, "saving the run's checkpoint failed: {e}");
            }
        }
        self.prune_now_and_then().await;
    }

    /// Snapshots the record's folder and returns the commit (child of `parent`, when given),
    /// also kept under the run's ref.
    async fn snapshot(&self, rec: &Record, parent: Option<&str>) -> Result<Snapshot> {
        let repo = rec.repo();
        let folder = PathBuf::from(&rec.folder);
        if !folder.is_dir() {
            return Err(Error::Invalid(
                "The folder is gone, so this run can't be undone.".into(),
            ));
        }
        std::fs::create_dir_all(self.tmp_dir())?;
        let (tree, big, partial) = match rec.kind {
            Kind::Git => self.git_tree(&repo, &rec.run_id).await?,
            Kind::Folder => self.shadow_tree(&repo, &folder).await?,
        };
        let which = if parent.is_some() { "after" } else { "before" };
        let message = format!("Dots checkpoint: {which} run {}", rec.run_id);
        let mut args = vec!["commit-tree", tree.as_str(), "-m", message.as_str()];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        let env: Vec<(&str, OsString)> = IDENTITY
            .iter()
            .map(|(k, v)| (*k, OsString::from(v)))
            .collect();
        let commit = String::from_utf8_lossy(&repo.ok(&args, &env, None).await?)
            .trim()
            .to_string();
        let name = format!("{REF_PREFIX}{}", rec.run_id);
        repo.ok(&["update-ref", &name, &commit], &[], None).await?;
        Ok(Snapshot {
            commit,
            big,
            partial,
        })
    }

    /// The tree of the folder in its git repository: a copy of the person's index in Dots' data
    /// folder, the folder's files added to it (`git add -A`, as the repository's ignore rules
    /// and filters say), then `write-tree`. Untracked files over `MAX_FILE_BYTES` are left out
    /// and returned. When the copy can't be used (a merge in progress, a split index), a fresh
    /// index of the folder alone is used instead.
    async fn git_tree(&self, repo: &Repo, run_id: &str) -> Result<(String, Vec<Big>, bool)> {
        let folder = repo.folder().to_path_buf();
        let listed = repo
            .ok(
                &[
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "-z",
                    "--",
                    ".",
                    &own_dir_exclude(),
                ],
                &[],
                None,
            )
            .await?;
        let mut big = Vec::new();
        let mut total = 0u64;
        let mut count = 0usize;
        for rel in String::from_utf8_lossy(&listed)
            .split('\0')
            .filter(|p| !p.is_empty())
        {
            let Ok(meta) = std::fs::symlink_metadata(folder.join(rel)) else {
                continue;
            };
            count += 1;
            if meta.len() > self.limits.max_file_bytes {
                big.push(Big {
                    path: rel.to_string(),
                    size: meta.len(),
                    mtime: meta.modified().map(millis).unwrap_or_default(),
                });
            } else {
                total += meta.len();
            }
        }
        self.limits.check(count, total)?;
        let mut spec = format!(".\0{}\0", own_dir_exclude());
        for b in &big {
            spec.push_str(&format!(":(exclude,literal){}\0", b.path));
        }
        let stamp = crate::util::new_id();
        let spec_file = self.tmp_dir().join(format!("{run_id}-{stamp}.pathspec"));
        let index = self.tmp_dir().join(format!("{run_id}-{stamp}.index"));
        std::fs::write(&spec_file, spec.as_bytes())?;
        let theirs = repo
            .text(&["rev-parse", "--path-format=absolute", "--git-path", "index"])
            .await?;
        let spec_arg = format!("--pathspec-from-file={}", spec_file.display());
        let add = [
            "add",
            "-A",
            "--ignore-errors",
            spec_arg.as_str(),
            "--pathspec-file-nul",
        ];
        let env = [("GIT_INDEX_FILE", index.clone().into_os_string())];
        let mut result = Err(Error::Other("no snapshot".into()));
        for seeded in [true, false] {
            let _ = std::fs::remove_file(&index);
            if seeded && std::fs::copy(&theirs, &index).is_err() {
                continue;
            }
            let added = repo.run(&add, &env, None).await;
            match repo.ok(&["write-tree"], &env, None).await {
                Ok(tree) => {
                    let tree = String::from_utf8_lossy(&tree).trim().to_string();
                    result = Ok((tree, std::mem::take(&mut big), !added.ok));
                    break;
                }
                Err(e) => result = Err(e),
            }
        }
        let _ = std::fs::remove_file(&index);
        let _ = std::fs::remove_file(&spec_file);
        result
    }

    /// The tree of a folder outside git, in Dots' private repository for it: every file the
    /// walk keeps, stored with its exact bytes (no filter, no ignore file), through a fresh
    /// index. Files unchanged since the last snapshot (same size and time, not touched in the
    /// seconds before it) are not read again.
    async fn shadow_tree(&self, repo: &Repo, folder: &Path) -> Result<(String, Vec<Big>, bool)> {
        let Repo::Shadow { git_dir, .. } = repo else {
            return Err(Error::Other("not a private repository".into()));
        };
        init_shadow(git_dir).await?;
        let walk_root = folder.to_path_buf();
        let (limits, own) = (self.limits, self.root.clone());
        let walked = tokio::task::spawn_blocking(move || walk(&walk_root, &own, limits))
            .await
            .map_err(|e| Error::Other(format!("reading the folder failed: {e}")))??;
        self.limits.check(walked.files.len(), walked.total)?;
        let cache_path = git_dir.join("dots-stat-cache.json");
        let cache: BTreeMap<String, Cached> = std::fs::read(&cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let now = millis(SystemTime::now());
        let mut blobs: BTreeMap<String, String> = BTreeMap::new();
        let mut to_hash: Vec<&Walked> = Vec::new();
        for f in &walked.files {
            match cache.get(&f.rel) {
                Some(c)
                    if c.size == f.size && c.mtime == f.mtime && c.hashed_at - c.mtime > 3000 =>
                {
                    blobs.insert(f.rel.clone(), c.blob.clone());
                }
                _ => to_hash.push(f),
            }
        }
        let mut partial = false;
        let hashed = hash_files(
            repo,
            folder,
            "",
            &to_hash.iter().map(|f| f.rel.clone()).collect::<Vec<_>>(),
            true,
        )
        .await;
        for (f, blob) in to_hash.iter().zip(hashed) {
            match blob {
                Some(blob) => {
                    blobs.insert(f.rel.clone(), blob);
                }
                None => partial = true,
            }
        }
        let mut info = Vec::new();
        let mut next_cache = BTreeMap::new();
        for f in &walked.files {
            let Some(blob) = blobs.get(&f.rel) else {
                continue;
            };
            let mode = if f.exec { "100755" } else { "100644" };
            info.extend_from_slice(format!("{mode} {blob}\t{}\0", f.rel).as_bytes());
            next_cache.insert(
                f.rel.clone(),
                Cached {
                    size: f.size,
                    mtime: f.mtime,
                    blob: blob.clone(),
                    hashed_at: cache
                        .get(&f.rel)
                        .filter(|c| c.blob == *blob && c.size == f.size && c.mtime == f.mtime)
                        .map_or(now, |c| c.hashed_at),
                },
            );
        }
        let index = self
            .tmp_dir()
            .join(format!("shadow-{}.index", crate::util::new_id()));
        let env = [("GIT_INDEX_FILE", index.clone().into_os_string())];
        let tree = async {
            repo.ok(&["update-index", "-z", "--index-info"], &env, Some(&info))
                .await?;
            repo.ok(&["write-tree"], &env, None).await
        }
        .await;
        let _ = std::fs::remove_file(&index);
        let tree = String::from_utf8_lossy(&tree?).trim().to_string();
        if let Ok(bytes) = serde_json::to_vec(&next_cache) {
            let tmp = cache_path.with_extension(format!("{}.tmp", crate::util::new_id()));
            if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, &cache_path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        Ok((tree, walked.big, partial || walked.unreadable))
    }

    /// The run's changes, from its record alone (no git): `None` when the run has no checkpoint
    /// (a worktree run, a run from before checkpoints, a run that has not started).
    pub fn changes(&self, run_id: &str) -> Result<Option<RunChanges>> {
        Ok(self.load(run_id)?.map(|rec| rec.summary()))
    }

    fn recorded(&self, run_id: &str) -> Result<(Record, Snapshot, Snapshot)> {
        let rec = self
            .load(run_id)?
            .ok_or_else(|| Error::NotFound(format!("run {run_id} has no record of its changes")))?;
        match (rec.before.clone(), rec.after.clone()) {
            (Some(before), Some(after)) => Ok((rec, before, after)),
            (Some(_), None) => Err(Error::Conflict(
                "this run is still working; its changes are not listed yet".into(),
            )),
            (None, _) => {
                Err(Error::Conflict(rec.note.clone().unwrap_or_else(|| {
                    "this run's changes were not recorded".into()
                })))
            }
        }
    }

    /// What undo (or, once undone, redo) would do to each file, the run's diff, and the later
    /// runs in the same folder that changed the same files.
    pub async fn preview(&self, run_id: &str) -> Result<ChangesPreview> {
        let (rec, before, after) = self.recorded(run_id)?;
        let redo = rec.undone_at.is_some();
        let files = plan(&rec, redo).await?;
        let diff = run_diff(&rec, &before, &after).await?;
        Ok(ChangesPreview {
            run_id: rec.run_id.clone(),
            redo,
            files: files.into_iter().map(|(p, _)| p).collect(),
            diff,
            later_runs: self.later_runs(&rec),
        })
    }

    /// The runs in the same folder, started after `rec`'s run, whose changes share a path with
    /// it and that were not undone.
    fn later_runs(&self, rec: &Record) -> Vec<String> {
        let paths: HashSet<&str> = rec.changes.iter().map(|c| c.path.as_str()).collect();
        let key = folder_key(Path::new(&rec.folder));
        let mut later: Vec<(String, String)> = self
            .all_records()
            .into_iter()
            .filter(|r| r.run_id != rec.run_id && r.created_at > rec.created_at)
            .filter(|r| r.undone_at.is_none() && folder_key(Path::new(&r.folder)) == key)
            .filter(|r| r.changes.iter().any(|c| paths.contains(c.path.as_str())))
            .map(|r| (r.created_at, r.run_id))
            .collect();
        later.sort();
        later.into_iter().map(|(_, id)| id).collect()
    }

    /// Puts the run's files back as they were before it (`redo` false), or as it left them
    /// (`redo` true). A file changed since is skipped unless `overwrite`. Writes only inside the
    /// run's folder, never through a link.
    pub async fn apply(&self, run_id: &str, redo: bool, overwrite: bool) -> Result<UndoOutcome> {
        let (rec, ..) = self.recorded(run_id)?;
        if rec.undone_at.is_some() != redo {
            return Err(Error::Conflict(if redo {
                "this run's changes are in place; there is nothing to redo".into()
            } else {
                "this run's changes were already undone".into()
            }));
        }
        let folder = PathBuf::from(&rec.folder);
        if !folder.is_dir() {
            return Err(Error::NotFound(format!(
                "folder {} no longer exists",
                folder.display()
            )));
        }
        let lock = lock_for(&folder);
        let _held = lock.lock().await;
        // Read again under the lock: another undo may have run in between.
        let (mut rec, ..) = self.recorded(run_id)?;
        if rec.undone_at.is_some() != redo {
            return Err(Error::Conflict(
                "this run's changes changed meanwhile; look again".into(),
            ));
        }
        let repo = rec.repo();
        let mut restored = Vec::new();
        let mut skipped = Vec::new();
        for (mut planned, change) in plan(&rec, redo).await? {
            let go = match planned.action {
                FileAction::Ready => true,
                FileAction::Conflict => overwrite,
                FileAction::Done | FileAction::Cannot => false,
            };
            if !go {
                if planned.action != FileAction::Done {
                    skipped.push(planned);
                }
                continue;
            }
            let target = if redo { &change.after } else { &change.before };
            let commit = if redo {
                rec.after.as_ref().map(|s| s.commit.clone())
            } else {
                rec.before.as_ref().map(|s| s.commit.clone())
            }
            .unwrap_or_default();
            match put_back(&repo, &rec, &commit, &change.path, target.as_ref()).await {
                Ok(()) => restored.push(planned.path),
                Err(e) => {
                    planned.action = FileAction::Cannot;
                    planned.reason = Some(e.to_string());
                    skipped.push(planned);
                }
            }
        }
        rec.undone_at = if redo { None } else { Some(crate::util::now()) };
        self.save(&rec)?;
        Ok(UndoOutcome {
            redo,
            restored,
            skipped,
            changes: rec.summary(),
        })
    }

    /// Runs `prune` at most once an hour, from `finish`.
    async fn prune_now_and_then(&self) {
        let now = chrono::Utc::now().timestamp();
        let last = LAST_PRUNE.load(Ordering::SeqCst);
        if now - last < 3600 {
            return;
        }
        if LAST_PRUNE
            .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.prune(chrono::Utc::now()).await;
        }
    }

    /// Removes checkpoints older than `KEEP_DAYS`, then the oldest private ones while Dots'
    /// private repositories hold more than `MAX_STORE_BYTES`, and private repositories no
    /// checkpoint uses. Returns how many checkpoints went.
    pub async fn prune(&self, now: chrono::DateTime<chrono::Utc>) -> usize {
        let cutoff = crate::util::timestamp(now - chrono::Duration::days(KEEP_DAYS));
        let mut records = self.all_records();
        records.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let mut removed = 0;
        let mut keep = Vec::new();
        for rec in records {
            if rec.created_at < cutoff {
                self.remove(&rec).await;
                removed += 1;
            } else {
                keep.push(rec);
            }
        }
        let shadow_root = self.root.join("shadow");
        let mut rounds = 0;
        while dir_size(&shadow_root) > MAX_STORE_BYTES && rounds < 20 {
            rounds += 1;
            let private: Vec<usize> = keep
                .iter()
                .enumerate()
                .filter(|(_, r)| r.kind == Kind::Folder)
                .map(|(i, _)| i)
                .collect();
            if private.is_empty() {
                break;
            }
            let take = private.len().div_ceil(10);
            let mut touched = HashSet::new();
            for i in private.into_iter().take(take).rev() {
                let rec = keep.remove(i);
                touched.insert(rec.shadow.clone());
                self.remove(&rec).await;
                removed += 1;
            }
            for git_dir in touched {
                let repo = Repo::Shadow {
                    git_dir: PathBuf::from(&git_dir),
                    folder: PathBuf::from(&git_dir),
                };
                let _ = repo.run(&["gc", "--prune=now", "--quiet"], &[], None).await;
            }
        }
        // A private repository no checkpoint names any more goes whole.
        let used: HashSet<String> = keep
            .iter()
            .filter(|r| r.kind == Kind::Folder)
            .map(|r| folder_key(Path::new(&r.shadow)))
            .collect();
        // Only one untouched for an hour: a snapshot that is making one has no record yet.
        if let Ok(dir) = std::fs::read_dir(&shadow_root) {
            for entry in dir.flatten() {
                if !used.contains(&folder_key(&entry.path())) && idle_for(&entry.path(), 3600) {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        // Leftovers of snapshots a crash cut short, not those of a snapshot running now.
        if let Ok(dir) = std::fs::read_dir(self.tmp_dir()) {
            for entry in dir.flatten() {
                if idle_for(&entry.path(), 3600) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        removed
    }

    async fn remove(&self, rec: &Record) {
        let name = format!("{REF_PREFIX}{}", rec.run_id);
        let folder_ok = rec.kind == Kind::Folder || Path::new(&rec.folder).is_dir();
        if folder_ok {
            let _ = rec
                .repo()
                .run(&["update-ref", "-d", &name], &[], None)
                .await;
        }
        if let Ok(path) = self.record_path(&rec.run_id) {
            let _ = std::fs::remove_file(path);
        }
    }
}

const PARTIAL_NOTE: &str =
    "Some files could not be read when Sage backed up the folder; changes to them can't be undone.";

/// `None` when `folder` is not in a git work tree; else the folder's path inside it
/// (`/`-terminated, or empty at the top).
async fn git_prefix(folder: &Path) -> Option<String> {
    let repo = Repo::Git {
        folder: folder.to_path_buf(),
    };
    let inside = repo
        .text(&["rev-parse", "--is-inside-work-tree"])
        .await
        .ok()?;
    if inside != "true" {
        return None;
    }
    // A repository's own folders are never a run's folder.
    let out = repo
        .ok(&["rev-parse", "--show-prefix"], &[], None)
        .await
        .ok()?;
    Some(
        String::from_utf8_lossy(&out)
            .trim_end_matches(['\r', '\n'])
            .to_string(),
    )
}

fn own_dir_exclude() -> String {
    format!(":(exclude,literal){OWN_DIR}")
}

async fn init_shadow(git_dir: &Path) -> Result<()> {
    if git_dir.join("HEAD").is_file() {
        return Ok(());
    }
    if let Some(parent) = git_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut cmd = Command::new("git");
    crate::proc::hide_window(&mut cmd);
    cmd.kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(["init", "--bare", "--quiet"])
        .arg(git_dir);
    let out = exec(cmd, None).await;
    if !out.ok {
        return Err(Error::Other(format!(
            "git init failed: {}",
            out.stderr.trim()
        )));
    }
    let repo = Repo::Shadow {
        git_dir: git_dir.to_path_buf(),
        folder: git_dir.to_path_buf(),
    };
    for (k, v) in [
        ("gc.auto", "0"),
        ("core.autocrlf", "false"),
        ("core.longpaths", "true"),
        ("core.fsmonitor", "false"),
    ] {
        repo.ok(&["config", k, v], &[], None).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached {
    size: u64,
    mtime: i64,
    blob: String,
    hashed_at: i64,
}

struct Walked {
    rel: String,
    size: u64,
    mtime: i64,
    exec: bool,
}

struct Walk {
    files: Vec<Walked>,
    big: Vec<Big>,
    total: u64,
    unreadable: bool,
}

/// The files of a folder outside git that a private snapshot keeps: not inside `SKIP_DIRS`, not
/// Dots' own `attachments` folder at the top, no link (never followed), none over
/// the file size limit (those are returned apart, by size and time), nor Dots' own checkpoints
/// folder `own` should the folder hold it. Stops counting past the file count limit + 1.
fn walk(root: &Path, own: &Path, limits: Limits) -> Result<Walk> {
    let mut out = Walk {
        files: Vec::new(),
        big: Vec::new(),
        total: 0,
        unreadable: false,
    };
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel_dir)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if rel_dir.is_empty() => return Err(e.into()),
            Err(_) => {
                out.unreadable = true;
                continue;
            }
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                out.unreadable = true;
                continue;
            };
            if name.contains('\n') {
                out.unreadable = true;
                continue;
            }
            let rel = if rel_dir.is_empty() {
                name.clone()
            } else {
                format!("{rel_dir}/{name}")
            };
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                out.unreadable = true;
                continue;
            };
            let kind = meta.file_type();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if SKIP_DIRS.contains(&name.as_str())
                    || (rel_dir.is_empty() && name == OWN_DIR)
                    || folder_key(&entry.path()) == folder_key(own)
                {
                    continue;
                }
                stack.push((entry.path(), rel));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let mtime = meta.modified().map(millis).unwrap_or_default();
            if meta.len() > limits.max_file_bytes {
                out.big.push(Big {
                    path: rel,
                    size: meta.len(),
                    mtime,
                });
                continue;
            }
            out.total += meta.len();
            out.files.push(Walked {
                rel,
                size: meta.len(),
                mtime,
                exec: is_exec(&meta),
            });
            if out.files.len() > limits.max_files {
                return Ok(out);
            }
        }
    }
    out.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.big.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[cfg(unix)]
fn is_exec(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_exec(_: &std::fs::Metadata) -> bool {
    false
}

/// The blob id of each file `rels` (relative to `folder`), `None` for one that could not be
/// read. `write` stores them in the repository; a private repository hashes exact bytes, the
/// person's repository applies its filters, as `git add` does. Git reads `--stdin-paths`
/// from the top of the work tree, so a path in the person's repository is given from there
/// (`prefix`, the folder's path inside it), which also lets its attributes apply.
async fn hash_files(
    repo: &Repo,
    folder: &Path,
    prefix: &str,
    rels: &[String],
    write: bool,
) -> Vec<Option<String>> {
    if rels.is_empty() {
        return Vec::new();
    }
    let shadow = matches!(repo, Repo::Shadow { .. });
    let mut args = vec!["hash-object"];
    if write {
        args.push("-w");
    }
    if shadow {
        args.push("--no-filters");
    }
    args.push("--stdin-paths");
    let path_of = |rel: &str| -> String {
        if shadow {
            folder.join(rel).to_string_lossy().into_owned()
        } else {
            format!("{prefix}{rel}")
        }
    };
    let input: String = rels.iter().map(|r| format!("{}\n", path_of(r))).collect();
    if let Ok(out) = repo.ok(&args, &[], Some(input.as_bytes())).await {
        let ids: Vec<Option<String>> = String::from_utf8_lossy(&out)
            .lines()
            .map(|l| Some(l.trim().to_string()))
            .collect();
        if ids.len() == rels.len() {
            return ids;
        }
    }
    // One file could not be read: each file on its own, so the others still count.
    let mut ids = Vec::with_capacity(rels.len());
    for rel in rels {
        let line = format!("{}\n", path_of(rel));
        let id = repo
            .ok(&args, &[], Some(line.as_bytes()))
            .await
            .ok()
            .map(|out| String::from_utf8_lossy(&out).trim().to_string())
            .filter(|id| !id.is_empty());
        ids.push(id);
    }
    ids
}

fn side(mode: &str, blob: &str) -> Option<Side> {
    (mode != "000000").then(|| Side {
        mode: mode.to_string(),
        blob: blob.to_string(),
    })
}

fn regular(side: &Option<Side>) -> bool {
    side.as_ref()
        .is_none_or(|s| s.mode == "100644" || s.mode == "100755")
}

/// The changes from `before` to `after`: what git lists between the two trees, files too large
/// to keep left to their own comparison by size and time.
async fn changes_between(rec: &Record, before: &Snapshot, after: &Snapshot) -> Result<Vec<Change>> {
    let repo = rec.repo();
    let mut args = vec![
        "diff-tree",
        "-r",
        "-z",
        "--no-renames",
        "--raw",
        "--no-abbrev",
    ];
    if rec.kind == Kind::Git {
        args.push("--relative");
    }
    let exclude = own_dir_exclude();
    args.extend([
        before.commit.as_str(),
        after.commit.as_str(),
        "--",
        ".",
        exclude.as_str(),
    ]);
    let out = repo.ok(&args, &[], None).await?;
    let big_paths: HashSet<&str> = before
        .big
        .iter()
        .chain(&after.big)
        .map(|b| b.path.as_str())
        .collect();
    let mut changes = Vec::new();
    let text = String::from_utf8_lossy(&out);
    let mut parts = text.split('\0');
    while let Some(meta) = parts.next() {
        let Some(meta) = meta.strip_prefix(':') else {
            continue;
        };
        let Some(path) = parts.next() else { break };
        let fields: Vec<&str> = meta.split(' ').collect();
        if fields.len() < 5 || big_paths.contains(path) {
            continue;
        }
        let before_side = side(fields[0], fields[2]);
        let after_side = side(fields[1], fields[3]);
        let status = match (&before_side, &after_side) {
            (None, Some(_)) => FileStatus::Added,
            (Some(_), None) => FileStatus::Deleted,
            _ => FileStatus::Modified,
        };
        let links = |s: &Option<Side>| s.as_ref().is_some_and(|s| s.mode == "120000");
        let repos = |s: &Option<Side>| s.as_ref().is_some_and(|s| s.mode == "160000");
        let (undoable, reason) = if regular(&before_side) && regular(&after_side) {
            (true, None)
        } else if repos(&before_side) || repos(&after_side) {
            (false, Some(REPO_REASON.to_string()))
        } else if links(&before_side) || links(&after_side) {
            (false, Some(LINK_REASON.to_string()))
        } else {
            (
                false,
                Some("Sage does not put this kind of file back.".to_string()),
            )
        };
        changes.push(Change {
            path: path.to_string(),
            status,
            before: before_side,
            after: after_side,
            undoable,
            reason,
        });
    }
    // Large files: compared by size and time, never undoable.
    let before_big: HashMap<&str, &Big> = before.big.iter().map(|b| (b.path.as_str(), b)).collect();
    let after_big: HashMap<&str, &Big> = after.big.iter().map(|b| (b.path.as_str(), b)).collect();
    let folder = Path::new(&rec.folder);
    let mut big_changes: BTreeMap<String, FileStatus> = BTreeMap::new();
    for (path, b) in &before_big {
        match after_big.get(path) {
            Some(a) if a.size == b.size && a.mtime == b.mtime => {}
            Some(_) => {
                big_changes.insert(path.to_string(), FileStatus::Modified);
            }
            None => {
                let status = if folder.join(path).exists() {
                    FileStatus::Modified
                } else {
                    FileStatus::Deleted
                };
                big_changes.insert(path.to_string(), status);
            }
        }
    }
    for path in after_big.keys() {
        if !before_big.contains_key(path) {
            // It may have been small before: then it was kept, and it changed all the same.
            big_changes.insert(path.to_string(), FileStatus::Modified);
        }
    }
    for (path, status) in &mut big_changes {
        if *status == FileStatus::Modified && !before_big.contains_key(path.as_str()) {
            let existed = tree_has(&repo, rec, &before.commit, path).await;
            if !existed {
                *status = FileStatus::Added;
            }
        }
    }
    for (path, status) in big_changes {
        changes.push(Change {
            path,
            status,
            before: None,
            after: None,
            undoable: false,
            reason: Some(TOO_LARGE_REASON.to_string()),
        });
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

async fn tree_has(repo: &Repo, rec: &Record, commit: &str, rel: &str) -> bool {
    let spec = format!("{commit}:{}{rel}", rec.prefix);
    repo.ok(&["cat-file", "-e", &spec], &[], None).await.is_ok()
}

/// The run's diff, patches capped as a worktree's are.
async fn run_diff(rec: &Record, before: &Snapshot, after: &Snapshot) -> Result<WorktreeDiff> {
    let repo = rec.repo();
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
    ];
    if rec.kind == Kind::Git {
        args.push("--relative");
    }
    let exclude = own_dir_exclude();
    let mut paths: Vec<&str> = rec
        .changes
        .iter()
        .filter(|c| c.reason.as_deref() != Some(TOO_LARGE_REASON))
        .map(|c| c.path.as_str())
        .collect();
    let mut files = Vec::new();
    if !paths.is_empty() {
        let literal: Vec<String> = paths.drain(..).map(|p| format!(":(literal){p}")).collect();
        args.extend([before.commit.as_str(), after.commit.as_str(), "--"]);
        args.extend(literal.iter().map(String::as_str));
        args.push(exclude.as_str());
        let out = repo.ok(&args, &[], None).await?;
        files = crate::workspace::parse_diff(&String::from_utf8_lossy(&out));
    }
    // Large files have no patch: listed all the same.
    for c in rec
        .changes
        .iter()
        .filter(|c| c.reason.as_deref() == Some(TOO_LARGE_REASON))
    {
        files.push(crate::model::FileDiff {
            path: c.path.clone(),
            status: c.status,
            patch: String::new(),
            binary: true,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut used = 0;
    let mut truncated = false;
    for f in &mut files {
        if used + f.patch.len() > crate::workspace::MAX_DIFF_BYTES {
            f.patch.clear();
            truncated = true;
        }
        used += f.patch.len();
    }
    Ok(WorktreeDiff {
        files,
        truncated,
        exists: true,
    })
}

/// What undo (or redo) would do to each of the run's files, from what is on disk now.
async fn plan(rec: &Record, redo: bool) -> Result<Vec<(PlannedFile, Change)>> {
    let repo = rec.repo();
    let folder = PathBuf::from(&rec.folder);
    let mut current: HashMap<String, Current> = HashMap::new();
    let mut to_hash = Vec::new();
    for c in rec.changes.iter().filter(|c| c.undoable) {
        let state = match safe_path(&folder, &c.path) {
            Err(e) => Current::Unsafe(e.to_string()),
            Ok(full) => match std::fs::symlink_metadata(&full) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Current::Absent,
                Err(e) => Current::Other(format!("could not be read: {e}")),
                Ok(meta) if meta.is_dir() => Current::Other("is a folder now".into()),
                Ok(meta) if !meta.is_file() => Current::Other("is a link now".into()),
                Ok(_) => {
                    to_hash.push(c.path.clone());
                    continue;
                }
            },
        };
        current.insert(c.path.clone(), state);
    }
    let ids = hash_files(&repo, &folder, &rec.prefix, &to_hash, false).await;
    for (rel, id) in to_hash.into_iter().zip(ids) {
        let state = match id {
            Some(id) => Current::Blob(id),
            None => Current::Other("could not be read".into()),
        };
        current.insert(rel, state);
    }
    let mut out = Vec::new();
    for c in &rec.changes {
        let mut planned = PlannedFile {
            path: c.path.clone(),
            status: c.status,
            action: FileAction::Cannot,
            reason: c.reason.clone(),
        };
        if c.undoable {
            let (target, expected) = if redo {
                (&c.after, &c.before)
            } else {
                (&c.before, &c.after)
            };
            let want = |s: &Option<Side>| match s {
                None => Current::Absent,
                Some(s) => Current::Blob(s.blob.clone()),
            };
            let now = current.get(&c.path).cloned().unwrap_or(Current::Absent);
            let (action, reason) = match now {
                Current::Unsafe(why) => (FileAction::Cannot, Some(why)),
                Current::Other(why) => (FileAction::Conflict, Some(format!("The file {why}."))),
                now if now == want(target) => (FileAction::Done, None),
                now if now == want(expected) => (FileAction::Ready, None),
                Current::Absent => (
                    FileAction::Conflict,
                    Some("The file was deleted after this run.".into()),
                ),
                Current::Blob(_) => (
                    FileAction::Conflict,
                    Some("The file changed after this run.".into()),
                ),
            };
            planned.action = action;
            planned.reason = reason;
        }
        out.push((planned, c.clone()));
    }
    Ok(out)
}

/// Writes `rel` as `target` says (its blob in `commit`), or deletes it when `target` is `None`.
/// The file is written beside its place and renamed over it, and only inside the run's folder:
/// the path is checked before and after its parent folders are made.
async fn put_back(
    repo: &Repo,
    rec: &Record,
    commit: &str,
    rel: &str,
    target: Option<&Side>,
) -> Result<()> {
    let folder = PathBuf::from(&rec.folder);
    let full = safe_path(&folder, rel)?;
    let Some(target) = target else {
        return match std::fs::symlink_metadata(&full) {
            Ok(meta) if meta.is_file() => Ok(std::fs::remove_file(&full)?),
            Ok(_) => Err(Error::Invalid("not a file any more".into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        };
    };
    let bytes = match rec.kind {
        // As git would check the file out: its filters and line endings applied.
        Kind::Git => {
            let spec = format!("{commit}:{}{rel}", rec.prefix);
            repo.ok(&["cat-file", "--filters", &spec], &[], None)
                .await?
        }
        Kind::Folder => {
            repo.ok(&["cat-file", "blob", &target.blob], &[], None)
                .await?
        }
    };
    let parent = full
        .parent()
        .ok_or_else(|| Error::Invalid("no parent folder".into()))?
        .to_path_buf();
    std::fs::create_dir_all(&parent)?;
    // Made folders could have raced with a link: look again, and at where the parent really is.
    safe_path(&folder, rel)?;
    let real_folder = std::fs::canonicalize(&folder)?;
    let real_parent = std::fs::canonicalize(&parent)?;
    if !real_parent.starts_with(&real_folder) {
        return Err(Error::Invalid(format!(
            "refusing to write {rel:?}: outside the folder"
        )));
    }
    let name = full
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{name}.dots-undo-{}", crate::util::new_id()));
    std::fs::write(&tmp, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if target.mode == "100755" {
            0o755
        } else {
            0o644
        };
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    if let Err(e) = std::fs::rename(&tmp, &full) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Whether `path` was last modified more than `secs` seconds ago.
fn idle_for(path: &Path, secs: u64) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() > secs)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_refuses_ways_out_of_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "./a",
            "a\\b",
            "C:/x",
            "a:stream",
            ".git/config",
            "sub/.GIT/hooks/pre-commit",
        ] {
            assert!(safe_path(root, bad).is_err(), "{bad:?} must be refused");
        }
        assert_eq!(
            safe_path(root, "a/b.txt").unwrap(),
            root.join("a").join("b.txt")
        );
        assert!(safe_path(root, ".gitignore").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn safe_path_refuses_names_windows_changes() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["con", "NUL.txt", "a/com1", "trailing.", "space "] {
            assert!(
                safe_path(dir.path(), bad).is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(safe_path(dir.path(), "console.txt").is_ok());
    }

    #[test]
    fn run_ids_are_checked_before_they_name_a_file() {
        assert!(check_id("01J0ABCDEF").is_ok());
        for bad in ["", "../x", "a/b", "a b", "x.json"] {
            assert!(check_id(bad).is_err());
        }
    }
}

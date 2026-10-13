use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use tokio::process::Command;

use crate::model::{Dot, FileDiff, FileStatus, Run, WorkspaceMode, WorktreeDiff};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
}

impl Prepared {
    pub fn from_run(run: &Run) -> Option<Prepared> {
        Some(Prepared {
            path: PathBuf::from(run.workspace_path.as_ref()?),
            branch: run.branch.clone(),
            base_commit: run.base_commit.clone(),
        })
    }
}

/// Runs `git -C <dir> <args>` and returns trimmed stdout.
pub async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    Ok(git_raw(dir, args).await?.trim().to_string())
}

/// Like [`git`], but returns stdout exactly as git wrote it. A patch needs this: its last line
/// can be a blank context line (a single space), which trimming would drop.
pub async fn git_raw(dir: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args).kill_on_drop(true);
    crate::proc::hide_window(&mut cmd);
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository a worktree belongs to, as the worktree itself records it: the git directory
/// its repository's worktrees share, in which `git -C` runs worktree and branch commands as it
/// does in the checkout. `None` when the folder is gone, is not a worktree, or has no `.git`
/// of its own: git would then search the parent folders and could answer with an unrelated
/// repository that happens to contain the data folder.
async fn repository_of(worktree: &Path) -> Option<PathBuf> {
    std::fs::symlink_metadata(worktree.join(".git")).ok()?;
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
    let dir = git(worktree, &args).await.ok()?;
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// Total patch text a diff returns; later files are listed without their patch.
pub const MAX_DIFF_BYTES: usize = 1024 * 1024;
/// Files with a NUL byte in their first 8000 bytes are treated as binary (as git does).
const BINARY_PROBE: usize = 8000;

/// Splits `git diff` output into one entry per file.
pub fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let header = header.trim_end();
            let path = header
                .rsplit_once(" b/")
                .map_or(header, |(_, b)| b)
                .to_string();
            files.push(FileDiff {
                path,
                status: FileStatus::Modified,
                patch: String::new(),
                binary: false,
            });
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        file.patch.push_str(line);
        let line = line.trim_end_matches(['\r', '\n']);
        if line.starts_with("new file mode") {
            file.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = FileStatus::Deleted;
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.status = FileStatus::Renamed;
            file.path = to.to_string();
        } else if line.starts_with("Binary files ") {
            file.binary = true;
        } else if let Some(p) = line.strip_prefix("+++ b/") {
            // git appends one TAB to header paths that contain a space.
            file.path = p.strip_suffix('\t').unwrap_or(p).to_string();
        }
    }
    for f in files.iter_mut().filter(|f| f.binary) {
        f.patch.clear();
    }
    files
}

/// A new-file patch for an untracked path with git file mode `mode`; `None` when it is binary.
fn untracked_patch(rel: &str, mode: &str, bytes: &[u8]) -> Option<String> {
    if bytes.iter().take(BINARY_PROBE).any(|b| *b == 0) {
        return None;
    }
    let text = String::from_utf8_lossy(bytes);
    let mut patch =
        format!("diff --git a/{rel} b/{rel}\nnew file mode {mode}\n--- /dev/null\n+++ b/{rel}\n");
    let lines: Vec<&str> = text.lines().collect();
    if !lines.is_empty() {
        patch.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
        for l in &lines {
            patch.push('+');
            patch.push_str(l);
            patch.push('\n');
        }
        if !text.ends_with('\n') {
            patch.push_str("\\ No newline at end of file");
        }
    }
    Some(patch)
}

/// The first part of `rel` (the whole of it included) that is a symlink or a junction, if any.
/// On Windows `git ls-files` walks into a junction and reports the files behind it, so the link
/// can sit above the path git names. Only the entries themselves are examined, never what they
/// point to.
pub fn link_on_path(root: &Path, rel: &str) -> Option<String> {
    let ends = rel.match_indices('/').map(|(i, _)| i).chain([rel.len()]);
    for end in ends {
        let part = &rel[..end];
        match std::fs::symlink_metadata(root.join(part)) {
            Ok(meta) if meta.file_type().is_symlink() => return Some(part.to_string()),
            Ok(_) => {}
            // Not there (any more): nothing below it can be read either.
            Err(_) => return None,
        }
    }
    None
}

/// One untracked path as the diff lists it: always an addition. `link` says the path is a
/// symlink or a junction; it is then shown as git shows a link (mode 120000 and the place it
/// points to) and never opened, because that place can be outside the worktree. The flag is
/// true when the file is too large to show.
pub async fn untracked_entry(root: &Path, rel: &str, link: bool) -> (FileDiff, bool) {
    let full = root.join(rel);
    let mut truncated = false;
    let (patch, binary) = if link {
        let patch = std::fs::read_link(&full)
            .ok()
            .and_then(|to| untracked_patch(rel, "120000", to.to_string_lossy().as_bytes()));
        (patch.unwrap_or_default(), false)
    } else if std::fs::metadata(&full).map_or(0, |m| m.len()) > MAX_DIFF_BYTES as u64 {
        truncated = true;
        (String::new(), false)
    } else {
        match tokio::fs::read(&full).await {
            Ok(bytes) => match untracked_patch(rel, "100644", &bytes) {
                Some(p) => (p, false),
                None => (String::new(), true),
            },
            Err(_) => (String::new(), false),
        }
    };
    let file = FileDiff {
        path: rel.to_string(),
        status: FileStatus::Added,
        patch,
        binary,
    };
    (file, truncated)
}

/// The changes of the worktree at `path` against `base_commit`: committed and uncommitted
/// changes to tracked files, then untracked files (not ignored) as additions. An untracked
/// symlink or junction is listed and not followed. Patch text is capped at `MAX_DIFF_BYTES`;
/// files past the cap are listed with an empty patch.
pub async fn diff(path: &Path, base_commit: &str) -> Result<WorktreeDiff> {
    let tracked = git_raw(
        path,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--end-of-options",
            base_commit,
            "--",
        ],
    )
    .await?;
    let mut files = parse_diff(&tracked);
    // Untrimmed: the names are separated by NUL, and a name may begin with a space.
    let listed = git_raw(
        path,
        &[
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )
    .await?;
    // A link is listed once, under its own path, however many paths git reported through it.
    let mut untracked: BTreeMap<String, bool> = BTreeMap::new();
    for rel in listed.split('\0').filter(|p| !p.is_empty()) {
        match link_on_path(path, rel) {
            Some(link) => untracked.insert(link, true),
            None => untracked.insert(rel.to_string(), false),
        };
    }
    let mut truncated = false;
    for (rel, link) in &untracked {
        let (file, cut) = untracked_entry(path, rel, *link).await;
        truncated |= cut;
        files.push(file);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut used = 0;
    for f in &mut files {
        if used + f.patch.len() > MAX_DIFF_BYTES {
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

#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    root: PathBuf,
}

impl WorkspaceManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub async fn prepare(&self, dot: &Dot, run_id: &str) -> Result<Prepared> {
        let workdir = PathBuf::from(&dot.spec.workdir);
        if !workdir.is_dir() {
            return Err(Error::Invalid(format!(
                "workdir does not exist: {}",
                workdir.display()
            )));
        }
        if dot.spec.workspace_mode == WorkspaceMode::Folder {
            return Ok(Prepared {
                path: workdir,
                branch: None,
                base_commit: None,
            });
        }
        let inside = git(&workdir, &["rev-parse", "--is-inside-work-tree"])
            .await
            .unwrap_or_default();
        if inside != "true" {
            return Err(Error::Invalid(format!(
                "workdir is not a git repository (required for worktree mode): {}",
                workdir.display()
            )));
        }
        let start = match git(
            &workdir,
            &[
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD",
            ],
        )
        .await
        {
            Ok(r) if !r.is_empty() => r,
            _ => "HEAD".to_string(),
        };
        let base = git(&workdir, &["rev-parse", &start]).await?;
        tokio::fs::create_dir_all(&self.root).await?;
        let path = self.root.join(run_id);
        let branch = format!("dots/{}/{}", dot.spec.name, run_id);
        let path_str = path.to_string_lossy().to_string();
        if let Err(e) = git(
            &workdir,
            &["worktree", "add", "-b", &branch, &path_str, &base],
        )
        .await
        {
            let _ = tokio::fs::remove_dir_all(&path).await;
            return Err(e);
        }
        Ok(Prepared {
            path,
            branch: Some(branch),
            base_commit: Some(base),
        })
    }

    /// Recreates a removed worktree at its original path so a resumed session finds the
    /// directory it remembers. Reuses the branch if it still exists, otherwise recreates it
    /// from `base_commit`. Returns `prepared` unchanged (same path, branch and base).
    pub async fn restore(&self, dot: &Dot, prepared: &Prepared) -> Result<Prepared> {
        let (Some(branch), Some(base)) = (&prepared.branch, &prepared.base_commit) else {
            return Err(Error::Invalid(
                "cannot restore a worktree without its branch and base commit".into(),
            ));
        };
        let workdir = PathBuf::from(&dot.spec.workdir);
        if !workdir.is_dir() {
            return Err(Error::Invalid(format!(
                "workdir does not exist: {}",
                workdir.display()
            )));
        }
        // Drop the stale registration of the deleted directory so the path is reusable.
        git(&workdir, &["worktree", "prune"]).await?;
        if let Some(parent) = prepared.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let path = prepared.path.to_string_lossy().to_string();
        let branch_ref = format!("refs/heads/{branch}");
        let exists = git(&workdir, &["rev-parse", "--verify", "--quiet", &branch_ref])
            .await
            .is_ok();
        let added = if exists {
            git(&workdir, &["worktree", "add", &path, branch]).await
        } else {
            git(&workdir, &["worktree", "add", "-b", branch, &path, base]).await
        };
        if let Err(e) = added {
            let _ = tokio::fs::remove_dir_all(&prepared.path).await;
            return Err(e);
        }
        Ok(prepared.clone())
    }

    pub async fn has_changes(&self, path: &Path, base_commit: &str) -> Result<bool> {
        if !git(path, &["status", "--porcelain"]).await?.is_empty() {
            return Ok(true);
        }
        let ahead = git(
            path,
            &["rev-list", "--count", &format!("{base_commit}..HEAD")],
        )
        .await?;
        Ok(ahead != "0")
    }

    /// Removes the worktree at `prepared.path`, registered in `repo`: check, git, leftover,
    /// prune. The row must pass `check_owned` before any git command runs. Git then removes
    /// the worktree; its failure is logged and does not stop the removal, because git may
    /// have failed half way (a held file) after unregistering the worktree, and a retry
    /// would only get "is not a working tree". Whatever is left of the folder (git leaves a
    /// junction where it is and still succeeds) is deleted without following any link:
    /// `remove_dir_all` removes a symlink or a junction as the link itself, never what it
    /// points to. When the folder cannot be deleted the error names it. If git failed, a
    /// `git worktree prune` clears any registration it kept.
    async fn remove_worktree(&self, repo: &Path, prepared: &Prepared) -> Result<()> {
        self.check_owned(prepared)?;
        let path = prepared.path.to_string_lossy().to_string();
        let removed = git(repo, &["worktree", "remove", "--force", &path]).await;
        if prepared.path.exists() {
            tokio::fs::remove_dir_all(&prepared.path)
                .await
                .map_err(|e| {
                    Error::Other(format!(
                        "could not remove what git left of worktree {}: {e}",
                        prepared.path.display()
                    ))
                })?;
        }
        if let Err(e) = removed {
            tracing::warn!(worktree = %path, "git could not remove the worktree: {e}");
            if let Err(e) = git(repo, &["worktree", "prune"]).await {
                tracing::warn!(worktree = %path, "pruning worktrees failed: {e}");
            }
        }
        Ok(())
    }

    pub async fn remove(&self, workdir: &Path, prepared: &Prepared) -> Result<()> {
        self.remove_worktree(workdir, prepared).await?;
        if let Some(branch) = &prepared.branch {
            git(workdir, &["branch", "-D", branch]).await?;
        }
        Ok(())
    }

    /// Checks that `prepared` is a worktree this manager made: a folder directly inside its
    /// root (one level, no `..`, not the root itself), on the branch `dots/…/<that folder's
    /// name>`. Only the text of the path and the branch is examined, so the check runs before
    /// anything touches the disk or git.
    pub fn check_owned(&self, prepared: &Prepared) -> Result<()> {
        let mut inside = prepared
            .path
            .strip_prefix(&self.root)
            .into_iter()
            .flat_map(Path::components);
        let folder = match (inside.next(), inside.next()) {
            (Some(Component::Normal(name)), None) => name.to_str(),
            _ => None,
        };
        let Some(folder) = folder else {
            return Err(Error::Invalid(format!(
                "refusing to use {}: not a dots worktree",
                prepared.path.display()
            )));
        };
        // The branch comes from the same row as the path: only accept the one dots made for
        // this worktree (`dots/<dot name>/<run id>` beside folder `<run id>`).
        let branch = prepared.branch.as_deref().unwrap_or_default();
        let ours = branch
            .strip_prefix("dots/")
            .and_then(|rest| rest.rsplit_once('/'))
            .is_some_and(|(dot, run)| !dot.is_empty() && run == folder);
        if !ours {
            return Err(Error::Invalid(format!(
                "refusing to use branch {branch}: not the dots branch of worktree {folder}"
            )));
        }
        Ok(())
    }

    /// Removes the worktree (if still there) and its branch (if it still exists), for the
    /// user's "Discard worktree" action. Harmless when both are already gone. The repository
    /// is the one the worktree itself names, because the dot's folder may have been changed
    /// since the run; `workdir` (that folder) is used only when the worktree cannot say.
    pub async fn discard(&self, workdir: &Path, prepared: &Prepared) -> Result<()> {
        let repo = match repository_of(&prepared.path).await {
            Some(repo) => repo,
            None => workdir.to_path_buf(),
        };
        if prepared.path.exists() {
            self.remove_worktree(&repo, prepared).await?;
        }
        git(&repo, &["worktree", "prune"]).await?;
        if let Some(branch) = &prepared.branch {
            let branch_ref = format!("refs/heads/{branch}");
            if git(&repo, &["rev-parse", "--verify", "--quiet", &branch_ref])
                .await
                .is_ok()
            {
                git(&repo, &["branch", "-D", branch]).await?;
            }
        }
        Ok(())
    }

    /// Removes a worktree that has no uncommitted changes and no new commits.
    pub async fn cleanup_if_unchanged(&self, dot: &Dot, prepared: &Prepared) -> Result<bool> {
        if dot.spec.workspace_mode != WorkspaceMode::Worktree {
            return Ok(false);
        }
        let (Some(_), Some(base)) = (&prepared.branch, &prepared.base_commit) else {
            return Ok(false);
        };
        // The row comes from the database: it must name a worktree of dots before git runs
        // anything in it.
        self.check_owned(prepared)?;
        if !prepared.path.exists() || self.has_changes(&prepared.path, base).await? {
            return Ok(false);
        }
        self.remove(Path::new(&dot.spec.workdir), prepared).await?;
        Ok(true)
    }
}

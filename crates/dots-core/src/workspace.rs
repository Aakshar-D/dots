use std::path::{Path, PathBuf};

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
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
            file.path = p.to_string();
        }
    }
    for f in files.iter_mut().filter(|f| f.binary) {
        f.patch.clear();
    }
    files
}

/// A new-file patch for an untracked file; `None` when it is binary.
fn untracked_patch(rel: &str, bytes: &[u8]) -> Option<String> {
    if bytes.iter().take(BINARY_PROBE).any(|b| *b == 0) {
        return None;
    }
    let text = String::from_utf8_lossy(bytes);
    let mut patch =
        format!("diff --git a/{rel} b/{rel}\nnew file mode 100644\n--- /dev/null\n+++ b/{rel}\n");
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

/// The changes of the worktree at `path` against `base_commit`: committed and uncommitted
/// changes to tracked files, then untracked files (not ignored) as additions. Patch text is
/// capped at `MAX_DIFF_BYTES`; files past the cap are listed with an empty patch.
pub async fn diff(path: &Path, base_commit: &str) -> Result<WorktreeDiff> {
    let tracked = git(
        path,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            base_commit,
            "--",
        ],
    )
    .await?;
    let mut files = parse_diff(&tracked);
    let listed = git(
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
    let mut untracked: Vec<&str> = listed.split('\0').filter(|p| !p.is_empty()).collect();
    untracked.sort();
    let mut truncated = false;
    for rel in untracked {
        let full = path.join(rel);
        let size = std::fs::metadata(&full).map_or(0, |m| m.len());
        let (patch, binary) = if size > MAX_DIFF_BYTES as u64 {
            truncated = true;
            (String::new(), false)
        } else {
            match tokio::fs::read(&full).await {
                Ok(bytes) => match untracked_patch(rel, &bytes) {
                    Some(p) => (p, false),
                    None => (String::new(), true),
                },
                Err(_) => (String::new(), false),
            }
        };
        files.push(FileDiff {
            path: rel.to_string(),
            status: FileStatus::Added,
            patch,
            binary,
        });
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
    Ok(WorktreeDiff { files, truncated })
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

    pub async fn remove(&self, workdir: &Path, prepared: &Prepared) -> Result<()> {
        let path = prepared.path.to_string_lossy().to_string();
        git(workdir, &["worktree", "remove", "--force", &path]).await?;
        if let Some(branch) = &prepared.branch {
            git(workdir, &["branch", "-D", branch]).await?;
        }
        Ok(())
    }

    /// Removes the worktree (if still there) and its branch (if it still exists), for the
    /// user's "Discard worktree" action. Harmless when both are already gone.
    pub async fn discard(&self, workdir: &Path, prepared: &Prepared) -> Result<()> {
        if prepared.path.exists() {
            let path = prepared.path.to_string_lossy().to_string();
            git(workdir, &["worktree", "remove", "--force", &path]).await?;
        }
        git(workdir, &["worktree", "prune"]).await?;
        if let Some(branch) = &prepared.branch {
            let branch_ref = format!("refs/heads/{branch}");
            if git(workdir, &["rev-parse", "--verify", "--quiet", &branch_ref])
                .await
                .is_ok()
            {
                git(workdir, &["branch", "-D", branch]).await?;
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
        if !prepared.path.exists() || self.has_changes(&prepared.path, base).await? {
            return Ok(false);
        }
        self.remove(Path::new(&dot.spec.workdir), prepared).await?;
        Ok(true)
    }
}

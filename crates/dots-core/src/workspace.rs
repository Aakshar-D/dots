use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::model::{Dot, Run, WorkspaceMode};
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

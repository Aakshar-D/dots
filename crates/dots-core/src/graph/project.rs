//! What a project is (project graph spec section 3): the main checkout of the repository a
//! dot works in, or its folder outside git, and the key its graph file is named by.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::workspace::git;

/// A project: its root, the key of its graph, and whether it is a git checkout (a folder
/// outside git gets no graph built on its own: spec section 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Project {
    pub root: PathBuf,
    pub key: String,
    pub git: bool,
}

impl Project {
    /// The project of `root`, taken as it is (already resolved), outside git.
    pub fn at(root: PathBuf) -> Project {
        let key = key_of(&root);
        Project {
            root,
            key,
            git: false,
        }
    }

    /// The project of the git checkout at `root` (already resolved).
    pub fn at_git(root: PathBuf) -> Project {
        Project {
            git: true,
            ..Project::at(root)
        }
    }

    /// The project of a run's own linked worktree at `path`: its own graph, seeded from the
    /// main checkout's (spec section 5). `None` when the folder is missing.
    pub fn worktree(path: &Path) -> Option<Project> {
        Some(Project::at_git(canonical(path)?))
    }

    /// The project a dot whose `workdir` is `dir` works in; `None` when the folder is missing.
    pub async fn resolve(dir: &Path) -> Option<Project> {
        let dir = canonical(dir)?;
        Some(match git_root(&dir).await {
            Some(root) => Project::at_git(canonical(&root).unwrap_or(root)),
            None => Project::at(dir),
        })
    }

    /// `dir` (a folder at or below the root) relative to the root, with `/`; `""` for the
    /// root itself, `None` outside it.
    pub fn relative(&self, dir: &Path) -> Option<String> {
        let dir = canonical(dir)?;
        let rel = dir.strip_prefix(&self.root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }
}

/// The first 16 hex digits of the SHA-256 of `root`'s text, lower-cased on Windows where
/// paths ignore case.
pub fn key_of(root: &Path) -> String {
    let mut text = root.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `path` canonicalized, without Windows' `\\?\` prefix.
fn canonical(path: &Path) -> Option<PathBuf> {
    let c = std::fs::canonicalize(path).ok()?;
    if !c.is_dir() {
        return None;
    }
    let text = c.to_string_lossy();
    Some(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    })
}

/// The main checkout of the repository `dir` is in: the parent of the shared git folder when
/// it is named `.git` (so a linked worktree gives the main checkout), else the top level of
/// `dir`'s own checkout. `None` outside git.
async fn git_root(dir: &Path) -> Option<PathBuf> {
    let common = match git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await
    {
        Ok(common) => PathBuf::from(common),
        // Git before 2.31 has no `--path-format`: its answer may be relative to `dir`.
        Err(_) => absolutize(
            dir,
            &git(dir, &["rev-parse", "--git-common-dir"]).await.ok()?,
        ),
    };
    if common.file_name().is_some_and(|n| n == ".git") {
        if let Some(parent) = common.parent() {
            if parent.is_dir() {
                return Some(parent.to_path_buf());
            }
        }
    }
    let top = git(dir, &["rev-parse", "--show-toplevel"]).await.ok()?;
    (!top.is_empty()).then(|| PathBuf::from(top))
}

/// The path git printed, relative to `dir` when it is not absolute.
fn absolutize(dir: &Path, printed: &str) -> PathBuf {
    let p = PathBuf::from(printed.trim());
    if p.is_absolute() {
        p
    } else {
        dir.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{:?}", out);
    }

    #[tokio::test]
    async fn a_worktree_and_a_subfolder_share_the_main_checkout() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("sub/deeper")).unwrap();
        run(&repo, &["init", "-q"]);
        run(&repo, &["config", "user.email", "t@t"]);
        run(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        run(&repo, &["add", "."]);
        run(&repo, &["commit", "-q", "-m", "first"]);
        let wt = tmp.path().join("wt");
        run(
            &repo,
            &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "side"],
        );

        let main = Project::resolve(&repo).await.unwrap();
        let from_wt = Project::resolve(&wt).await.unwrap();
        let from_sub = Project::resolve(&repo.join("sub/deeper")).await.unwrap();
        assert_eq!(main, from_wt);
        assert_eq!(main, from_sub);
        assert_eq!(main.key.len(), 16);
        assert!(main.root.ends_with("repo"), "{}", main.root.display());
        assert!(main.git);
        // The worktree itself is a project of its own, for a run's graph.
        let own = Project::worktree(&wt).unwrap();
        assert!(own.git);
        assert_ne!(own.key, main.key);
        assert_eq!(main.relative(&repo.join("sub")).as_deref(), Some("sub"));
        assert_eq!(main.relative(&repo).as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_plain_folder_is_its_own_project_and_a_missing_one_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let a = Project::resolve(tmp.path()).await.unwrap();
        assert!(!a.git);
        assert!(a.root.ends_with(tmp.path().file_name().unwrap()));
        assert!(Project::resolve(&tmp.path().join("gone")).await.is_none());
    }

    #[test]
    fn an_old_gits_relative_answer_is_made_absolute() {
        let dir = Path::new("/repo/sub");
        assert_eq!(absolutize(dir, "../.git"), Path::new("/repo/sub/../.git"));
        let abs = std::env::temp_dir();
        assert_eq!(absolutize(dir, abs.to_str().unwrap()), abs);
    }

    #[test]
    fn keys_are_stable_and_differ_by_root() {
        assert_eq!(key_of(Path::new("/a/b")), key_of(Path::new("/a/b")));
        assert_ne!(key_of(Path::new("/a/b")), key_of(Path::new("/a/c")));
    }
}

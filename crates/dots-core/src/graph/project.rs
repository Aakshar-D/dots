//! What a project is (project graph spec section 3): the main checkout of the repository a
//! dot works in, or its folder outside git, and the key its graph file is named by.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::workspace::git;

/// A project: its root and the key of its graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Project {
    pub root: PathBuf,
    pub key: String,
}

impl Project {
    /// The project of `root`, taken as it is (already resolved).
    pub fn at(root: PathBuf) -> Project {
        let key = key_of(&root);
        Project { root, key }
    }

    /// The project a dot whose `workdir` is `dir` works in; `None` when the folder is missing.
    pub async fn resolve(dir: &Path) -> Option<Project> {
        let dir = canonical(dir)?;
        let root = match git_root(&dir).await {
            Some(root) => canonical(&root).unwrap_or(root),
            None => dir,
        };
        Some(Project::at(root))
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
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await
    .ok()?;
    let common = PathBuf::from(common);
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
    }

    #[tokio::test]
    async fn a_plain_folder_is_its_own_project_and_a_missing_one_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let a = Project::resolve(tmp.path()).await.unwrap();
        assert!(a.root.ends_with(tmp.path().file_name().unwrap()));
        assert!(Project::resolve(&tmp.path().join("gone")).await.is_none());
    }

    #[test]
    fn keys_are_stable_and_differ_by_root() {
        assert_eq!(key_of(Path::new("/a/b")), key_of(Path::new("/a/b")));
        assert_ne!(key_of(Path::new("/a/b")), key_of(Path::new("/a/c")));
    }
}

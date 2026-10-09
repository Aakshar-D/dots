mod common;

use common::{fake_dot, git_out, init_repo};
use dots_core::model::WorkspaceMode;
use dots_core::workspace::WorkspaceManager;
use dots_core::Error;

#[tokio::test]
async fn folder_mode_uses_workdir_directly() {
    let tmp = tempfile::tempdir().unwrap();
    let dot = fake_dot("f", tmp.path(), WorkspaceMode::Folder);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert_eq!(p.path, tmp.path());
    assert!(p.branch.is_none() && p.base_commit.is_none());
    assert!(!wm.cleanup_if_unchanged(&dot, &p).await.unwrap());
    assert!(tmp.path().exists());
}

#[tokio::test]
async fn missing_workdir_is_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let dot = fake_dot("m", &tmp.path().join("nope"), WorkspaceMode::Folder);
    let err = WorkspaceManager::new(tmp.path().join("wt"))
        .prepare(&dot, "R1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn worktree_mode_requires_a_git_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let dot = fake_dot("g", &plain, WorkspaceMode::Worktree);
    let err = WorkspaceManager::new(tmp.path().join("wt"))
        .prepare(&dot, "R1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn worktree_is_created_on_a_new_branch_from_head() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let head = git_out(&repo, &["rev-parse", "HEAD"]);
    let dot = fake_dot("triage", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert_eq!(p.path, tmp.path().join("wt").join("R1"));
    assert!(p.path.join("README.md").exists());
    assert_eq!(p.branch.as_deref(), Some("dots/triage/R1"));
    assert_eq!(p.base_commit.as_deref(), Some(head.as_str()));
    assert!(!git_out(&repo, &["branch", "--list", "dots/triage/R1"]).is_empty());
    assert!(!wm.has_changes(&p.path, &head).await.unwrap());
}

#[tokio::test]
async fn unchanged_worktree_is_removed_with_its_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("clean", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert!(wm.cleanup_if_unchanged(&dot, &p).await.unwrap());
    assert!(!p.path.exists());
    assert!(git_out(&repo, &["branch", "--list", "dots/clean/R1"]).is_empty());
}

#[tokio::test]
async fn changed_worktrees_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("dirty", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));

    let uncommitted = wm.prepare(&dot, "R1").await.unwrap();
    std::fs::write(uncommitted.path.join("new.txt"), "x").unwrap();
    assert!(!wm.cleanup_if_unchanged(&dot, &uncommitted).await.unwrap());
    assert!(uncommitted.path.exists());

    let committed = wm.prepare(&dot, "R2").await.unwrap();
    std::fs::write(committed.path.join("c.txt"), "y").unwrap();
    git_out(&committed.path, &["add", "."]);
    git_out(&committed.path, &["commit", "-q", "-m", "work"]);
    assert!(wm
        .has_changes(&committed.path, committed.base_commit.as_deref().unwrap())
        .await
        .unwrap());
    assert!(!wm.cleanup_if_unchanged(&dot, &committed).await.unwrap());
}

#[tokio::test]
async fn restore_recreates_a_removed_worktree_at_the_same_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("again", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));

    // Branch deleted by cleanup: recreated from the base commit.
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert!(wm.cleanup_if_unchanged(&dot, &p).await.unwrap());
    assert!(!p.path.exists());
    let restored = wm.restore(&dot, &p).await.unwrap();
    assert_eq!(restored, p);
    assert!(p.path.join("README.md").is_file());
    assert_eq!(
        git_out(&p.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "dots/again/R1"
    );
    assert_eq!(
        git_out(&p.path, &["rev-parse", "HEAD"]),
        p.base_commit.clone().unwrap()
    );

    // Directory deleted behind git's back, branch still there (with a commit): reused.
    std::fs::write(p.path.join("c.txt"), "y").unwrap();
    git_out(&p.path, &["add", "."]);
    git_out(&p.path, &["commit", "-q", "-m", "work"]);
    let tip = git_out(&p.path, &["rev-parse", "HEAD"]);
    std::fs::remove_dir_all(&p.path).unwrap();
    wm.restore(&dot, &p).await.unwrap();
    assert_eq!(git_out(&p.path, &["rev-parse", "HEAD"]), tip);
}

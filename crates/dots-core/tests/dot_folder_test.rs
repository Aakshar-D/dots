//! A new dot may leave its folder blank, and never gets worktree mode outside git.

mod common;

use std::path::PathBuf;

use common::{init_repo, spec};
use dots_core::model::WorkspaceMode;
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};
use tempfile::TempDir;

const REFUSAL: &str =
    "This folder is not a git repository. Choose 'The folder itself' or pick a git folder.";

struct T {
    dir: TempDir,
    root: PathBuf,
    rt: Runtime,
}

async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let root = dir.path().join("Dots");
    cfg.new_dot_root = root.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    T { dir, root, rt }
}

fn same(a: &str, b: &std::path::Path) -> bool {
    std::path::Path::new(a) == b
}

#[tokio::test]
async fn blank_workdir_gets_default_folder() {
    let t = start().await;
    assert!(!t.root.exists());
    let mut s = spec("Inbox helper");
    s.workdir = "  ".into();
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let want = t.root.join("inbox-helper");
    assert!(same(&dot.spec.workdir, &want), "{}", dot.spec.workdir);
    assert!(want.is_dir());
    // A plain folder is not a repository, so the dot runs in the folder itself.
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
}

#[tokio::test]
async fn default_folder_skips_non_empty() {
    let t = start().await;
    let taken = t.root.join("inbox-helper");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("a.txt"), "x").unwrap();
    let mut s = spec("Inbox helper");
    s.workdir = String::new();
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    assert!(same(&dot.spec.workdir, &t.root.join("inbox-helper-2")));

    // An existing empty folder is reused.
    std::fs::create_dir_all(t.root.join("other")).unwrap();
    let mut s = spec("Other");
    s.workdir = String::new();
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    assert!(same(&dot.spec.workdir, &t.root.join("other")));
}

#[tokio::test]
async fn worktree_on_plain_folder_becomes_folder_at_create() {
    let t = start().await;
    let plain = t.dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let mut s = spec("Plain");
    s.workdir = plain.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
}

#[tokio::test]
async fn worktree_on_repo_stays() {
    let t = start().await;
    let repo = t.dir.path().join("repo");
    init_repo(&repo);
    let mut s = spec("Repo");
    s.workdir = repo.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Worktree);
}

#[tokio::test]
async fn update_refuses_worktree_on_plain_folder() {
    let t = start().await;
    let plain = t.dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let mut s = spec("Plain");
    s.workdir = plain.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let mut next = dot.spec.clone();
    next.workspace_mode = WorkspaceMode::Worktree;
    match t.rt.update_dot(&dot.id, next).await {
        Err(Error::Invalid(m)) => assert_eq!(m, REFUSAL),
        other => panic!("expected refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn update_blank_workdir_refused() {
    let t = start().await;
    let plain = t.dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let mut s = spec("Plain");
    s.workdir = plain.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let mut next = dot.spec.clone();
    next.workdir = String::new();
    match t.rt.update_dot(&dot.id, next).await {
        Err(Error::Invalid(m)) => assert_eq!(m, "workdir must not be empty"),
        other => panic!("expected invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn stored_worktree_dot_without_repo_keeps_other_fields_editable() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let plain = dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let dot = {
        let store = Store::open(&data.join("dots.db")).await.unwrap();
        let mut s = spec("Gone");
        s.workdir = plain.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Worktree;
        store.create_dot(&s, None, "default").await.unwrap()
    };
    let mut cfg = Config::new(data);
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();

    let mut edited = dot.spec.clone();
    edited.instructions = "Do something else.".into();
    let saved = rt.update_dot(&dot.id, edited).await.unwrap();
    assert_eq!(saved.spec.instructions, "Do something else.");
    assert_eq!(saved.spec.workspace_mode, WorkspaceMode::Worktree);

    let other = dir.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut moved = saved.spec.clone();
    moved.workdir = other.to_string_lossy().to_string();
    match rt.update_dot(&dot.id, moved).await {
        Err(Error::Invalid(m)) => assert_eq!(m, REFUSAL),
        other => panic!("expected refusal, got {other:?}"),
    }
    rt.shutdown();
}

#[tokio::test]
async fn failed_create_keeps_an_empty_folder_it_did_not_make() {
    let t = start().await;
    // A dot named "Inbox helper" exists, with a folder elsewhere.
    let elsewhere = t.dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let mut s = spec("Inbox helper");
    s.workdir = elsewhere.to_string_lossy().to_string();
    t.rt.create_dot(s, None, None).await.unwrap();
    // The default folder of the name is there already, empty.
    let there = t.root.join("inbox-helper");
    std::fs::create_dir_all(&there).unwrap();
    let mut s = spec("inbox HELPER");
    s.workdir = String::new();
    let err = t.rt.create_dot(s, None, None).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert!(there.is_dir());
}

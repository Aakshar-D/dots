//! Checkpoints of folder runs (undo spec): the change list, undo and redo, conflicts, path
//! safety and size limits, in a git repository and in a plain folder.

mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::{fake_dot, git_out, init_repo, temp_store, wait_status};
use dots_core::checkpoint::{ChangesState, Checkpoints, FileAction, Limits};
use dots_core::engine::{Engine, EngineEvent, RunContext};
use dots_core::events::new_bus;
use dots_core::model::{EngineKind, FileStatus, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::runner::{Runner, RunnerConfig};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn statuses(c: &dots_core::checkpoint::RunChanges) -> Vec<(String, FileStatus, bool)> {
    c.files
        .iter()
        .map(|f| (f.path.clone(), f.status, f.undoable))
        .collect()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// A data folder and a work folder, side by side.
struct Setup {
    _data: TempDir,
    work: TempDir,
    cp: Checkpoints,
}

fn setup() -> Setup {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let cp = Checkpoints::new(data.path().join("checkpoints"));
    Setup {
        _data: data,
        work,
        cp,
    }
}

/// A repo whose checkouts write files byte for byte, whatever the machine's global
/// `core.autocrlf` says (the line-endings test sets its own).
fn plain_repo(dir: &Path) {
    init_repo(dir);
    git_out(dir, &["config", "core.autocrlf", "false"]);
}

/// The run's work: changes one file, creates one, deletes one.
fn do_the_work(dir: &Path) {
    std::fs::write(dir.join("README.md"), "hello, changed\n").unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/new.txt"), "made by the run\n").unwrap();
    std::fs::remove_file(dir.join("old.txt")).unwrap();
}

#[tokio::test]
async fn a_git_checkpoint_leaves_the_persons_index_branches_and_stash_alone() {
    let s = setup();
    let repo = s.work.path().join("repo");
    plain_repo(&repo);
    std::fs::write(repo.join("old.txt"), "old\n").unwrap();
    git_out(&repo, &["add", "old.txt"]);
    git_out(&repo, &["commit", "-q", "-m", "old"]);
    // A stash entry, a second branch, a staged change and an untracked file of the person's.
    std::fs::write(repo.join("README.md"), "stashed\n").unwrap();
    git_out(&repo, &["stash", "-q"]);
    git_out(&repo, &["branch", "feature"]);
    std::fs::write(repo.join("staged.txt"), "staged\n").unwrap();
    git_out(&repo, &["add", "staged.txt"]);
    std::fs::write(repo.join("notes.txt"), "mine, untracked\n").unwrap();

    let index_before = std::fs::read(repo.join(".git/index")).unwrap();
    let stash = git_out(&repo, &["stash", "list"]);
    let branches = git_out(
        &repo,
        &[
            "for-each-ref",
            "refs/heads",
            "--format=%(refname) %(objectname)",
        ],
    );
    let head = git_out(&repo, &["rev-parse", "HEAD"]);
    let cached = git_out(&repo, &["diff", "--cached", "--name-status"]);

    s.cp.begin("RUN1", &repo).await;
    assert_eq!(
        s.cp.changes("RUN1").unwrap().unwrap().state,
        ChangesState::Recording
    );
    do_the_work(&repo);
    s.cp.finish("RUN1").await;

    assert_eq!(
        std::fs::read(repo.join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(git_out(&repo, &["stash", "list"]), stash);
    assert_eq!(
        git_out(
            &repo,
            &[
                "for-each-ref",
                "refs/heads",
                "--format=%(refname) %(objectname)"
            ]
        ),
        branches
    );
    assert_eq!(git_out(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git_out(&repo, &["diff", "--cached", "--name-status"]),
        cached
    );
    // The checkpoint lives under its own ref: the run's "after", child of its "before".
    let after = git_out(&repo, &["rev-parse", "refs/dots/checkpoints/RUN1"]);
    git_out(&repo, &["rev-parse", &format!("{after}^")]);

    let changes = s.cp.changes("RUN1").unwrap().unwrap();
    assert_eq!(changes.state, ChangesState::Recorded);
    assert!(changes.git);
    assert_eq!(
        statuses(&changes),
        vec![
            ("README.md".into(), FileStatus::Modified, true),
            ("old.txt".into(), FileStatus::Deleted, true),
            ("src/new.txt".into(), FileStatus::Added, true),
        ]
    );
    // The person's own untracked and staged files did not change in the run: not listed.
    let preview = s.cp.preview("RUN1").await.unwrap();
    assert!(!preview.redo);
    assert!(preview.files.iter().all(|f| f.action == FileAction::Ready));
    let readme = preview
        .diff
        .files
        .iter()
        .find(|f| f.path == "README.md")
        .unwrap();
    assert!(readme.patch.contains("+hello, changed"));
}

#[tokio::test]
async fn undo_puts_created_modified_and_deleted_files_back_and_redo_undoes_that_in_git() {
    let s = setup();
    let repo = s.work.path().join("repo");
    plain_repo(&repo);
    std::fs::write(repo.join("old.txt"), "old\n").unwrap();
    git_out(&repo, &["add", "old.txt"]);
    git_out(&repo, &["commit", "-q", "-m", "old"]);
    let index_before = std::fs::read(repo.join(".git/index")).unwrap();

    s.cp.begin("RUN1", &repo).await;
    do_the_work(&repo);
    s.cp.finish("RUN1").await;

    let undone = s.cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(undone.restored.len(), 3, "{undone:?}");
    assert!(undone.skipped.is_empty());
    assert!(undone.changes.undone_at.is_some());
    assert_eq!(read(&repo.join("README.md")), "hello\n");
    assert_eq!(read(&repo.join("old.txt")), "old\n");
    assert!(!repo.join("src/new.txt").exists());
    assert_eq!(
        std::fs::read(repo.join(".git/index")).unwrap(),
        index_before
    );
    assert!(
        s.cp.apply("RUN1", false, false).await.is_err(),
        "undone twice"
    );

    let preview = s.cp.preview("RUN1").await.unwrap();
    assert!(preview.redo);
    let redone = s.cp.apply("RUN1", true, false).await.unwrap();
    assert_eq!(redone.restored.len(), 3);
    assert_eq!(read(&repo.join("README.md")), "hello, changed\n");
    assert_eq!(read(&repo.join("src/new.txt")), "made by the run\n");
    assert!(!repo.join("old.txt").exists());
    assert!(s.cp.changes("RUN1").unwrap().unwrap().undone_at.is_none());
}

#[tokio::test]
async fn a_subfolder_of_a_repo_lists_only_its_own_files() {
    let s = setup();
    let repo = s.work.path().join("repo");
    plain_repo(&repo);
    let sub = repo.join("app");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("a.txt"), "a\n").unwrap();

    s.cp.begin("RUN1", &sub).await;
    std::fs::write(sub.join("a.txt"), "a2\n").unwrap();
    std::fs::write(repo.join("README.md"), "outside the folder\n").unwrap();
    s.cp.finish("RUN1").await;

    let changes = s.cp.changes("RUN1").unwrap().unwrap();
    // Untracked before and after, and changed: listed as the folder sees it.
    assert_eq!(
        statuses(&changes),
        vec![("a.txt".into(), FileStatus::Modified, true)]
    );
    let outcome = s.cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(outcome.restored, vec!["a.txt".to_string()], "{outcome:?}");
    assert_eq!(read(&sub.join("a.txt")), "a\n");
    assert_eq!(read(&repo.join("README.md")), "outside the folder\n");
}

#[tokio::test]
async fn undo_restores_the_line_endings_git_checks_out() {
    let s = setup();
    let repo = s.work.path().join("repo");
    init_repo(&repo);
    git_out(&repo, &["config", "core.autocrlf", "true"]);
    std::fs::write(repo.join("crlf.txt"), "one\r\ntwo\r\n").unwrap();
    git_out(&repo, &["add", "crlf.txt"]);
    git_out(&repo, &["commit", "-q", "-m", "crlf"]);
    // Checked out again, as git writes it.
    std::fs::remove_file(repo.join("crlf.txt")).unwrap();
    git_out(&repo, &["checkout", "--", "crlf.txt"]);
    let original = std::fs::read(repo.join("crlf.txt")).unwrap();

    s.cp.begin("RUN1", &repo).await;
    std::fs::write(repo.join("crlf.txt"), "one\r\nthree\r\n").unwrap();
    s.cp.finish("RUN1").await;

    let preview = s.cp.preview("RUN1").await.unwrap();
    assert_eq!(preview.files[0].action, FileAction::Ready, "{preview:?}");
    s.cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(std::fs::read(repo.join("crlf.txt")).unwrap(), original);
}

#[tokio::test]
async fn a_plain_folder_is_kept_in_a_private_repository_and_undone() {
    let s = setup();
    let dir = s.work.path();
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    std::fs::write(dir.join("old.txt"), "old\n").unwrap();
    std::fs::write(dir.join("bytes.bin"), [0u8, 159, 146, 150, 13, 10]).unwrap();
    std::fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    std::fs::write(dir.join("node_modules/pkg/index.js"), "a").unwrap();

    s.cp.begin("RUN1", dir).await;
    do_the_work(dir);
    std::fs::write(dir.join("bytes.bin"), [1u8, 2, 3]).unwrap();
    // Dependencies are not tracked.
    std::fs::write(dir.join("node_modules/pkg/index.js"), "b").unwrap();
    s.cp.finish("RUN1").await;

    let changes = s.cp.changes("RUN1").unwrap().unwrap();
    assert!(!changes.git);
    assert_eq!(
        statuses(&changes),
        vec![
            ("README.md".into(), FileStatus::Modified, true),
            ("bytes.bin".into(), FileStatus::Modified, true),
            ("old.txt".into(), FileStatus::Deleted, true),
            ("src/new.txt".into(), FileStatus::Added, true),
        ]
    );
    // Nothing git-like was made in the person's folder.
    assert!(!dir.join(".git").exists());

    let undone = s.cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(undone.restored.len(), 4);
    assert_eq!(read(&dir.join("README.md")), "hello\n");
    assert_eq!(read(&dir.join("old.txt")), "old\n");
    assert_eq!(
        std::fs::read(dir.join("bytes.bin")).unwrap(),
        vec![0u8, 159, 146, 150, 13, 10]
    );
    assert!(!dir.join("src/new.txt").exists());
    assert_eq!(read(&dir.join("node_modules/pkg/index.js")), "b");
}

#[tokio::test]
async fn a_second_run_reuses_the_private_repository() {
    let s = setup();
    let dir = s.work.path();
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    s.cp.begin("RUN1", dir).await;
    std::fs::write(dir.join("a.txt"), "2\n").unwrap();
    s.cp.finish("RUN1").await;
    s.cp.begin("RUN2", dir).await;
    std::fs::write(dir.join("a.txt"), "3\n").unwrap();
    s.cp.finish("RUN2").await;

    // The later run changed the same file: undoing the first warns of it, and the file is a
    // conflict (it is no longer as the first run left it).
    let preview = s.cp.preview("RUN1").await.unwrap();
    assert_eq!(preview.later_runs, vec!["RUN2".to_string()]);
    assert_eq!(preview.files[0].action, FileAction::Conflict);
    assert!(s.cp.preview("RUN2").await.unwrap().later_runs.is_empty());

    s.cp.apply("RUN2", false, false).await.unwrap();
    assert_eq!(read(&dir.join("a.txt")), "2\n");
    // Undone, the later run no longer counts.
    assert!(s.cp.preview("RUN1").await.unwrap().later_runs.is_empty());
    s.cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(read(&dir.join("a.txt")), "1\n");
}

#[tokio::test]
async fn a_file_changed_after_the_run_is_skipped_unless_overwrite() {
    let s = setup();
    let dir = s.work.path();
    std::fs::write(dir.join("a.txt"), "before\n").unwrap();
    std::fs::write(dir.join("b.txt"), "before\n").unwrap();
    s.cp.begin("RUN1", dir).await;
    std::fs::write(dir.join("a.txt"), "run\n").unwrap();
    std::fs::write(dir.join("b.txt"), "run\n").unwrap();
    std::fs::write(dir.join("c.txt"), "run\n").unwrap();
    s.cp.finish("RUN1").await;
    // The person edits a.txt and deletes c.txt after the run.
    std::fs::write(dir.join("a.txt"), "person\n").unwrap();
    std::fs::remove_file(dir.join("c.txt")).unwrap();

    let preview = s.cp.preview("RUN1").await.unwrap();
    let action = |p: &str| preview.files.iter().find(|f| f.path == p).unwrap().action;
    assert_eq!(action("a.txt"), FileAction::Conflict);
    assert_eq!(action("b.txt"), FileAction::Ready);
    // c.txt is gone already: as undo would leave it.
    assert_eq!(action("c.txt"), FileAction::Done);

    let outcome = s.cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(outcome.restored, vec!["b.txt".to_string()]);
    assert_eq!(outcome.skipped.len(), 1);
    assert_eq!(outcome.skipped[0].path, "a.txt");
    assert_eq!(read(&dir.join("a.txt")), "person\n");
    assert_eq!(read(&dir.join("b.txt")), "before\n");

    // Redo, then undo again with overwrite: the person's edit is replaced.
    s.cp.apply("RUN1", true, false).await.unwrap();
    std::fs::write(dir.join("a.txt"), "person again\n").unwrap();
    let outcome = s.cp.apply("RUN1", false, true).await.unwrap();
    assert!(
        outcome.restored.contains(&"a.txt".to_string()),
        "{outcome:?}"
    );
    assert_eq!(read(&dir.join("a.txt")), "before\n");
}

#[tokio::test]
async fn large_files_are_listed_but_can_not_be_undone() {
    let s = setup();
    let cp = s.cp.clone().with_limits(Limits {
        max_file_bytes: 10,
        ..Limits::default()
    });
    let dir = s.work.path();
    std::fs::write(dir.join("big.log"), "0123456789abcdef").unwrap();
    std::fs::write(dir.join("small.txt"), "x").unwrap();
    cp.begin("RUN1", dir).await;
    std::fs::write(dir.join("big.log"), "0123456789abcdefg").unwrap();
    std::fs::write(dir.join("new-big.log"), "0123456789abcdefg").unwrap();
    std::fs::write(dir.join("small.txt"), "y").unwrap();
    cp.finish("RUN1").await;

    let changes = cp.changes("RUN1").unwrap().unwrap();
    assert_eq!(
        statuses(&changes),
        vec![
            ("big.log".into(), FileStatus::Modified, false),
            ("new-big.log".into(), FileStatus::Added, false),
            ("small.txt".into(), FileStatus::Modified, true),
        ]
    );
    assert!(changes.files[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("can't be put back"));
    let outcome = cp.apply("RUN1", false, false).await.unwrap();
    assert_eq!(outcome.restored, vec!["small.txt".to_string()]);
    assert_eq!(outcome.skipped.len(), 2);
    assert!(dir.join("new-big.log").exists());
}

#[tokio::test]
async fn a_folder_over_the_limits_is_not_snapshotted_and_says_why() {
    let s = setup();
    let dir = s.work.path();
    for i in 0..5 {
        std::fs::write(dir.join(format!("{i}.txt")), "x").unwrap();
    }
    let cp = s.cp.clone().with_limits(Limits {
        max_files: 3,
        ..Limits::default()
    });
    cp.begin("RUN1", dir).await;
    cp.finish("RUN1").await;
    let changes = cp.changes("RUN1").unwrap().unwrap();
    assert_eq!(changes.state, ChangesState::Unavailable);
    assert!(changes.note.unwrap().contains("more than 3 files"));
    assert!(cp.apply("RUN1", false, false).await.is_err());

    let cp = s.cp.clone().with_limits(Limits {
        max_total_bytes: 2,
        ..Limits::default()
    });
    cp.begin("RUN2", dir).await;
    assert_eq!(
        cp.changes("RUN2").unwrap().unwrap().state,
        ChangesState::Unavailable
    );
}

/// Makes `link` a link to the folder `to`: a junction on Windows, a symlink elsewhere.
fn link_dir(to: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(to)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(to, link).unwrap();
}

#[tokio::test]
async fn undo_never_writes_through_a_link_out_of_the_folder() {
    let s = setup();
    let dir = s.work.path().join("work");
    let outside = s.work.path().join("outside");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("file.txt"), "outside\n").unwrap();
    cp_run_creating_sub_file(&s.cp, &dir).await;
    // After the run, `sub` becomes a link to a folder outside with a file of the same name.
    std::fs::remove_dir_all(dir.join("sub")).unwrap();
    link_dir(&outside, &dir.join("sub"));

    let preview = s.cp.preview("RUN1").await.unwrap();
    assert_eq!(preview.files[0].action, FileAction::Cannot, "{preview:?}");
    let outcome = s.cp.apply("RUN1", false, true).await.unwrap();
    assert!(outcome.restored.is_empty());
    assert_eq!(read(&outside.join("file.txt")), "outside\n");
}

async fn cp_run_creating_sub_file(cp: &Checkpoints, dir: &Path) {
    cp.begin("RUN1", dir).await;
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/file.txt"), "run\n").unwrap();
    cp.finish("RUN1").await;
}

#[tokio::test]
async fn old_checkpoints_are_pruned_with_their_ref() {
    let s = setup();
    let repo = s.work.path().join("repo");
    init_repo(&repo);
    s.cp.begin("RUN1", &repo).await;
    std::fs::write(repo.join("README.md"), "x\n").unwrap();
    s.cp.finish("RUN1").await;
    assert_eq!(s.cp.prune(chrono::Utc::now()).await, 0);
    assert!(s.cp.changes("RUN1").unwrap().is_some());

    let later = chrono::Utc::now() + chrono::Duration::days(31);
    assert_eq!(s.cp.prune(later).await, 1);
    assert!(s.cp.changes("RUN1").unwrap().is_none());
    assert_eq!(git_out(&repo, &["for-each-ref", "refs/dots"]), "");
}

#[tokio::test]
async fn a_run_id_that_is_not_one_is_refused() {
    let s = setup();
    assert!(s.cp.changes("../x").is_err());
    assert!(s.cp.apply("a/b", false, false).await.is_err());
}

/// An engine that writes a file in its folder, then finishes.
struct Writer;

#[async_trait]
impl Engine for Writer {
    async fn start(&self, ctx: RunContext) -> dots_core::Result<mpsc::Receiver<EngineEvent>> {
        let (tx, rx) = mpsc::channel(8);
        std::fs::write(ctx.workspace.join("made.txt"), "by the dot\n").unwrap();
        std::fs::write(ctx.workspace.join("keep.txt"), "changed by the dot\n").unwrap();
        tokio::spawn(async move {
            let _ = tx
                .send(EngineEvent::Finished {
                    summary: "done".into(),
                })
                .await;
        });
        Ok(rx)
    }
}

#[tokio::test]
async fn a_folder_run_records_its_changes_and_a_worktree_run_does_not() {
    let (dir, store) = temp_store().await;
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("keep.txt"), "mine\n").unwrap();
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(Writer));
    let runner = Runner::new(
        store.clone(),
        new_bus(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("worktrees"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let spec = fake_dot("writer", work.path(), WorkspaceMode::Folder).spec;
    let dot = store.create_dot(&spec, None, "default").await.unwrap();
    let run = runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&store, &run.id, RunStatus::Succeeded, 30).await;

    let cp = runner.checkpoints();
    let changes = cp.changes(&run.id).unwrap().unwrap();
    assert_eq!(
        statuses(&changes),
        vec![
            ("keep.txt".into(), FileStatus::Modified, true),
            ("made.txt".into(), FileStatus::Added, true),
        ]
    );
    cp.apply(&run.id, false, false).await.unwrap();
    assert_eq!(read(&work.path().join("keep.txt")), "mine\n");
    assert!(!work.path().join("made.txt").exists());

    // A worktree run has its branch instead.
    let repo = tempfile::tempdir().unwrap();
    init_repo(repo.path());
    let spec = fake_dot("worktree", repo.path(), WorkspaceMode::Worktree).spec;
    let wt_dot = store.create_dot(&spec, None, "default").await.unwrap();
    let run = runner
        .enqueue(NewRun::new(&wt_dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&store, &run.id, RunStatus::Succeeded, 30).await;
    assert!(cp.changes(&run.id).unwrap().is_none());
}

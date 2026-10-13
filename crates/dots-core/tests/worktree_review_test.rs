mod common;
mod llm;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{git_out, init_repo, spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::model::{FileStatus, Run, RunStatus, WorkspaceMode};
#[cfg(windows)]
use dots_core::policy::{Policy, Preset};
use dots_core::workspace::{parse_diff, MAX_DIFF_BYTES};
use dots_core::{Config, Error, Runtime};
use llm::{call, calls, text, Llm};
use serde_json::json;
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    repo: PathBuf,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, repo, rt }
}

/// A local worktree dot whose model edits README.md, writes new.txt, then finishes.
async fn edited_run(t: &Rt) -> Run {
    let llm = Llm::start(vec![
        calls(&[
            (
                "c1",
                "edit_file",
                json!({"file_path": "README.md", "old_string": "hello", "new_string": "hello world"}),
            ),
            ("c2", "write_file", json!({"file_path": "new.txt", "content": "a\nb"})),
        ]),
        text("Edited."),
    ])
    .await;
    let mut s = spec("editor");
    s.workdir = t.repo.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    s.engine = dots_core::model::EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "m".into();
    let dot = t.rt.create_dot(s).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    let run = wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    drop(llm);
    run
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    !git_out(repo, &["branch", "--list", branch]).is_empty()
}

#[tokio::test]
async fn the_diff_shows_tracked_and_untracked_changes() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    std::fs::write(ws.join("blob.bin"), [0u8, 1, 2, 0, 3]).unwrap();

    let diff = t.rt.worktree_diff(&run.id).await.unwrap();
    assert!(!diff.truncated);
    let paths: Vec<&str> = diff.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["README.md", "blob.bin", "new.txt"]);
    let readme = &diff.files[0];
    assert_eq!(readme.status, FileStatus::Modified);
    assert!(
        readme.patch.contains("-hello\n+hello world"),
        "{}",
        readme.patch
    );
    let blob = &diff.files[1];
    assert_eq!(
        (blob.status, blob.binary, blob.patch.as_str()),
        (FileStatus::Added, true, "")
    );
    let new = &diff.files[2];
    assert_eq!(new.status, FileStatus::Added);
    assert!(
        new.patch
            .ends_with("@@ -0,0 +1,2 @@\n+a\n+b\n\\ No newline at end of file"),
        "{}",
        new.patch
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_worktree_run_announces_its_workspace_while_it_runs() {
    let t = start().await;
    let mut events = t.rt.subscribe();
    let run = edited_run(&t).await;
    let mut announced = false;
    while let Ok(event) = events.try_recv() {
        if let RuntimeEvent::RunUpdated { run: seen } = event {
            announced |= seen.id == run.id
                && seen.status == RunStatus::Running
                && seen.branch == run.branch
                && seen.workspace_path == run.workspace_path;
        }
    }
    assert!(
        announced,
        "no RunUpdated carried the branch and folder of the running run"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_patch_keeps_its_trailing_blank_context_line() {
    let t = start().await;
    std::fs::write(t.repo.join("notes.txt"), "one\ntwo\n\n").unwrap();
    git_out(&t.repo, &["add", "notes.txt"]);
    git_out(&t.repo, &["commit", "-q", "-m", "notes"]);
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    std::fs::write(ws.join("notes.txt"), "uno\ntwo\n\n").unwrap();

    let diff = t.rt.worktree_diff(&run.id).await.unwrap();
    let notes = diff.files.iter().find(|f| f.path == "notes.txt").unwrap();
    // The hunk is "-one +uno two <blank>": the blank context line is a single space.
    assert!(
        notes
            .patch
            .ends_with("@@ -1,3 +1,3 @@\n-one\n+uno\n two\n \n"),
        "{:?}",
        notes.patch
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_huge_diff_is_truncated_but_lists_every_file() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    let line = "x".repeat(99) + "\n";
    std::fs::write(ws.join("big.txt"), line.repeat(MAX_DIFF_BYTES / 100 + 10)).unwrap();
    std::fs::write(ws.join("zz.txt"), "after\n").unwrap();
    let diff = t.rt.worktree_diff(&run.id).await.unwrap();
    assert!(diff.truncated);
    let total: usize = diff.files.iter().map(|f| f.patch.len()).sum();
    assert!(total <= MAX_DIFF_BYTES);
    let big = diff.files.iter().find(|f| f.path == "big.txt").unwrap();
    assert!(big.patch.is_empty());
    assert!(diff.files.iter().any(|f| f.path == "zz.txt"));
    t.rt.shutdown();
}

#[tokio::test]
async fn discarding_removes_the_worktree_and_branch_and_clears_the_inbox() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    let branch = run.branch.clone().unwrap();
    assert!(ws.is_dir() && branch_exists(&t.repo, &branch));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    assert_eq!(t.rt.run_folder(&run.id).await.unwrap(), ws);
    assert!(t.rt.worktree_diff(&run.id).await.unwrap().exists);

    let after = t.rt.discard_worktree(&run.id).await.unwrap();
    assert!(after.reviewed_at.is_some());
    assert!(!ws.exists());
    assert!(!branch_exists(&t.repo, &branch));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    // The removed worktree has nothing left to show or open.
    let gone = t.rt.worktree_diff(&run.id).await.unwrap();
    assert!(!gone.exists && gone.files.is_empty());
    assert!(matches!(
        t.rt.run_folder(&run.id).await,
        Err(Error::NotFound(_))
    ));
    // Discarding twice is harmless.
    t.rt.discard_worktree(&run.id).await.unwrap();
    t.rt.shutdown();
}

#[tokio::test]
async fn folder_runs_have_no_worktree_to_diff_or_discard() {
    let t = start().await;
    let mut s = spec("plain");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = t.rt.create_dot(s).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
    assert!(matches!(
        t.rt.worktree_diff(&run.id).await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        t.rt.discard_worktree(&run.id).await,
        Err(Error::Invalid(_))
    ));
    assert_eq!(t.rt.run_folder(&run.id).await.unwrap(), t.dir.path());
    t.rt.shutdown();
}

/// Rewrites one column of a run row behind the runtime's back, as a tampered database would.
async fn tamper(t: &Rt, run_id: &str, column: &str, value: &str) {
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        t.dir.path().join("data").join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query(&format!("UPDATE runs SET {column} = ? WHERE id = ?"))
        .bind(value)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn diff_and_discard_refuse_a_path_that_is_not_a_direct_child_of_the_worktrees_folder() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    let branch = run.branch.clone().unwrap();
    let worktrees = ws.parent().unwrap().to_path_buf();
    let name = ws.file_name().unwrap().to_string_lossy().to_string();
    std::fs::create_dir(ws.join("sub")).unwrap();
    let tampered = [
        // The user's own checkout.
        t.repo.clone(),
        // The worktrees folder itself, a folder deeper inside it, and a way round through `..`.
        worktrees.clone(),
        ws.join("sub"),
        worktrees.join("elsewhere").join("..").join(&name),
        // Outside and not on disk: still refused, not reported as a removed worktree.
        t.dir.path().join("nowhere"),
    ];
    for path in &tampered {
        let path = path.to_string_lossy().to_string();
        tamper(&t, &run.id, "workspace_path", &path).await;
        let err = t.rt.worktree_diff(&run.id).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "diff of {path}: {err}");
        let err = t.rt.discard_worktree(&run.id).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "discard of {path}: {err}");
        assert!(t.repo.join("README.md").is_file());
        assert!(ws.join("new.txt").is_file() && branch_exists(&t.repo, &branch));
    }
    assert!(t.rt.get_run(&run.id).await.unwrap().reviewed_at.is_none());

    // The true row still works, and a worktree folder that is simply gone is not an error.
    tamper(&t, &run.id, "workspace_path", &ws.to_string_lossy()).await;
    assert!(t.rt.worktree_diff(&run.id).await.unwrap().exists);
    std::fs::remove_dir_all(&ws).unwrap();
    let gone = t.rt.worktree_diff(&run.id).await.unwrap();
    assert!(!gone.exists && gone.files.is_empty() && !gone.truncated);
    t.rt.shutdown();
}

#[tokio::test]
async fn diff_and_discard_refuse_a_branch_dots_did_not_create() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    let name = ws.file_name().unwrap().to_string_lossy().to_string();
    git_out(&t.repo, &["branch", "feature-x"]);
    for tampered in [
        "feature-x".to_string(),
        "dots/x/not-this-run".to_string(),
        format!("dots/{name}"),
    ] {
        tamper(&t, &run.id, "branch", &tampered).await;
        let err = t.rt.worktree_diff(&run.id).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{tampered}: {err}");
        let err = t.rt.discard_worktree(&run.id).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{tampered}: {err}");
        assert!(branch_exists(&t.repo, "feature-x"));
        assert!(ws.is_dir());
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn discard_finds_the_repository_from_the_worktree_when_the_dot_moved() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    let branch = run.branch.clone().unwrap();
    // After the run, the dot is pointed at another repository.
    let other = t.dir.path().join("other");
    init_repo(&other);
    let mut spec = t.rt.get_dot(&run.dot_id).await.unwrap().spec;
    spec.workdir = other.to_string_lossy().to_string();
    t.rt.update_dot(&run.dot_id, spec).await.unwrap();

    let after = t.rt.discard_worktree(&run.id).await.unwrap();
    assert!(after.reviewed_at.is_some());
    assert!(!ws.exists());
    assert!(!branch_exists(&t.repo, &branch));
    // The repository the worktree came from no longer lists it.
    let listed = git_out(&t.repo, &["worktree", "list", "--porcelain"]);
    assert_eq!(listed.matches("worktree ").count(), 1, "{listed}");
    assert_eq!(git_out(&other, &["branch", "--list"]), "* main");
    t.rt.shutdown();
}

/// A folder outside every worktree and repository, holding `keep.txt`.
#[cfg(windows)]
fn outside_folder(t: &Rt) -> PathBuf {
    let outside = t.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "keep me\n").unwrap();
    outside
}

/// Git removes a worktree but leaves a junction inside it where it is, and still reports
/// success. A junction needs no privilege, so these tests always run on Windows.
#[cfg(windows)]
#[tokio::test]
async fn discarding_a_worktree_with_a_junction_removes_the_folder_and_spares_the_target() {
    let t = start().await;
    let run = edited_run(&t).await;
    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    let branch = run.branch.clone().unwrap();
    let outside = outside_folder(&t);
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(ws.join("junc"))
        .arg(&outside)
        .output()
        .unwrap();
    assert!(made.status.success());
    assert!(ws.join("junc").join("keep.txt").is_file());

    let after = t.rt.discard_worktree(&run.id).await.unwrap();
    assert!(after.reviewed_at.is_some());
    assert!(!ws.exists(), "the worktree folder was left behind");
    assert!(!branch_exists(&t.repo, &branch));
    // Only the junction went: what it pointed to is as it was.
    assert_eq!(
        std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep me\n"
    );
    // The removed worktree reads as removed, and a second discard is still harmless.
    let gone = t.rt.worktree_diff(&run.id).await.unwrap();
    assert!(!gone.exists && gone.files.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    t.rt.discard_worktree(&run.id).await.unwrap();
    t.rt.shutdown();
}

/// What pnpm makes of `node_modules`: a junction the repository ignores. The run changed
/// nothing git can see, so its worktree is cleaned up when it ends.
#[cfg(windows)]
#[tokio::test]
async fn an_unchanged_run_with_an_ignored_junction_is_cleaned_up_and_stays_out_of_the_inbox() {
    let t = start().await;
    std::fs::write(t.repo.join(".gitignore"), "node_modules\n").unwrap();
    git_out(&t.repo, &["add", ".gitignore"]);
    git_out(&t.repo, &["commit", "-q", "-m", "ignore node_modules"]);
    let outside = outside_folder(&t);
    let link = format!("cmd /c mklink /J node_modules '{}'", outside.display());
    let llm = Llm::start(vec![
        call("c1", "shell", json!({ "command": link })),
        text("Linked."),
    ])
    .await;
    let mut s = spec("linker");
    s.workdir = t.repo.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    s.engine = dots_core::model::EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "m".into();
    s.policy = Policy::preset(Preset::Trusted);
    let dot = t.rt.create_dot(s).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    let run = wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;

    // The dot did make the junction.
    let events = t.rt.list_run_events(&run.id, 0).await.unwrap();
    let result = events.iter().find(|e| e.kind == "tool_result").unwrap();
    assert_eq!(result.data["is_error"], json!(false), "{}", result.data);
    assert!(
        result.data["output"]
            .as_str()
            .unwrap()
            .contains("Junction created"),
        "{}",
        result.data
    );

    let ws = PathBuf::from(run.workspace_path.as_deref().unwrap());
    assert!(!ws.exists(), "the worktree folder was left behind");
    assert!(!branch_exists(&t.repo, run.branch.as_deref().unwrap()));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "keep me\n"
    );
    t.rt.shutdown();
}

#[test]
fn parse_diff_reads_status_paths_and_binary_markers() {
    let text = "diff --git a/old name.txt b/new name.txt\n\
similarity index 90%\n\
rename from old name.txt\n\
rename to new name.txt\n\
diff --git a/gone.rs b/gone.rs\n\
deleted file mode 100644\n\
index 1111111..0000000\n\
--- a/gone.rs\n\
+++ /dev/null\n\
@@ -1 +0,0 @@\n\
-fn x() {}\n\
diff --git a/logo.png b/logo.png\n\
index 1111111..2222222 100644\n\
Binary files a/logo.png and b/logo.png differ\n\
diff --git a/my file.txt b/my file.txt\n\
index 1111111..2222222 100644\n\
--- a/my file.txt\t\n\
+++ b/my file.txt\t\n\
@@ -1 +1 @@\n\
-a\n\
+b\n";
    let files = parse_diff(text);
    assert_eq!(files.len(), 4);
    assert_eq!(
        (files[3].path.as_str(), files[3].status),
        ("my file.txt", FileStatus::Modified)
    );
    assert_eq!(
        (files[0].path.as_str(), files[0].status),
        ("new name.txt", FileStatus::Renamed)
    );
    assert_eq!(
        (files[1].path.as_str(), files[1].status),
        ("gone.rs", FileStatus::Deleted)
    );
    assert!(files[1].patch.ends_with("-fn x() {}\n"));
    assert_eq!(
        (files[2].path.as_str(), files[2].status, files[2].binary),
        ("logo.png", FileStatus::Modified, true)
    );
}

#[tokio::test]
async fn discard_waits_for_the_thread_to_finish() {
    let t = start().await;
    let llm = Llm::start(vec![
        call(
            "c1",
            "write_file",
            json!({"file_path": "a.txt", "content": "a"}),
        ),
        text("late").set_delay(Duration::from_secs(5)),
    ])
    .await;
    let mut s = spec("slowpoke");
    s.workdir = t.repo.to_string_lossy().to_string();
    s.engine = dots_core::model::EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "m".into();
    let dot = t.rt.create_dot(s).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Running, 15).await;
    assert!(matches!(
        t.rt.discard_worktree(&run.id).await,
        Err(Error::Conflict(_))
    ));
    t.rt.cancel_run(&run.id).await.unwrap();
    t.rt.shutdown();
}

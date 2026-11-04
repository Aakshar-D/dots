//! Sending files with a chat message (attachments spec sections 4.3 and 4.4): linking in the
//! run's own transaction, names within a topic, the checks, the workspace copy before every run
//! of the topic, its safety, and Retry. Runs go through the fake CLI, which records the files
//! under `attachments/` as it starts.

mod common;

use std::path::{Path, PathBuf};

use chrono::Utc;
use common::{git_out, init_repo, spec, wait_status};
use dots_core::chat_files::{COPY_FAILED, GONE, TOO_MANY};
use dots_core::model::{
    Delivery, Dot, NewRun, Run, RunStatus, StagedAttachment, TriggerKind, WorkspaceMode,
};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
    args_out: PathBuf,
}

/// A runtime on the fake CLI; its queue held when `held`.
async fn start(held: bool) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    let rt = Runtime::start(cfg).await.unwrap();
    if held {
        rt.hold_runs(true).await;
    }
    Rt { dir, rt, args_out }
}

fn work(t: &Rt) -> PathBuf {
    t.dir.path().join("work")
}

async fn folder_dot(t: &Rt, name: &str) -> Dot {
    std::fs::create_dir_all(work(t)).unwrap();
    let mut s = spec(name);
    s.workdir = work(t).to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    t.rt.create_dot(s, None, None).await.unwrap()
}

async fn stage(t: &Rt, dot: &Dot, name: &str, text: &str) -> StagedAttachment {
    t.rt.add_attachment_bytes(&dot.id, name, text.as_bytes())
        .await
        .unwrap()
}

fn ids(files: &[&StagedAttachment]) -> Vec<String> {
    files.iter().map(|f| f.id.clone()).collect()
}

fn names(run: &Run) -> Vec<String> {
    run.payload.as_ref().unwrap()["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_string())
        .collect()
}

/// The files the fake CLI found under `attachments/` as it started.
fn seen(t: &Rt) -> Vec<String> {
    let args: Value = serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    args["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

/// Ends a queued run (queue held) as succeeded with a session, by the store.
async fn end(t: &Rt, run: &Run) -> Run {
    let store = t.rt.store();
    assert!(store.claim_run(&run.id).await.unwrap());
    store.mark_delivered(&run.id, Utc::now()).await.unwrap();
    store.set_session(&run.id, "s1").await.unwrap();
    store
        .finish_run(&run.id, RunStatus::Succeeded, Some("done"), None)
        .await
        .unwrap()
}

async fn run_count(t: &Rt, dot: &Dot) -> usize {
    t.rt.store()
        .list_runs(Some(&dot.id), 100)
        .await
        .unwrap()
        .len()
}

fn message(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

#[tokio::test]
async fn a_message_is_never_claimed_without_its_files() {
    let t = start(false).await;
    let dot = folder_dot(&t, "reader").await;
    let a = stage(&t, &dot, "a.txt", "A").await;
    let b = stage(&t, &dot, "b.txt", "B").await;
    let run =
        t.rt.chat_with_files(&dot.id, "Read these", None, &ids(&[&a, &b]))
            .await
            .unwrap();
    assert_eq!(names(&run), vec!["a.txt", "b.txt"]);
    let ended = wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    assert_eq!(ended.delivery, Some(Delivery::Delivered));
    let found = seen(&t);
    for f in [".gitignore", "a.txt", "b.txt"] {
        assert!(found.contains(&format!("{}/{f}", run.id)), "{found:?}");
    }
    let args: Value = serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    let prompt = args["prompt"].as_str().unwrap();
    assert!(prompt.contains("## Attached files"), "{prompt}");
    assert!(prompt.contains("a.txt (1 B)"), "{prompt}");
}

#[tokio::test]
async fn names_are_unique_within_a_topic() {
    let t = start(true).await;
    let dot = folder_dot(&t, "namer").await;
    let first = stage(&t, &dot, "report.pdf", "1").await;
    let p =
        t.rt.chat_with_files(&dot.id, "One", None, &ids(&[&first]))
            .await
            .unwrap();
    let p = end(&t, &p).await;
    let upper = stage(&t, &dot, "REPORT.pdf", "2").await;
    let lower = stage(&t, &dot, "report.pdf", "3").await;
    let ignore = stage(&t, &dot, ".gitignore", "4").await;
    let reply =
        t.rt.chat_with_files(
            &dot.id,
            "Two",
            Some(&p.id),
            &ids(&[&upper, &lower, &ignore]),
        )
        .await
        .unwrap();
    assert_eq!(
        names(&reply),
        vec!["REPORT (2).pdf", "report (3).pdf", ".gitignore (2)"]
    );
    let row = t.rt.store().get_attachment(&lower.id).await.unwrap();
    assert_eq!(row.name.as_deref(), Some("report (3).pdf"));
    assert_eq!(row.run_id.as_deref(), Some(reply.id.as_str()));
}

/// A reply in a watcher's topic: the email's files share the topic folder, so their names are
/// taken (attachments spec section 3.1.5).
#[tokio::test]
async fn a_reply_in_a_watcher_topic_does_not_take_an_email_file_name() {
    let t = start(true).await;
    let dot = folder_dot(&t, "mailer").await;
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.payload = Some(json!({
        "message": "an email",
        "attachments": [{"name": "invoice.pdf", "path": "x"}, {"name": "notes.txt"}],
    }));
    new.workspace_path = Some(work(&t).to_string_lossy().to_string());
    let root = t.rt.store().create_run(&new).await.unwrap();
    let root = end(&t, &root).await;
    let mine = stage(&t, &dot, "invoice.pdf", "mine").await;
    let notes = stage(&t, &dot, "NOTES.txt", "mine too").await;
    let reply =
        t.rt.chat_with_files(&dot.id, "fixed", Some(&root.id), &ids(&[&mine, &notes]))
            .await
            .unwrap();
    assert_eq!(names(&reply), vec!["invoice (2).pdf", "NOTES (2).txt"]);
}

/// A file the dot made in the topic folder takes its name: an attached file of the same name
/// gets another, and never stands for the dot's file.
#[tokio::test]
async fn a_file_the_dot_made_in_the_topic_folder_takes_its_name() {
    let t = start(false).await;
    let dot = folder_dot(&t, "maker").await;
    let a = stage(&t, &dot, "a.txt", "A").await;
    let first =
        t.rt.chat_with_files(&dot.id, "one", None, &ids(&[&a]))
            .await
            .unwrap();
    let first = wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 30).await;
    let folder = work(&t).join("attachments").join(&first.id);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("summary.txt"), "the dot's").unwrap();
    let mine = stage(&t, &dot, "summary.txt", "the person's").await;
    let reply =
        t.rt.chat_with_files(&dot.id, "two", Some(&first.id), &ids(&[&mine]))
            .await
            .unwrap();
    assert_eq!(names(&reply), vec!["summary (2).txt"]);
}

/// Something else at the name of this message's own file, when the run starts: the run fails
/// rather than point the dot at the wrong file. Dots' own unchanged copy there (a run started
/// again) is fine.
#[tokio::test]
async fn another_file_at_a_new_attachments_name_fails_the_run() {
    for (there, fails) in [("someone else's", true), ("A", false)] {
        let t = start(true).await;
        let dot = folder_dot(&t, "strict").await;
        let a = stage(&t, &dot, "a.txt", "A").await;
        let run =
            t.rt.chat_with_files(&dot.id, "read", None, &ids(&[&a]))
                .await
                .unwrap();
        let folder = work(&t).join("attachments").join(&run.id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), there).unwrap();
        t.rt.hold_runs(false).await;
        if fails {
            let failed = wait_status(t.rt.store(), &run.id, RunStatus::Failed, 30).await;
            let error = failed.error.clone().unwrap_or_default();
            assert!(error.starts_with(COPY_FAILED), "{error}");
            assert!(!t.args_out.exists(), "the engine started");
        } else {
            wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
        }
        assert_eq!(
            std::fs::read_to_string(folder.join("a.txt")).unwrap(),
            there
        );
    }
}

#[tokio::test]
async fn limits_and_ownership_are_checked() {
    let t = start(true).await;
    let dot = folder_dot(&t, "owner").await;
    let mut s = spec("other");
    s.workdir = work(&t).to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let other = t.rt.create_dot(s, None, None).await.unwrap();

    let eleven: Vec<String> = (0..11).map(|i| format!("X{i}")).collect();
    let e =
        t.rt.chat_with_files(&dot.id, "hi", None, &eleven)
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(message(e), TOO_MANY);

    let theirs = stage(&t, &other, "theirs.txt", "x").await;
    let e =
        t.rt.chat_with_files(&dot.id, "hi", None, &ids(&[&theirs]))
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");

    let sent = stage(&t, &dot, "sent.txt", "x").await;
    t.rt.chat_with_files(&dot.id, "first", None, &ids(&[&sent]))
        .await
        .unwrap();
    let before = run_count(&t, &dot).await;
    let e =
        t.rt.chat_with_files(&dot.id, "again", None, &ids(&[&sent]))
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");

    let gone = stage(&t, &dot, "gone.txt", "x").await;
    std::fs::remove_dir_all(t.dir.path().join("data").join("chat-files").join(&gone.id)).unwrap();
    let e =
        t.rt.chat_with_files(&dot.id, "hi", None, &ids(&[&gone]))
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");
    assert_eq!(message(e), GONE);

    let e =
        t.rt.chat_with_files(&dot.id, "hi", None, &["NOSUCH".to_string()])
            .await
            .unwrap_err();
    assert_eq!(message(e), GONE);
    assert_eq!(run_count(&t, &dot).await, before);
    assert_eq!(run_count(&t, &other).await, 0);
}

#[tokio::test]
async fn a_message_may_be_files_only() {
    let t = start(true).await;
    let dot = folder_dot(&t, "quiet").await;
    let a = stage(&t, &dot, "a.txt", "x").await;
    let run =
        t.rt.chat_with_files(&dot.id, "", None, &ids(&[&a]))
            .await
            .unwrap();
    assert_eq!(run.payload.as_ref().unwrap()["message"], json!(""));
    let e =
        t.rt.chat_with_files(&dot.id, "  ", None, &[])
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
}

#[tokio::test]
async fn two_sends_of_the_same_files_link_once() {
    let t = start(true).await;
    let dot = folder_dot(&t, "twice").await;
    let a = stage(&t, &dot, "a.txt", "x").await;
    let fresh = stage(&t, &dot, "fresh.txt", "y").await;
    let first =
        t.rt.chat_with_files(&dot.id, "one", None, &ids(&[&a]))
            .await
            .unwrap();
    let e =
        t.rt.chat_with_files(&dot.id, "two", None, &ids(&[&a]))
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");
    let row = t.rt.store().get_attachment(&a.id).await.unwrap();
    assert_eq!(row.run_id.as_deref(), Some(first.id.as_str()));
    // The store's own transaction: a link that loses rolls back the run and the other links.
    let before = run_count(&t, &dot).await;
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "three"}));
    let links = [
        (fresh.id.clone(), "fresh.txt".to_string()),
        (a.id.clone(), "a.txt".to_string()),
    ];
    let e =
        t.rt.store()
            .create_chat_run(&new, &links, None, None)
            .await
            .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");
    assert_eq!(run_count(&t, &dot).await, before);
    let fresh_row = t.rt.store().get_attachment(&fresh.id).await.unwrap();
    assert_eq!(fresh_row.run_id, None);
    assert_eq!(fresh_row.name, None);
    let row = t.rt.store().get_attachment(&a.id).await.unwrap();
    assert_eq!(row.run_id.as_deref(), Some(first.id.as_str()));
}

#[tokio::test]
async fn chat_files_survive_the_end_of_a_run() {
    let t = start(false).await;
    let dot = folder_dot(&t, "keeper").await;
    let a = stage(&t, &dot, "a.txt", "A").await;
    let run =
        t.rt.chat_with_files(&dot.id, "keep", None, &ids(&[&a]))
            .await
            .unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let folder = work(&t).join("attachments").join(&run.id);
    assert_eq!(std::fs::read_to_string(folder.join("a.txt")).unwrap(), "A");
    assert_eq!(
        std::fs::read_to_string(folder.join(".gitignore")).unwrap(),
        "*\n"
    );
}

#[tokio::test]
async fn a_follow_up_gets_missing_files_back_and_edits_are_kept() {
    let t = start(false).await;
    let dot = folder_dot(&t, "mender").await;
    let a = stage(&t, &dot, "a.txt", "A").await;
    let b = stage(&t, &dot, "b.txt", "B").await;
    let first =
        t.rt.chat_with_files(&dot.id, "one", None, &ids(&[&a, &b]))
            .await
            .unwrap();
    wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 30).await;
    let folder = work(&t).join("attachments").join(&first.id);
    std::fs::remove_file(folder.join("a.txt")).unwrap();
    std::fs::write(folder.join("b.txt"), "edited").unwrap();
    let reply = t.rt.chat(&dot.id, "two", Some(&first.id)).await.unwrap();
    wait_status(t.rt.store(), &reply.id, RunStatus::Succeeded, 30).await;
    assert_eq!(std::fs::read_to_string(folder.join("a.txt")).unwrap(), "A");
    assert_eq!(
        std::fs::read_to_string(folder.join("b.txt")).unwrap(),
        "edited"
    );
    // The reply carries no files of its own: its prompt has no block.
    let args: Value = serde_json::from_str(&std::fs::read_to_string(&t.args_out).unwrap()).unwrap();
    assert!(!args["prompt"]
        .as_str()
        .unwrap()
        .contains("## Attached files"));
}

#[tokio::test]
async fn a_reply_finds_its_files_in_a_restored_worktree() {
    let t = start(false).await;
    let repo = t.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);
    let mut s = spec("worker");
    s.workdir = repo.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let a = stage(&t, &dot, "a.txt", "A").await;
    let first =
        t.rt.chat_with_files(&dot.id, "one", None, &ids(&[&a]))
            .await
            .unwrap();
    let first = wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 30).await;
    let tree = PathBuf::from(first.workspace_path.clone().unwrap());
    // The files are ignored by git, so the unchanged worktree was cleaned up.
    assert!(!tree.exists(), "the unchanged worktree is still there");

    let reply = t.rt.chat(&dot.id, "two", Some(&first.id)).await.unwrap();
    let reply = wait_status(t.rt.store(), &reply.id, RunStatus::Succeeded, 30).await;
    assert_eq!(reply.workspace_path, first.workspace_path);
    assert!(
        seen(&t).contains(&format!("{}/a.txt", first.id)),
        "{:?}",
        seen(&t)
    );
    // Unchanged again despite the files: cleaned up once more.
    assert!(
        !tree.exists(),
        "the restored worktree counted the files as changes"
    );
    assert!(git_out(&repo, &["status", "--porcelain"]).is_empty());
}

/// A runtime on the fake CLI that plays the script at `script`, its queue held.
async fn start_scripted(script: &Path) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script.to_string_lossy().to_string(),
    ));
    let rt = Runtime::start(cfg).await.unwrap();
    rt.hold_runs(true).await;
    Rt { dir, rt, args_out }
}

/// A script whose engine writes `text` at `path` (relative to its workspace), then succeeds.
fn writes(path: &str, text: &str) -> String {
    json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"write": {"path": path, "text": text}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false,
                  "result": "done", "usage": {"input_tokens": 1, "output_tokens": 1}}},
    ])
    .to_string()
}

/// A worktree dot's run that edits an attachment, or writes a file beside it: the worktree is
/// kept, so the dot's work is not lost with it (attachments spec section 7).
#[tokio::test]
async fn a_worktree_keeps_what_the_dot_did_to_its_attachments() {
    for (written, text) in [("a.txt", "shortened"), ("a-short.txt", "new")] {
        let scripts = tempfile::tempdir().unwrap();
        let script = scripts.path().join("script.json");
        let t = start_scripted(&script).await;
        let repo = t.dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let mut s = spec("editor");
        s.workdir = repo.to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Worktree;
        let dot = t.rt.create_dot(s, None, None).await.unwrap();
        let a = stage(&t, &dot, "a.txt", "A").await;
        let run =
            t.rt.chat_with_files(&dot.id, "shorten", None, &ids(&[&a]))
                .await
                .unwrap();
        let path = format!("attachments/{}/{written}", run.id);
        std::fs::write(&script, writes(&path, text)).unwrap();
        t.rt.hold_runs(false).await;
        let ended = wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
        let tree = PathBuf::from(ended.workspace_path.clone().unwrap());
        assert!(
            tree.exists(),
            "{written}: the worktree went with the dot's work"
        );
        assert_eq!(
            std::fs::read_to_string(tree.join(&path)).unwrap(),
            text,
            "{written}"
        );
    }
}

#[cfg(windows)]
fn junction(link: &Path, target: &Path) -> bool {
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(unix)]
fn junction(link: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[tokio::test]
async fn copying_refuses_a_linked_attachments_folder() {
    let t = start(false).await;
    let dot = folder_dot(&t, "guarded").await;
    let outside = t.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    assert!(junction(&work(&t).join("attachments"), &outside));
    let a = stage(&t, &dot, "a.txt", "A").await;
    let run =
        t.rt.chat_with_files(&dot.id, "hi", None, &ids(&[&a]))
            .await
            .unwrap();
    let failed = wait_status(t.rt.store(), &run.id, RunStatus::Failed, 30).await;
    let error = failed.error.clone().unwrap_or_default();
    assert!(error.starts_with(COPY_FAILED), "{error}");
    assert_eq!(failed.delivery, Some(Delivery::NotDelivered));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    assert!(!t.args_out.exists(), "the engine started");
}

#[tokio::test]
async fn a_watcher_topic_still_loses_its_watcher_files() {
    let t = start(false).await;
    let dot = folder_dot(&t, "watched").await;
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.id = Some(dots_core::util::new_id());
    let root = new.id.clone().unwrap();
    new.payload = Some(json!({"message": "an event"}));
    let left = work(&t).join("attachments").join(&root);
    std::fs::create_dir_all(&left).unwrap();
    std::fs::write(left.join("mail.pdf"), "x").unwrap();
    let run = t.rt.store().create_run(&new).await.unwrap();
    t.rt.hold_runs(false).await;
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    assert!(
        !left.exists(),
        "a watcher topic's folder is removed after its run"
    );
}

#[tokio::test]
async fn retry_carries_the_files() {
    let t = start(true).await;
    let dot = folder_dot(&t, "retrier").await;
    let a = stage(&t, &dot, "a.txt", "A").await;
    let run =
        t.rt.chat_with_files(&dot.id, "", None, &ids(&[&a]))
            .await
            .unwrap();
    let store = t.rt.store();
    assert!(store.claim_run(&run.id).await.unwrap());
    store
        .finish_run(
            &run.id,
            RunStatus::Failed,
            None,
            Some("claude CLI not found"),
        )
        .await
        .unwrap();
    let again = t.rt.retry_delivery(&run.id).await.unwrap();
    assert_ne!(again.id, run.id);
    let carried = &again.payload.as_ref().unwrap()["attachments"];
    assert_eq!(carried, &json!([{"id": a.id, "name": "a.txt", "size": 1}]));
    let row = store.get_attachment(&a.id).await.unwrap();
    assert_eq!(row.run_id.as_deref(), Some(again.id.as_str()));
}

//! The Files panel's list (attachments spec section 4.5): the topics' attached files and the
//! files the dot wrote or edited, inside its workspace only, merged by path and newest first;
//! and the open target, which refuses a path not in the list and never opens a program.

mod common;

use std::path::{Path, PathBuf};

use common::spec;
use dots_core::chat_files::{NOT_THERE, PROGRAM};
use dots_core::model::{
    ConversationFile, Dot, FileSource, NewRun, Run, TriggerKind, WorkspaceMode,
};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    rt.hold_runs(true).await;
    Rt { dir, rt }
}

fn ws(t: &Rt) -> PathBuf {
    t.dir.path().join("work")
}

async fn dot(t: &Rt, name: &str) -> Dot {
    std::fs::create_dir_all(ws(t)).unwrap();
    let mut s = spec(name);
    s.workdir = ws(t).to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    t.rt.create_dot(s, None, None).await.unwrap()
}

fn at(n: u32) -> String {
    format!("2026-11-04T09:00:0{n}.000Z")
}

/// A chat run in the workspace, queued at `at(n)`, with `links` (staged id, name) as its files.
async fn chat_run(
    t: &Rt,
    dot: &Dot,
    parent: Option<&Run>,
    n: u32,
    links: &[(String, String)],
) -> Run {
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "m", "typed": true}));
    new.workspace_path = Some(ws(t).to_string_lossy().to_string());
    if let Some(p) = parent {
        new.parent_run_id = Some(p.id.clone());
        new.root_run_id = Some(p.root_run_id.clone());
    }
    let run =
        t.rt.store()
            .create_chat_run(&new, links, None, None)
            .await
            .unwrap();
    t.rt.store()
        .set_run_queued_at(&run.id, &at(n))
        .await
        .unwrap();
    t.rt.get_run(&run.id).await.unwrap()
}

/// A `tool_call` event of `run`, at `at(n)`.
async fn tool(t: &Rt, run: &Run, n: u32, tool: &str, input: serde_json::Value) {
    let ev =
        t.rt.store()
            .append_event(
                &run.id,
                "tool_call",
                &json!({"kind": "tool_call", "id": "t", "tool": tool, "input": input}),
            )
            .await
            .unwrap();
    t.rt.store()
        .set_event_ts(&run.id, ev.seq, &at(n))
        .await
        .unwrap();
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "x").unwrap();
}

async fn list(t: &Rt, dot: &Dot, roots: &[&Run]) -> Vec<ConversationFile> {
    let roots: Vec<String> = roots.iter().map(|r| r.root_run_id.clone()).collect();
    t.rt.conversation_files(&dot.id, &roots).await.unwrap()
}

fn paths(files: &[ConversationFile]) -> Vec<PathBuf> {
    files.iter().map(|f| PathBuf::from(&f.path)).collect()
}

#[tokio::test]
async fn written_files_come_from_every_tool() {
    let t = start().await;
    let d = dot(&t, "writer").await;
    let run = chat_run(&t, &d, None, 1, &[]).await;
    let w = ws(&t);
    for f in [
        "w.md", "out/a.md", "m.md", "n.ipynb", "l.txt", "e.txt", "r.md", "b.md",
    ] {
        touch(&w.join(f));
    }
    tool(&t, &run, 1, "Write", json!({"file_path": w.join("w.md")})).await;
    tool(&t, &run, 1, "Edit", json!({"file_path": "out/a.md"})).await;
    tool(
        &t,
        &run,
        1,
        "MultiEdit",
        json!({"file_path": w.join("m.md")}),
    )
    .await;
    tool(
        &t,
        &run,
        1,
        "NotebookEdit",
        json!({"notebook_path": w.join("n.ipynb")}),
    )
    .await;
    tool(
        &t,
        &run,
        1,
        "write_file",
        json!({"file_path": w.join("l.txt")}),
    )
    .await;
    tool(
        &t,
        &run,
        1,
        "edit_file",
        json!({"file_path": w.join("e.txt")}),
    )
    .await;
    tool(&t, &run, 1, "Read", json!({"file_path": w.join("r.md")})).await;
    tool(
        &t,
        &run,
        1,
        "Bash",
        json!({"command": "echo > b.md", "file_path": w.join("b.md")}),
    )
    .await;
    let files = list(&t, &d, &[&run]).await;
    let mut got = paths(&files);
    got.sort();
    let mut want: Vec<PathBuf> = ["e.txt", "l.txt", "m.md", "n.ipynb", "out/a.md", "w.md"]
        .iter()
        .map(|f| w.join(f))
        .collect();
    want.sort();
    assert_eq!(got, want);
    for f in &files {
        assert_eq!(f.source, FileSource::Written);
        assert!(f.exists && f.can_open, "{f:?}");
        assert_eq!(f.run_id, run.id);
    }
    let a = files.iter().find(|f| f.name == "a.md").unwrap();
    assert!(Path::new(&a.path).is_absolute());
}

#[tokio::test]
async fn paths_outside_the_workspace_are_left_out() {
    let t = start().await;
    let d = dot(&t, "fenced").await;
    let run = chat_run(&t, &d, None, 1, &[]).await;
    let outside = t.dir.path().join("other").join("x.txt");
    touch(&outside);
    touch(&t.dir.path().join("escape.txt"));
    tool(&t, &run, 1, "Write", json!({"file_path": outside})).await;
    tool(&t, &run, 1, "Write", json!({"file_path": "../escape.txt"})).await;
    tool(
        &t,
        &run,
        1,
        "Write",
        json!({"file_path": "sub/../../escape.txt"}),
    )
    .await;
    assert!(list(&t, &d, &[&run]).await.is_empty());
}

#[tokio::test]
async fn files_merge_and_come_newest_first() {
    let t = start().await;
    let d = dot(&t, "merger").await;
    let staged =
        t.rt.add_attachment_bytes(&d.id, "r.pdf", b"pdf")
            .await
            .unwrap();
    let first = chat_run(&t, &d, None, 2, &[(staged.id.clone(), "r.pdf".into())]).await;
    let w = ws(&t);
    let copy = w.join("attachments").join(&first.id).join("r.pdf");
    touch(&copy);
    touch(&w.join("A.md"));
    let upper = if cfg!(windows) { "A.md" } else { "a.md" };
    tool(&t, &first, 1, "Write", json!({"file_path": w.join(upper)})).await;
    tool(&t, &first, 3, "Edit", json!({"file_path": w.join("a.md")})).await;
    tool(&t, &first, 4, "Edit", json!({"file_path": copy})).await;
    let files = list(&t, &d, &[&first]).await;
    if cfg!(windows) {
        assert_eq!(files.len(), 2, "{files:#?}");
    }
    assert_eq!(files[0].name, "r.pdf");
    assert_eq!(files[0].source, FileSource::Attached);
    assert_eq!(files[0].at, at(4));
    assert_eq!(PathBuf::from(&files[0].path), copy);
    assert_eq!(files[1].source, FileSource::Written);
    assert_eq!(files[1].at, at(3));
}

#[tokio::test]
async fn a_deleted_file_is_gone() {
    let t = start().await;
    let d = dot(&t, "loser").await;
    let run = chat_run(&t, &d, None, 1, &[]).await;
    tool(
        &t,
        &run,
        1,
        "Write",
        json!({"file_path": ws(&t).join("gone.md")}),
    )
    .await;
    let files = list(&t, &d, &[&run]).await;
    assert_eq!(files.len(), 1);
    assert!(!files[0].exists);
    let e =
        t.rt.conversation_file_target(&d.id, &[run.id.clone()], &files[0].path, false)
            .await
            .unwrap_err();
    assert!(matches!(&e, Error::NotFound(m) if m == NOT_THERE), "{e:?}");
}

#[tokio::test]
async fn an_attachment_falls_back_to_its_stored_copy() {
    let t = start().await;
    let d = dot(&t, "keeper").await;
    let staged =
        t.rt.add_attachment_bytes(&d.id, "r.pdf", b"pdf")
            .await
            .unwrap();
    let run = chat_run(&t, &d, None, 1, &[(staged.id.clone(), "r.pdf".into())]).await;
    let files = list(&t, &d, &[&run]).await;
    assert_eq!(files.len(), 1);
    let stored = t
        .dir
        .path()
        .join("data")
        .join("chat-files")
        .join(&staged.id)
        .join("r.pdf");
    assert_eq!(PathBuf::from(&files[0].path), stored);
    assert!(files[0].exists);
    assert_eq!(files[0].source, FileSource::Attached);
    assert_eq!(files[0].at, at(1));
    assert_eq!(files[0].run_id, run.id);
}

#[tokio::test]
async fn a_path_not_in_the_list_is_not_opened() {
    let t = start().await;
    let d = dot(&t, "guard").await;
    let run = chat_run(&t, &d, None, 1, &[]).await;
    let roots = [run.id.clone()];
    let made_up = if cfg!(windows) {
        "C:\\Windows\\notepad.exe"
    } else {
        "/bin/sh"
    };
    let e =
        t.rt.conversation_file_target(&d.id, &roots, made_up, true)
            .await
            .unwrap_err();
    assert!(matches!(&e, Error::NotFound(m) if m == NOT_THERE), "{e:?}");
    let e =
        t.rt.conversation_file_target(&d.id, &roots, made_up, false)
            .await
            .unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

#[tokio::test]
async fn a_program_is_listed_but_not_opened() {
    let t = start().await;
    let d = dot(&t, "scripter").await;
    let run = chat_run(&t, &d, None, 1, &[]).await;
    let script = ws(&t).join("run.ps1");
    touch(&script);
    tool(&t, &run, 1, "Write", json!({"file_path": script})).await;
    let files = list(&t, &d, &[&run]).await;
    assert_eq!(files.len(), 1);
    assert!(!files[0].can_open);
    let roots = [run.id.clone()];
    let path = files[0].path.clone();
    let e =
        t.rt.conversation_file_target(&d.id, &roots, &path, true)
            .await
            .unwrap_err();
    assert!(matches!(&e, Error::Invalid(m) if m == PROGRAM), "{e:?}");
    let target =
        t.rt.conversation_file_target(&d.id, &roots, &path, false)
            .await
            .unwrap();
    assert_eq!(target, script);
}

#[tokio::test]
async fn topics_of_another_dot_are_ignored() {
    let t = start().await;
    let mine = dot(&t, "mine").await;
    let theirs = dot(&t, "theirs").await;
    let run = chat_run(&t, &theirs, None, 1, &[]).await;
    let file = ws(&t).join("t.md");
    touch(&file);
    tool(&t, &run, 1, "Write", json!({"file_path": file})).await;
    assert!(list(&t, &mine, &[&run]).await.is_empty());
    assert_eq!(list(&t, &theirs, &[&run]).await.len(), 1);
    let e =
        t.rt.conversation_files("NODOT", &[run.id.clone()])
            .await
            .unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

//! Open Dot against a running core and the fake CLI: the picture of now (what it lists for the
//! account and workspace in view, and what it only counts).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::{spec, wait_status};
use dots_core::inbox::{InboxKey, InboxKind};
use dots_core::model::{Approval, Dot, DotSpec, Message, RunStatus, WorkspaceMode};
use dots_core::open_dot::picture::{Picture, View};
use dots_core::open_dot::prompt::{prompt, Turn};
use dots_core::{Config, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

/// A running core whose fake CLI follows the script last written with `script`.
struct Rt {
    dir: TempDir,
    rt: Arc<Runtime>,
    script_path: PathBuf,
}

impl Rt {
    async fn start() -> Rt {
        Self::start_with(|_| {}).await
    }

    async fn start_with(change: impl FnOnce(&mut Config)) -> Rt {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("script.json");
        std::fs::write(&script_path, succeeding().to_string()).unwrap();
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        cfg.claude_env.push((
            "FAKE_CLAUDE_ARGS_OUT".into(),
            dir.path().join("args.json").to_string_lossy().to_string(),
        ));
        cfg.claude_env.push((
            "FAKE_CLAUDE_SCRIPT".into(),
            script_path.to_string_lossy().to_string(),
        ));
        cfg.new_dot_root = dir.path().join("Dots");
        change(&mut cfg);
        let rt = Arc::new(Runtime::start(cfg).await.unwrap());
        Rt {
            dir,
            rt,
            script_path,
        }
    }

    /// The script the fake CLI follows from its next start.
    fn script(&self, script: Value) {
        std::fs::write(&self.script_path, script.to_string()).unwrap();
    }

    /// A folder dot of this test's own folder that asks for every approval at once.
    fn dot_spec(&self, name: &str) -> DotSpec {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.approval_wait_secs = 0;
        s
    }

    /// A dot in `org` (`None` for Personal) and `workspace`, which must exist already
    /// (`add_workspace` adds a Personal one).
    async fn make_dot(&self, name: &str, org: Option<&str>, workspace: &str) -> Dot {
        self.rt
            .create_dot(
                self.dot_spec(name),
                org.map(str::to_string),
                Some(workspace.to_string()),
            )
            .await
            .unwrap()
    }

    /// Adds a Personal workspace; returns its id.
    async fn add_workspace(&self, name: &str) -> String {
        self.rt.personal_workspace_add(name).await.unwrap().id
    }

    /// Runs `dot` once with a CLI that asks to use `tool` with `input`; returns the approval
    /// that waits.
    async fn pending_approval(&self, dot: &Dot, tool: &str, input: Value) -> Approval {
        self.script(json!([
            {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
            {"approve": {"tool_name": tool, "input": input}},
            {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                      "usage": {"input_tokens": 1, "output_tokens": 1}}}
        ]));
        let run = self.rt.run_now(&dot.id).await.unwrap();
        wait_status(self.rt.store(), &run.id, RunStatus::AwaitingApproval, 15).await;
        self.rt
            .store()
            .list_pending_approvals()
            .await
            .unwrap()
            .into_iter()
            .find(|a| a.run_id == run.id)
            .expect("the run's approval")
    }

    /// Runs `dot` once to its end and posts an open question of that run.
    async fn post_question(&self, dot: &Dot, text: &str, choices: &[&str]) -> Message {
        self.script(succeeding());
        let run = self.rt.run_now(&dot.id).await.unwrap();
        wait_status(self.rt.store(), &run.id, RunStatus::Succeeded, 15).await;
        let choices: Vec<String> = choices.iter().map(|c| c.to_string()).collect();
        let choices = (!choices.is_empty()).then_some(choices);
        self.rt
            .store()
            .add_question(&dot.id, &run.id, text, choices.as_deref())
            .await
            .unwrap()
            .expect("the run's first question")
    }

    async fn picture(&self, workspace: &str) -> Picture {
        Picture::take(&self.rt, &view("personal", workspace))
            .await
            .unwrap()
    }
}

/// A script that ends at once with success.
fn succeeding() -> Value {
    json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ])
}

fn view(account: &str, workspace: &str) -> View {
    View {
        account: account.into(),
        workspace: workspace.into(),
        screen: "home".into(),
        dot_id: None,
        topic_root: None,
    }
}

#[tokio::test]
async fn picture_lists_only_this_workspace() {
    let t = Rt::start().await;
    let payments = t.add_workspace("Payments").await;
    let here = t.make_dot("Deploy watcher", None, "default").await;
    let there = t.make_dot("Invoice sender", None, &payments).await;

    let p = t.picture("default").await;
    let ids: Vec<&str> = p.dots.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec![here.id.as_str()]);
    assert_eq!(p.dots[0].handle, "d1");
    assert_eq!(p.dots[0].name, "Deploy watcher");
    assert_eq!(p.dots[0].status, "idle");
    assert!(p.text().contains("Deploy watcher"));
    assert!(!p.text().contains("Invoice sender"));

    let p = t.picture(&payments).await;
    let ids: Vec<&str> = p.dots.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec![there.id.as_str()]);
    assert!(!p.text().contains("Deploy watcher"));

    // The dot on screen is named by its handle, and only when it is in the picture.
    let mut v = view("personal", "default");
    v.dot_id = Some(here.id.clone());
    let p = Picture::take(&t.rt, &v).await.unwrap();
    assert_eq!(p.viewing_dot.as_deref(), Some("d1"));
    v.dot_id = Some(there.id.clone());
    let p = Picture::take(&t.rt, &v).await.unwrap();
    assert_eq!(p.viewing_dot, None);
    t.rt.shutdown();
}

#[tokio::test]
async fn private_approval_is_counted_not_described() {
    let t = Rt::start().await;
    let dot = t.make_dot("Config reader", None, "default").await;
    let approval = t
        .pending_approval(&dot, "Bash", json!({"command": "cat .env.zebra"}))
        .await;
    assert!(t.rt.approval_touches_private(&approval.id).await.unwrap());

    let p = t.picture("default").await;
    assert_eq!(p.private_approvals, 1);
    assert!(p.approvals.is_empty());
    assert_eq!(p.approval("a1"), None);
    let text = p.text();
    let said = prompt(
        &p,
        &[Turn {
            heard: "what needs me".into(),
            said: "One thing.".into(),
        }],
        "approve it",
    );
    for shown in [&text, &said] {
        for secret in ["zebra", ".env", "cat ", &approval.id, "Bash"] {
            assert!(!shown.contains(secret), "{secret:?} in {shown}");
        }
    }
    // It is counted where the person can be told to look.
    assert!(text.contains("1 private"), "{text}");
    // The dot is still listed, waiting.
    assert_eq!(p.dots[0].status, "waiting for approval");
    assert_eq!(p.dots[0].needs_you, 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn an_approval_that_is_not_private_is_described() {
    let t = Rt::start().await;
    let dot = t.make_dot("Cleaner", None, "default").await;
    let approval = t
        .pending_approval(&dot, "Bash", json!({"command": "rm a.txt b.txt"}))
        .await;
    let p = t.picture("default").await;
    assert_eq!(p.private_approvals, 0);
    let a = p.approval("a1").expect("a1");
    assert_eq!(a.id, approval.id);
    assert_eq!(a.dot_id, dot.id);
    assert_eq!(a.dot_name, "Cleaner");
    assert_eq!(a.tool, "Bash");
    assert_eq!(a.target, "delete 2 files: a.txt, b.txt");
    assert!(p.text().contains("delete 2 files: a.txt, b.txt"));
    t.rt.shutdown();
}

#[tokio::test]
async fn questions_are_listed_with_choices() {
    let t = Rt::start().await;
    let payments = t.add_workspace("Payments").await;
    let dot = t.make_dot("Release helper", None, "default").await;
    let other = t.make_dot("Elsewhere", None, &payments).await;
    let open = t
        .post_question(&dot, "Ship it to\nstaging?", &["Yes", "No"])
        .await;
    t.post_question(&other, "Not here?", &[]).await;

    let p = t.picture("default").await;
    assert_eq!(p.questions.len(), 1);
    let q = p.question("q1").expect("q1");
    assert_eq!(q.id, open.id);
    assert_eq!(q.dot_id, dot.id);
    assert_eq!(q.dot_name, "Release helper");
    assert_eq!(q.text, "Ship it to staging?");
    assert_eq!(q.choices, vec!["Yes".to_string(), "No".to_string()]);
    let text = p.text();
    assert!(text.contains("Ship it to staging?"), "{text}");
    assert!(text.contains("Yes") && text.contains("No"), "{text}");
    assert!(!text.contains("Not here?"));

    // An answered question is no longer listed.
    t.rt.store().dismiss_question(&open.id).await.unwrap();
    let p = t.picture("default").await;
    assert!(p.questions.is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn snoozed_and_done_questions_are_neither_listed_nor_counted() {
    let t = Rt::start().await;
    let dot = t.make_dot("Release helper", None, "default").await;
    let snoozed = t.post_question(&dot, "Snoozed one?", &[]).await;
    let done = t.post_question(&dot, "Done one?", &[]).await;
    let open = t.post_question(&dot, "Still open?", &[]).await;
    let key = |id: &str| InboxKey::new(InboxKind::Message, id);
    t.rt.inbox_snooze(
        &[key(&snoozed.id)],
        chrono::Utc::now() + chrono::Duration::hours(2),
    )
    .await
    .unwrap();
    t.rt.inbox_done(&key(&done.id)).await.unwrap();

    let p = t.picture("default").await;
    let ids: Vec<&str> = p.questions.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids, vec![open.id.as_str()]);
    let text = p.text();
    assert!(text.contains("Questions waiting: 1"), "{text}");
    assert!(
        !text.contains("Snoozed one") && !text.contains("Done one"),
        "{text}"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn an_open_conversation_shows_on_screen() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    let mut v = view("personal", "default");
    v.dot_id = Some(dot.id.clone());
    let p = Picture::take(&t.rt, &v).await.unwrap();
    assert!(!p.viewing_topic);
    assert!(p.text().contains("On screen: d1 \"Research\"\n"));
    v.topic_root = Some("some-root".into());
    let p = Picture::take(&t.rt, &v).await.unwrap();
    assert!(p.viewing_topic);
    assert!(p
        .text()
        .contains("On screen: d1 \"Research\", a conversation is open"));
    // a conversation of a dot not in the picture is not said
    let mut v = view("personal", "default");
    v.dot_id = Some("not-a-dot".into());
    v.topic_root = Some("some-root".into());
    let p = Picture::take(&t.rt, &v).await.unwrap();
    assert!(!p.viewing_topic);
    t.rt.shutdown();
}

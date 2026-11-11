//! Open Dot against a running core and the fake CLI: the picture of now (what it lists for the
//! account and workspace in view, and what it only counts); its streamed thinking through the
//! CLI, and what its requests used, against the spending caps.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{spec, temp_store, wait_status};
use dots_core::engine::claude::once::{OnceFailure, OnceUsage};
use dots_core::inbox::{InboxKey, InboxKind};
use dots_core::model::{
    Approval, ApprovalStatus, Dot, DotSpec, Message, NewRun, RunStatus, TriggerKind, UsagePeriod,
    WorkspaceMode,
};
use dots_core::open_dot::act::OpenTarget;
use dots_core::open_dot::act::{carry_out, plan, Deed, Step};
use dots_core::open_dot::picture::{Picture, View};
use dots_core::open_dot::prompt::{prompt, Turn};
use dots_core::open_dot::reply::Wish;
use dots_core::open_dot::think::{cap_hit, CliThinker, Thinker, THINK_LIMIT};
use dots_core::open_dot::{Card, MindEvent, Then};
use dots_core::settings::Settings;
use dots_core::spend::{day_bounds_in, Spend};
use dots_core::store::OpenDotUsage;
use dots_core::{Config, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

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

// ---- Acting: checked deeds carried out through the runtime ----

/// The deed `wish` plans to on the picture of `v`, confirmed or done.
async fn deed_for(t: &Rt, wish: Wish, v: &View) -> Deed {
    let p = Picture::take(&t.rt, v).await.unwrap();
    match plan(&wish, &p, v) {
        Step::Confirm { deed, .. } | Step::Do(deed) => deed,
        other => panic!("{wish:?} planned {other:?}"),
    }
}

/// The dot's runs that `pick` chooses, once one is there.
async fn wait_run(t: &Rt, dot_id: &str, pick: impl Fn(&dots_core::model::Run) -> bool) -> String {
    for _ in 0..150 {
        let runs = t.rt.store().list_runs(Some(dot_id), 50).await.unwrap();
        if let Some(r) = runs.iter().find(|r| pick(r)) {
            return r.id.clone();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no such run");
}

fn message_of(r: &dots_core::model::Run) -> Option<&str> {
    r.payload.as_ref()?.get("message")?.as_str()
}

#[tokio::test]
async fn approve_resumes_the_run() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    let approval = t
        .pending_approval(&dot, "Bash", json!({"command": "git push origin main"}))
        .await;
    t.script(succeeding());
    let v = view("personal", "default");
    let p = t.picture("default").await;
    let Step::Confirm { deed, question, .. } = plan(&Wish::Approve { a: "a1".into() }, &p, &v)
    else {
        panic!("approving confirms");
    };
    // the question is the record's: the guard's summary of the call, not the model's words
    assert_eq!(
        question,
        "Research wants to use Bash on push commits with git (git push origin main). Approve it?"
    );
    assert_eq!(carry_out(&t.rt, &deed).await, "Approved.");
    let decided = t.rt.store().get_approval(&approval.id).await.unwrap();
    assert_eq!(decided.status, ApprovalStatus::Approved);
    let resumed = wait_run(&t, &dot.id, |r| r.trigger == TriggerKind::Resume).await;
    wait_status(t.rt.store(), &resumed, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn deny_is_recorded() {
    let t = Rt::start().await;
    let dot = t.make_dot("Backup", None, "default").await;
    let approval = t
        .pending_approval(&dot, "Bash", json!({"command": "npm test"}))
        .await;
    t.script(succeeding());
    let deed = deed_for(
        &t,
        Wish::Deny { a: "a1".into() },
        &view("personal", "default"),
    )
    .await;
    assert!(matches!(deed, Deed::Deny { .. }));
    assert_eq!(carry_out(&t.rt, &deed).await, "Denied.");
    let decided = t.rt.store().get_approval(&approval.id).await.unwrap();
    assert_eq!(decided.status, ApprovalStatus::Denied);
    t.rt.shutdown();
}

#[tokio::test]
async fn already_decided_approval_says_handled() {
    let t = Rt::start().await;
    let dot = t.make_dot("Backup", None, "default").await;
    t.pending_approval(&dot, "Bash", json!({"command": "npm test"}))
        .await;
    t.script(succeeding());
    let v = view("personal", "default");
    let approve = deed_for(&t, Wish::Approve { a: "a1".into() }, &v).await;
    let deny = deed_for(&t, Wish::Deny { a: "a1".into() }, &v).await;
    // decided elsewhere (the Inbox) between the picture and the yes
    let Deed::Approve { approval_id, .. } = &approve else {
        panic!()
    };
    t.rt.decide_approval(approval_id, true, None, None)
        .await
        .unwrap();
    assert_eq!(
        carry_out(&t.rt, &approve).await,
        "That one is already handled."
    );
    assert_eq!(
        carry_out(&t.rt, &deny).await,
        "That one is already handled."
    );
    // gone altogether
    let gone = Deed::Approve {
        approval_id: "01KNOTANAPPROVAL0000000000".into(),
        dot_name: "Backup".into(),
        tool: "Bash".into(),
        target: "npm test".into(),
    };
    assert_eq!(
        carry_out(&t.rt, &gone).await,
        "That one is already handled."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn run_now_starts_a_run() {
    let t = Rt::start().await;
    let dot = t.make_dot("Backup", None, "default").await;
    let v = view("personal", "default");
    let p = t.picture("default").await;
    let Step::Confirm { deed, question, .. } = plan(&Wish::Run { d: "d1".into() }, &p, &v) else {
        panic!("running confirms");
    };
    assert_eq!(question, "Run Backup now?");
    assert_eq!(carry_out(&t.rt, &deed).await, "Backup is running.");
    let run = wait_run(&t, &dot.id, |r| r.trigger == TriggerKind::Manual).await;
    wait_status(t.rt.store(), &run, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn send_makes_a_new_topic() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    let wish = Wish::Send {
        d: "d1".into(),
        text: "look into the build".into(),
        here: true,
    };
    // "here" on the home screen: no topic is open, so a new one
    let deed = deed_for(&t, wish, &view("personal", "default")).await;
    assert!(
        matches!(
            &deed,
            Deed::Send {
                topic_root: None,
                ..
            }
        ),
        "{deed:?}"
    );
    assert_eq!(carry_out(&t.rt, &deed).await, "Sent to Research.");
    let id = wait_run(&t, &dot.id, |r| {
        message_of(r) == Some("look into the build")
    })
    .await;
    let run = t.rt.store().get_run(&id).await.unwrap();
    assert_eq!(run.trigger, TriggerKind::Chat);
    assert_eq!(run.parent_run_id, None);
    assert_eq!(run.root_run_id, run.id);
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn send_here_continues_the_topic() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    let first = t.rt.chat(&dot.id, "first", None).await.unwrap();
    wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 15).await;
    let mut v = view("personal", "default");
    v.dot_id = Some(dot.id.clone());
    v.topic_root = Some(first.id.clone());
    let wish = Wish::Send {
        d: "d1".into(),
        text: "and the tests".into(),
        here: true,
    };
    let deed = deed_for(&t, wish, &v).await;
    assert_eq!(carry_out(&t.rt, &deed).await, "Sent to Research.");
    let id = wait_run(&t, &dot.id, |r| message_of(r) == Some("and the tests")).await;
    let run = t.rt.store().get_run(&id).await.unwrap();
    assert_eq!(run.parent_run_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(run.root_run_id, first.id);
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn send_into_a_busy_topic_says_so() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    t.script(json!([init(), {"sleep_ms": 20000}]));
    let first = t.rt.chat(&dot.id, "first", None).await.unwrap();
    wait_status(t.rt.store(), &first.id, RunStatus::Running, 15).await;
    let deed = Deed::Send {
        dot_id: dot.id.clone(),
        dot_name: "Research".into(),
        text: "and more".into(),
        topic_root: Some(first.id.clone()),
    };
    assert_eq!(
        carry_out(&t.rt, &deed).await,
        "Research is still working on that. Try again when it finishes."
    );
    let runs = t.rt.store().list_runs(Some(&dot.id), 10).await.unwrap();
    assert_eq!(runs.len(), 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn send_into_a_topic_waiting_for_approval_says_the_runtimes_words() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    t.script(json!([
        init(),
        {"approve": {"tool_name": "Bash", "input": {"command": "npm test"}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]));
    let first = t.rt.chat(&dot.id, "first", None).await.unwrap();
    wait_status(t.rt.store(), &first.id, RunStatus::AwaitingApproval, 15).await;
    let deed = Deed::Send {
        dot_id: dot.id.clone(),
        dot_name: "Research".into(),
        text: "and more".into(),
        topic_root: Some(first.id.clone()),
    };
    assert_eq!(
        carry_out(&t.rt, &deed).await,
        "This chat is waiting for an approval; answer it, then send your message again."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn send_into_another_dots_topic_is_refused_in_plain_words() {
    let t = Rt::start().await;
    let research = t.make_dot("Research", None, "default").await;
    let backup = t.make_dot("Backup", None, "default").await;
    let theirs = t.rt.chat(&backup.id, "first", None).await.unwrap();
    wait_status(t.rt.store(), &theirs.id, RunStatus::Succeeded, 15).await;
    let deed = Deed::Send {
        dot_id: research.id.clone(),
        dot_name: "Research".into(),
        text: "hello".into(),
        topic_root: Some(theirs.id.clone()),
    };
    assert_eq!(
        carry_out(&t.rt, &deed).await,
        "That didn't work: run belongs to a different dot."
    );
    assert!(t
        .rt
        .store()
        .list_runs(Some(&research.id), 10)
        .await
        .unwrap()
        .is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn send_or_run_to_a_dot_that_is_gone() {
    let t = Rt::start().await;
    let gone = "01KNOTADOT00000000000000000";
    let send = Deed::Send {
        dot_id: gone.into(),
        dot_name: "Research".into(),
        text: "hello".into(),
        topic_root: None,
    };
    assert_eq!(
        carry_out(&t.rt, &send).await,
        "I can't find that dot anymore."
    );
    let run = Deed::Run {
        dot_id: gone.into(),
        dot_name: "Research".into(),
    };
    assert_eq!(
        carry_out(&t.rt, &run).await,
        "I can't find that dot anymore."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn answer_replies_to_the_question() {
    let t = Rt::start().await;
    let dot = t.make_dot("Release helper", None, "default").await;
    let question = t
        .post_question(&dot, "Ship it to staging?", &["Yes", "No"])
        .await;
    let wish = Wish::Answer {
        q: "q1".into(),
        text: "Yes".into(),
        unsure: false,
    };
    let deed = deed_for(&t, wish, &view("personal", "default")).await;
    assert_eq!(carry_out(&t.rt, &deed).await, "Answered Release helper.");
    let answered = t.rt.store().get_message(&question.id).await.unwrap();
    assert!(answered.answered_at.is_some());
    let run = answered.answer_run_id.expect("the answer's run");
    wait_status(t.rt.store(), &run, RunStatus::Succeeded, 15).await;
    // the same answer again: already handled
    assert_eq!(
        carry_out(&t.rt, &deed).await,
        "That one is already handled."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn viewer_cannot_approve_and_hears_why() {
    use common::clock::FakeClock;
    use common::fake_openbao::FakeOpenBao;
    use common::orgs::{
        at, code, keys, openbao, policy, public_key, roster, sign, sign_roster, token, ACME,
        ACME_NAME, ISSUED, START, TOKEN,
    };
    use dots_core::accounts::identity;
    use dots_core::vault::MemorySecrets;

    let bao = FakeOpenBao::start().await;
    bao.allow_token(TOKEN, false);
    let owner_keys = keys();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.clock = FakeClock::new(at(START));
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    let me = identity(rt.store()).await.unwrap().public_key;
    let serve = |seq: u64, role: &str| {
        let member = |id: &str, email: &str, role: &str, key: &str| {
            json!({"id": id, "name": email, "email": email, "role": role, "keys": [key],
                   "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"})
        };
        let members = json!([
            member("01OWNER", "owner@acme.com", "owner", &public_key(&keys())),
            member("01ME", "me@acme.com", role, &me),
        ]);
        let text = roster(ACME, seq, members);
        let extra = json!({"v": 2, "roster": "required", "admins": [],
                           "plain": [], "offline_hours": 0});
        bao.put_env("secret", "dots", json!({}));
        bao.put_policy(
            "secret",
            "dots",
            &sign(&owner_keys, &policy(ACME, ISSUED, extra)),
        );
        bao.put_roster("secret", "dots", &sign_roster(&owner_keys, &text));
    };
    serve(1, "member");
    let orgs = rt.vault().orgs().expect("the runtime's orgs");
    orgs.join(
        &code(
            ACME,
            ACME_NAME,
            openbao(&bao.address(), "dots"),
            &owner_keys,
        ),
        Some(token()),
    )
    .await
    .unwrap();
    let mut s = spec("Triage");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = rt.create_dot(s, Some(ACME.into()), None).await.unwrap();
    // Held before the run is queued, so nothing but the deeds below changes it.
    rt.hold_runs(true).await;
    let run = rt
        .store()
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let approval = rt
        .store()
        .create_approval(&run.id, "Bash", &json!({"command": "ls"}))
        .await
        .unwrap();
    serve(2, "viewer");
    rt.refresh_org(ACME).await.unwrap();

    let approve = Deed::Approve {
        approval_id: approval.id.clone(),
        dot_name: "Triage".into(),
        tool: "Bash".into(),
        target: "ls".into(),
    };
    assert_eq!(
        carry_out(&rt, &approve).await,
        "Acme Engineering: a Viewer can't answer approvals. Ask an Admin or Owner."
    );
    let still = rt.store().get_approval(&approval.id).await.unwrap();
    assert_eq!(still.status, ApprovalStatus::Pending);
    let run_it = Deed::Run {
        dot_id: dot.id.clone(),
        dot_name: "Triage".into(),
    };
    assert_eq!(
        carry_out(&rt, &run_it).await,
        "Acme Engineering: a Viewer can't run dots. Ask an Admin or Owner."
    );
    assert_eq!(
        rt.store().list_runs(Some(&dot.id), 10).await.unwrap().len(),
        1
    );
    rt.shutdown();
}

// ---- Thinking: one streamed request to the CLI ----

/// A `stream_event` line carrying `text` as a text delta.
fn delta(text: &str) -> Value {
    json!({"emit": {"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
        "delta": {"type": "text_delta", "text": text}}, "session_id": "$SESSION"}})
}

/// The final assistant message and the result, with usage.
fn finished(text: &str) -> Vec<Value> {
    vec![
        json!({"emit": {"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}}),
        json!({"emit": {"type": "result", "subtype": "success", "is_error": false, "result": text,
            "total_cost_usd": 0.0025,
            "usage": {"input_tokens": 120, "output_tokens": 30, "cache_read_input_tokens": 7}}}),
    ]
}

fn init() -> Value {
    json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}})
}

/// A thinker over the fake CLI following `script`, in folders under `dir`.
fn thinker(dir: &Path, script: Value, limit: Duration) -> CliThinker {
    let script_path = dir.join("think-script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let env = vec![
        (
            "FAKE_CLAUDE_SCRIPT".to_string(),
            script_path.to_string_lossy().to_string(),
        ),
        (
            "FAKE_CLAUDE_ARGS_OUT".to_string(),
            dir.join("think-args.json").to_string_lossy().to_string(),
        ),
    ];
    CliThinker::new(
        Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude"))),
        env,
        dir.join("think"),
        limit,
    )
}

/// The request folders left under `dir/think`.
fn folders_left(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("think"))
        .map(|d| d.count())
        .unwrap_or(0)
}

#[tokio::test]
async fn streamed_text_arrives_in_order_and_equals_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let mut script = vec![init(), delta("Hel"), delta("lo "), delta("there.")];
    // a thinking delta and a message start are not text
    script.push(
        json!({"emit": {"type": "stream_event", "event": {"type": "content_block_delta",
        "index": 0, "delta": {"type": "thinking_delta", "thinking": "hmm"}}}}),
    );
    script.push(json!({"emit": {"type": "stream_event", "event": {"type": "message_start"}}}));
    script.extend(finished("Hello there."));
    let t = thinker(dir.path(), Value::Array(script), THINK_LIMIT);
    let mut got: Vec<String> = Vec::new();
    let answer = t
        .think(
            "system",
            "prompt",
            "haiku",
            &CancellationToken::new(),
            &mut |s: &str| got.push(s.to_string()),
        )
        .await
        .unwrap();
    assert_eq!(got, vec!["Hel", "lo ", "there."]);
    assert_eq!(got.concat(), answer.text);
    assert_eq!(answer.text, "Hello there.");
    assert_eq!(
        answer.usage,
        Some(OnceUsage {
            tokens_in: 127,
            tokens_out: 30,
            cache_read: 7,
            cost_usd: Some(0.0025),
        })
    );
    assert_eq!(folders_left(dir.path()), 0);
}

#[tokio::test]
async fn no_partials_still_delivers_the_text_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut script = vec![init()];
    script.extend(finished("Nothing needs you."));
    let t = thinker(dir.path(), Value::Array(script), THINK_LIMIT);
    let mut got: Vec<String> = Vec::new();
    let answer = t
        .think(
            "system",
            "prompt",
            "haiku",
            &CancellationToken::new(),
            &mut |s: &str| got.push(s.to_string()),
        )
        .await
        .unwrap();
    assert_eq!(got, vec!["Nothing needs you."]);
    assert_eq!(answer.text, "Nothing needs you.");
}

#[tokio::test]
async fn flags_include_partial_messages_and_no_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut script = vec![init()];
    script.extend(finished("Ok."));
    let t = thinker(dir.path(), Value::Array(script), THINK_LIMIT);
    t.think(
        "the system prompt",
        "prompt",
        "claude-haiku-4-5",
        &CancellationToken::new(),
        &mut |_: &str| {},
    )
    .await
    .unwrap();
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("think-args.json")).unwrap())
            .unwrap();
    let args: Vec<String> = record["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let after = |flag: &str| {
        let i = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag}: {args:?}"));
        args.get(i + 1).cloned().unwrap_or_default()
    };
    assert!(
        args.contains(&"--include-partial-messages".to_string()),
        "{args:?}"
    );
    assert_eq!(after("--tools"), "");
    assert_eq!(after("--max-turns"), "1");
    assert_eq!(after("--model"), "claude-haiku-4-5");
    assert_eq!(after("--system-prompt"), "the system prompt");
    assert_eq!(after("--output-format"), "stream-json");
    for flag in [
        "--strict-mcp-config",
        "--no-session-persistence",
        "-p",
        "--verbose",
    ] {
        assert!(args.contains(&flag.to_string()), "{flag}: {args:?}");
    }
    assert!(!args.iter().any(|a| a == "--mcp-config"));
}

#[tokio::test]
async fn cancel_stops_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut script = vec![init(), delta("First words."), json!({"sleep_ms": 20000})];
    script.extend(finished("First words. More."));
    let t = thinker(dir.path(), Value::Array(script), THINK_LIMIT);
    let cancel = CancellationToken::new();
    let mut got: Vec<String> = Vec::new();
    let started = Instant::now();
    // the first words arrive while the CLI still runs: cancel on them
    let asked = t
        .think("system", "prompt", "haiku", &cancel, &mut |s: &str| {
            got.push(s.to_string());
            cancel.cancel();
        })
        .await;
    assert_eq!(asked, Err(OnceFailure::Cancelled));
    assert_eq!(got, vec!["First words."]);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(folders_left(dir.path()), 0);
}

#[tokio::test]
async fn times_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut script = vec![init(), json!({"sleep_ms": 20000})];
    script.extend(finished("Too late."));
    let t = thinker(dir.path(), Value::Array(script), Duration::from_millis(500));
    let started = Instant::now();
    let asked = t
        .think(
            "system",
            "prompt",
            "haiku",
            &CancellationToken::new(),
            &mut |_: &str| {},
        )
        .await;
    assert_eq!(asked, Err(OnceFailure::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(15));
    assert_eq!(folders_left(dir.path()), 0);
}

#[tokio::test]
async fn a_dropped_request_leaves_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut script = vec![init(), delta("First words."), json!({"sleep_ms": 20000})];
    script.extend(finished("First words. More."));
    let t = thinker(dir.path(), Value::Array(script), THINK_LIMIT);
    let cancel = CancellationToken::new();
    let mut got: Vec<String> = Vec::new();
    let (first, mut heard) = tokio::sync::mpsc::unbounded_channel::<()>();
    {
        let mut on_text = |s: &str| {
            got.push(s.to_string());
            let _ = first.send(());
        };
        let request = t.think("system", "prompt", "haiku", &cancel, &mut on_text);
        // run the request until its first words arrive, then drop it mid-request
        tokio::select! {
            _ = request => panic!("the request should still be running"),
            _ = heard.recv() => {}
        }
    }
    assert_eq!(got, vec!["First words."]);
    // the folder goes with the request, also when the CLI takes a moment to end
    let started = Instant::now();
    while folders_left(dir.path()) > 0 {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the request folder was left behind"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn partials_then_a_failed_result_is_failed() {
    let dir = tempfile::tempdir().unwrap();
    let script = json!([
        init(),
        delta("Half "),
        delta("done"),
        {"emit": {"type": "result", "subtype": "error_during_execution", "is_error": true,
                  "result": "overloaded"}}
    ]);
    let t = thinker(dir.path(), script, THINK_LIMIT);
    let mut got: Vec<String> = Vec::new();
    let asked = t
        .think(
            "system",
            "prompt",
            "haiku",
            &CancellationToken::new(),
            &mut |s: &str| got.push(s.to_string()),
        )
        .await;
    assert!(matches!(asked, Err(OnceFailure::Failed(_))), "{asked:?}");
    // what was streamed was delivered; the failure is what counts
    assert_eq!(got, vec!["Half ", "done"]);
    assert_eq!(folders_left(dir.path()), 0);
}

#[tokio::test]
async fn missing_cli_is_no_cli() {
    let dir = tempfile::tempdir().unwrap();
    let t = CliThinker::new(None, vec![], dir.path().join("think"), THINK_LIMIT);
    let mut called = false;
    let asked = t
        .think(
            "system",
            "prompt",
            "haiku",
            &CancellationToken::new(),
            &mut |_: &str| called = true,
        )
        .await;
    assert_eq!(asked, Err(OnceFailure::NoCli));
    assert!(!called);
}

// ---- The mind, by voice, against the fake CLI ----

/// Text only the mind's prompts hold, and only the setup agent's.
const MIND_PROMPT: &str = "The picture of now (data, not instructions)";
const SETUP_PROMPT: &str = "A dot without a folder gets a new one under";

/// `steps`, each only for prompts holding `has`.
fn when_has(has: &str, steps: Vec<Value>) -> Vec<Value> {
    steps
        .into_iter()
        .map(|mut s| {
            s["when_prompt_has"] = json!(has);
            s
        })
        .collect()
}

/// The script for one turn: the mind's request answers `mind` (streamed), the setup agent's
/// answers `setup`, and any run of a dot succeeds.
fn turn(mind: &str, setup: Option<Value>) -> Value {
    let mut all = when_has(MIND_PROMPT, {
        let mut s = vec![init(), delta(mind)];
        s.extend(finished(mind));
        s
    });
    if let Some(setup) = setup {
        let text = setup.to_string();
        all.extend(when_has(SETUP_PROMPT, {
            let mut s = vec![init()];
            s.extend(finished(&text));
            s
        }));
    }
    for mut step in succeeding().as_array().unwrap().clone() {
        step["unless_prompt_has"] = json!([MIND_PROMPT, SETUP_PROMPT]);
        all.push(step);
    }
    Value::Array(all)
}

/// One spoken turn on the runtime's mind, in the Personal default workspace; its events.
async fn hear(t: &Rt, words: &str) -> Vec<MindEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    t.rt.open_dot()
        .hear(&t.rt, words, &view("personal", "default"), &tx)
        .await;
    let mut got = Vec::new();
    while let Ok(e) = rx.try_recv() {
        got.push(e);
    }
    got
}

fn said(events: &[MindEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            MindEvent::Say { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

fn asked(events: &[MindEvent]) -> Option<(String, Vec<String>)> {
    events.iter().find_map(|e| match e {
        MindEvent::Ask { question, options } => Some((question.clone(), options.clone())),
        _ => None,
    })
}

fn yes_no() -> Vec<String> {
    vec!["Yes".into(), "No".into()]
}

#[tokio::test]
async fn what_needs_me_reads_the_inbox() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    t.pending_approval(&dot, "Bash", json!({"command": "npm test"}))
        .await;
    t.script(turn("Here's what needs you.\nDO {\"act\":\"needs\"}", None));
    let got = hear(&t, "what needs me").await;
    assert_eq!(
        said(&got),
        "Here's what needs you. One thing needs you. Research wants approval to use Bash."
    );
    assert!(got.iter().any(|e| matches!(
        e,
        MindEvent::Card { card: Card::List { lines, .. } } if lines.len() == 1
    )));
    assert_eq!(
        got.last(),
        Some(&MindEvent::SayEnd {
            then: Then::FollowUp
        })
    );
    // the request's use is counted
    let used = t.rt.store().open_dot_usage_since(None).await.unwrap();
    assert_eq!(used.map(|u| u.requests), Some(1));
    t.rt.shutdown();
}

#[tokio::test]
async fn approve_by_voice_end_to_end() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    let approval = t
        .pending_approval(&dot, "Bash", json!({"command": "npm test"}))
        .await;
    t.script(turn(
        "Approving it.\nDO {\"act\":\"approve\",\"a\":\"a1\"}",
        None,
    ));
    let got = hear(&t, "approve the research one").await;
    let (question, options) = asked(&got).expect("a question");
    assert!(
        question.starts_with("Research wants to use Bash"),
        "{question}"
    );
    assert_eq!(options, yes_no());
    assert_eq!(
        t.rt.store()
            .get_approval(&approval.id)
            .await
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
    let got = hear(&t, "yes").await;
    assert_eq!(said(&got), "Approved.");
    assert_eq!(
        t.rt.store()
            .get_approval(&approval.id)
            .await
            .unwrap()
            .status,
        ApprovalStatus::Approved
    );
    let resumed = wait_run(&t, &dot.id, |r| r.trigger == TriggerKind::Resume).await;
    wait_status(t.rt.store(), &resumed, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

fn downloads_proposal() -> Value {
    json!({"proposal": {
        "name": "Downloads digest",
        "instructions": "1. List the files.",
        "folder": null,
        "schedule": {"rule": {"kind": "daily", "time": "09:00"}},
        "preset": "read-only",
        "extras": [],
        "uses": {},
        "first_message": "Tell me what is in Downloads.",
        "notes": ""
    }})
}

const CREATE: &str =
    "Let's make it.\nDO {\"act\":\"create\",\"description\":\"digest my downloads\"}";

#[tokio::test]
async fn set_up_a_dot_by_voice_end_to_end() {
    let t = Rt::start().await;
    t.script(turn(
        CREATE,
        Some(json!({"questions": ["Which folder?", "What time?"]})),
    ));
    let got = hear(&t, "make a dot that digests my downloads").await;
    assert_eq!(said(&got), "Let's make it. Let me set that up.");
    assert_eq!(asked(&got), Some(("Which folder?".into(), vec![])));
    assert_eq!(got.last(), Some(&MindEvent::SayEnd { then: Then::Ask }));
    // the second question needs no request
    let got = hear(&t, "my downloads folder").await;
    assert_eq!(asked(&got), Some(("What time?".into(), vec![])));
    // the round is answered: the agent proposes
    t.script(turn("Not asked.", Some(downloads_proposal())));
    let got = hear(&t, "nine in the morning").await;
    assert!(got.iter().any(|e| matches!(
        e,
        MindEvent::Card { card: Card::Proposal { proposal } } if proposal.name == "Downloads digest"
    )));
    assert_eq!(
        asked(&got),
        Some((
            "Downloads digest. Runs every day at 09:00. The rest is on screen. Create it?".into(),
            yes_no()
        ))
    );
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(t.dir.path().join("args.json")).unwrap())
            .unwrap();
    let sent = record["prompt"].as_str().unwrap();
    for part in [
        "digest my downloads",
        "Which folder?",
        "my downloads folder",
        "nine in the morning",
    ] {
        assert!(sent.contains(part), "{part} in {sent}");
    }
    assert!(t.rt.store().list_dots().await.unwrap().is_empty());
    // yes: made, and its first message sent
    t.script(succeeding());
    let got = hear(&t, "yes").await;
    assert_eq!(said(&got), "Created Downloads digest.");
    let dots = t.rt.store().list_dots().await.unwrap();
    assert_eq!(dots.len(), 1);
    assert!(got.contains(&MindEvent::Open {
        target: OpenTarget::Dot {
            dot_id: dots[0].id.clone()
        }
    }));
    let first = wait_run(&t, &dots[0].id, |r| {
        message_of(r) == Some("Tell me what is in Downloads.")
    })
    .await;
    wait_status(t.rt.store(), &first, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

#[tokio::test]
async fn set_up_declined_creates_nothing() {
    let t = Rt::start().await;
    t.script(turn(CREATE, Some(downloads_proposal())));
    let got = hear(&t, "make a dot that digests my downloads").await;
    assert_eq!(asked(&got).map(|a| a.1), Some(yes_no()));
    let got = hear(&t, "no").await;
    assert_eq!(said(&got), "Okay, not created.");
    assert!(t.rt.store().list_dots().await.unwrap().is_empty());
    assert!(!t.rt.open_dot().pending());
    // the mind's request and the setup agent's request it made are both counted
    let used = t.rt.store().open_dot_usage_since(None).await.unwrap();
    assert_eq!(used.map(|u| u.requests), Some(2));
    t.rt.shutdown();
}

#[tokio::test]
async fn ask_a_dot_sends_and_says_to_whom() {
    let t = Rt::start().await;
    let dot = t.make_dot("Research", None, "default").await;
    t.script(turn(
        "Sending it.\nDO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"look into the build\"}",
        None,
    ));
    let got = hear(&t, "ask research to look into the build").await;
    assert_eq!(said(&got), "Sending it. Sent to Research.");
    let id = wait_run(&t, &dot.id, |r| {
        message_of(r) == Some("look into the build")
    })
    .await;
    wait_status(t.rt.store(), &id, RunStatus::Succeeded, 15).await;
    t.rt.shutdown();
}

// ---- What Open Dot used, and the caps ----

fn usage(tokens_in: u64, tokens_out: u64, cost_usd: Option<f64>) -> OnceUsage {
    OnceUsage {
        tokens_in,
        tokens_out,
        cache_read: 3,
        cost_usd,
    }
}

/// The local day of `now`; each test takes one `now` for its writes and this, so they cannot
/// fall on two sides of midnight.
fn today(now: chrono::DateTime<chrono::Utc>) -> (String, String) {
    day_bounds_in(now, &chrono::Local)
}

/// `now` as the store writes times.
fn at(now: chrono::DateTime<chrono::Utc>) -> String {
    dots_core::util::timestamp(now)
}

#[tokio::test]
async fn usage_is_recorded_and_summed() {
    let (_d, store) = temp_store().await;
    let now = chrono::Utc::now();
    assert_eq!(store.open_dot_usage_since(None).await.unwrap(), None);
    // no request reported a cost: none is known
    store
        .add_open_dot_usage_at("haiku", &usage(50, 5, None), &at(now))
        .await
        .unwrap();
    assert_eq!(
        store.open_dot_usage_since(None).await.unwrap(),
        Some(OpenDotUsage {
            requests: 1,
            tokens_in: 50,
            tokens_out: 5,
            cost_usd: None,
        })
    );
    store
        .add_open_dot_usage_at("haiku", &usage(100, 20, Some(0.01)), &at(now))
        .await
        .unwrap();
    assert_eq!(
        store.open_dot_usage_since(None).await.unwrap(),
        Some(OpenDotUsage {
            requests: 2,
            tokens_in: 150,
            tokens_out: 25,
            cost_usd: Some(0.01),
        })
    );
    let (start, end) = today(now);
    assert_eq!(
        store.open_dot_spend_between(&start, &end).await.unwrap(),
        Spend {
            cost_usd: 0.01,
            tokens: 175,
        }
    );
    // nothing since a time in the future
    let later = at(now + chrono::TimeDelta::hours(1));
    assert_eq!(
        store.open_dot_usage_since(Some(&later)).await.unwrap(),
        None
    );
    assert_eq!(
        store.open_dot_spend_between(&later, "9999").await.unwrap(),
        Spend::default()
    );
}

/// A run of a new dot started at `now` that used `tokens` and `cost`; returns the dot's id.
async fn a_run(
    store: &dots_core::store::Store,
    now: chrono::DateTime<chrono::Utc>,
    tokens: (i64, i64),
    cost: f64,
) -> String {
    let dot = store
        .create_dot(&spec("Spender"), None, "default")
        .await
        .unwrap();
    let r = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    store.set_run_times(&r.id, &at(now), None).await.unwrap();
    store
        .add_usage(&r.id, tokens.0, tokens.1, 0, Some(cost))
        .await
        .unwrap();
    dot.id
}

#[tokio::test]
async fn global_spend_includes_open_dot() {
    let (_d, store) = temp_store().await;
    let now = chrono::Utc::now();
    a_run(&store, now, (1000, 100), 0.5).await;
    store
        .add_open_dot_usage_at("haiku", &usage(200, 40, Some(0.25)), &at(now))
        .await
        .unwrap();
    let (start, end) = today(now);
    assert_eq!(
        store.spend_between(None, &start, &end).await.unwrap(),
        Spend {
            cost_usd: 0.75,
            tokens: 1340,
        }
    );
}

#[tokio::test]
async fn per_dot_spend_does_not() {
    let (_d, store) = temp_store().await;
    let now = chrono::Utc::now();
    let dot_id = a_run(&store, now, (1000, 100), 0.5).await;
    store
        .add_open_dot_usage_at("haiku", &usage(200, 40, Some(0.25)), &at(now))
        .await
        .unwrap();
    let (start, end) = today(now);
    assert_eq!(
        store
            .spend_between(Some(&dot_id), &start, &end)
            .await
            .unwrap(),
        Spend {
            cost_usd: 0.5,
            tokens: 1100,
        }
    );
}

#[tokio::test]
async fn usage_summary_carries_open_dot() {
    let t = Rt::start().await;
    let s = t.rt.usage_summary(UsagePeriod::Week).await.unwrap();
    assert_eq!(s.open_dot, None);
    t.rt.store()
        .add_open_dot_usage("haiku", &usage(10, 2, Some(0.002)))
        .await
        .unwrap();
    let s = t.rt.usage_summary(UsagePeriod::FiveHours).await.unwrap();
    assert_eq!(
        s.open_dot,
        Some(OpenDotUsage {
            requests: 1,
            tokens_in: 10,
            tokens_out: 2,
            cost_usd: Some(0.002),
        })
    );
    // the dots' rows are not changed by it
    assert!(s.rows.is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn cap_hit_names_the_daily_cap() {
    let (_d, store) = temp_store().await;
    let now = chrono::Utc::now();
    let mut settings = Settings {
        spend_daily_usd: Some(1.0),
        ..Settings::default()
    };
    assert_eq!(cap_hit(&store, &settings, now).await.unwrap(), None);
    store
        .add_open_dot_usage_at("haiku", &usage(10, 2, Some(1.5)), &at(now))
        .await
        .unwrap();
    let said = cap_hit(&store, &settings, now)
        .await
        .unwrap()
        .expect("a cap");
    assert!(said.contains("today's spending limit"), "{said}");
    assert_eq!(said.matches('.').count(), 1, "one sentence: {said}");
    // the token cap and the monthly cap have words of their own
    settings.spend_daily_usd = None;
    settings.spend_daily_tokens = Some(5);
    let said = cap_hit(&store, &settings, now)
        .await
        .unwrap()
        .expect("a cap");
    assert!(said.contains("today's token limit"), "{said}");
    settings.spend_daily_tokens = None;
    settings.spend_monthly_usd = Some(1.0);
    let said = cap_hit(&store, &settings, now)
        .await
        .unwrap()
        .expect("a cap");
    assert!(said.contains("this month's spending limit"), "{said}");
    // no cap set: nothing to say
    assert_eq!(
        cap_hit(&store, &Settings::default(), now).await.unwrap(),
        None
    );
}

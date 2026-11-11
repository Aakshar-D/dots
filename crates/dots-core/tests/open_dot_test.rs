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
    Approval, Dot, DotSpec, Message, NewRun, RunStatus, TriggerKind, UsagePeriod, WorkspaceMode,
};
use dots_core::open_dot::picture::{Picture, View};
use dots_core::open_dot::prompt::{prompt, Turn};
use dots_core::open_dot::think::{cap_hit, CliThinker, Thinker, THINK_LIMIT};
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

// ---- What Open Dot used, and the caps ----

fn usage(tokens_in: u64, tokens_out: u64, cost_usd: Option<f64>) -> OnceUsage {
    OnceUsage {
        tokens_in,
        tokens_out,
        cache_read: 3,
        cost_usd,
    }
}

fn today() -> (String, String) {
    day_bounds_in(chrono::Utc::now(), &chrono::Local)
}

#[tokio::test]
async fn usage_is_recorded_and_summed() {
    let (_d, store) = temp_store().await;
    assert_eq!(store.open_dot_usage_since(None).await.unwrap(), None);
    store
        .add_open_dot_usage("haiku", &usage(100, 20, Some(0.01)))
        .await
        .unwrap();
    store
        .add_open_dot_usage("haiku", &usage(50, 5, None))
        .await
        .unwrap();
    assert_eq!(
        store.open_dot_usage_since(None).await.unwrap(),
        Some(OpenDotUsage {
            requests: 2,
            tokens_in: 150,
            tokens_out: 25,
            cost_usd: 0.01,
        })
    );
    let (start, end) = today();
    assert_eq!(
        store.open_dot_spend_between(&start, &end).await.unwrap(),
        Spend {
            cost_usd: 0.01,
            tokens: 175,
        }
    );
    // nothing since a time in the future
    let later = dots_core::util::timestamp(chrono::Utc::now() + chrono::TimeDelta::hours(1));
    assert_eq!(
        store.open_dot_usage_since(Some(&later)).await.unwrap(),
        None
    );
    assert_eq!(
        store.open_dot_spend_between(&later, "9999").await.unwrap(),
        Spend::default()
    );
}

/// A run of a new dot started now that used `tokens` and `cost`; returns the dot's id.
async fn a_run(store: &dots_core::store::Store, tokens: (i64, i64), cost: f64) -> String {
    let dot = store
        .create_dot(&spec("Spender"), None, "default")
        .await
        .unwrap();
    let r = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let now = dots_core::util::now();
    store.set_run_times(&r.id, &now, None).await.unwrap();
    store
        .add_usage(&r.id, tokens.0, tokens.1, 0, Some(cost))
        .await
        .unwrap();
    dot.id
}

#[tokio::test]
async fn global_spend_includes_open_dot() {
    let (_d, store) = temp_store().await;
    a_run(&store, (1000, 100), 0.5).await;
    store
        .add_open_dot_usage("haiku", &usage(200, 40, Some(0.25)))
        .await
        .unwrap();
    let (start, end) = today();
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
    let dot_id = a_run(&store, (1000, 100), 0.5).await;
    store
        .add_open_dot_usage("haiku", &usage(200, 40, Some(0.25)))
        .await
        .unwrap();
    let (start, end) = today();
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
            cost_usd: 0.002,
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
        .add_open_dot_usage("haiku", &usage(10, 2, Some(1.5)))
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

//! Drafting a skill with the `claude` CLI (skill creator spec §5), against the fake CLI.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dots_core::skill_creator::{
    draft_prompt, unwrap_fence, DraftRequest, DRAFTING_INSTRUCTIONS, MAX_REQUEST_CHARS,
};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

const DRAFT: &str = "---\nname: pr-tests\ndescription: \"Checks a PR for missing tests.\"\n---\n\
                     # Missing tests\n\nList the changed functions.\n";

struct Rt {
    dir: TempDir,
    rt: Arc<Runtime>,
    args_out: PathBuf,
}

impl Rt {
    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// What the fake CLI recorded: its arguments, stdin and folder.
    fn recorded(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.args_out).unwrap()).unwrap()
    }
}

async fn start_with(script: Value, change: impl FnOnce(&mut Config)) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let args_out = dir.path().join("args.json");
    let script_path = dir.path().join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script_path.to_string_lossy().to_string(),
    ));
    change(&mut cfg);
    let rt = Arc::new(Runtime::start(cfg).await.unwrap());
    Rt { dir, rt, args_out }
}

async fn start(script: Value) -> Rt {
    start_with(script, |_| {}).await
}

/// A script that answers `text` as the result.
fn answering(text: &str) -> Value {
    json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"emit": {"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": text,
                  "usage": {"input_tokens": 1, "output_tokens": 1}}},
    ])
}

fn idea(text: &str) -> DraftRequest {
    DraftRequest {
        idea: text.into(),
        model: "haiku".into(),
        current: None,
        change: None,
    }
}

fn message(err: Error) -> String {
    err.to_string()
}

#[tokio::test]
async fn a_draft_asks_the_cli_once_with_no_tools_and_returns_its_answer() {
    let t = start(answering(DRAFT)).await;
    let text =
        t.rt.skill_drafts()
            .draft("d1", &idea("Check a PR for missing tests"))
            .await
            .unwrap();
    assert_eq!(text, DRAFT);
    let rec = t.recorded();
    let args: Vec<String> = rec["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let pair = |name: &str| {
        let i = args
            .iter()
            .position(|a| a == name)
            .unwrap_or_else(|| panic!("{name}"));
        args[i + 1].clone()
    };
    assert_eq!(args[0], "-p");
    assert_eq!(pair("--output-format"), "stream-json");
    assert!(args.contains(&"--verbose".to_string()));
    assert_eq!(pair("--model"), "haiku");
    assert_eq!(pair("--max-turns"), "1");
    assert_eq!(pair("--tools"), "");
    assert!(args.contains(&"--strict-mcp-config".to_string()));
    assert_eq!(pair("--setting-sources"), "");
    assert!(args.contains(&"--no-session-persistence".to_string()));
    assert_eq!(pair("--system-prompt"), DRAFTING_INSTRUCTIONS);
    for absent in [
        "--mcp-config",
        "--plugin-dir",
        "--allowedTools",
        "--disallowedTools",
        "--permission-prompt-tool",
        "--resume",
    ] {
        assert!(!args.contains(&absent.to_string()), "{absent}");
    }
    assert!(rec["mcp_config"].is_null());
    assert!(rec["prompt"]
        .as_str()
        .unwrap()
        .contains("Check a PR for missing tests"));
    // It ran in a folder of its own under the data folder's drafts, gone afterwards.
    let cwd = PathBuf::from(rec["cwd"].as_str().unwrap());
    assert!(
        cwd.parent().unwrap().ends_with("data/drafts"),
        "{}",
        cwd.display()
    );
    assert!(!cwd.exists());
    assert!(t.data().join("drafts").is_dir());
}

#[tokio::test]
async fn a_change_sends_the_idea_the_current_text_and_the_change() {
    let t = start(answering(DRAFT)).await;
    let req = DraftRequest {
        idea: "Check a PR for missing tests".into(),
        model: "sonnet".into(),
        current: Some("---\nname: old\n---\nMy hand edit\n".into()),
        change: Some("Mention snapshot tests".into()),
    };
    t.rt.skill_drafts().draft("d1", &req).await.unwrap();
    let prompt = t.recorded()["prompt"].as_str().unwrap().to_string();
    assert!(prompt.contains("Check a PR for missing tests"));
    assert!(prompt.contains("<<<SKILL.md\n---\nname: old\n---\nMy hand edit\nSKILL.md>>>"));
    assert!(prompt.contains("Change it like this:\n\nMention snapshot tests"));
    assert!(prompt.contains("whole revised SKILL.md"));
}

#[tokio::test]
async fn a_fenced_answer_is_unwrapped() {
    let t = start(answering(&format!("```markdown\n{DRAFT}```"))).await;
    let text = t.rt.skill_drafts().draft("d1", &idea("x")).await.unwrap();
    assert_eq!(text, DRAFT);
}

#[test]
fn only_an_answer_that_is_one_fenced_block_loses_its_fence() {
    assert_eq!(unwrap_fence(&format!("\n\n{DRAFT}\n\n")), DRAFT);
    assert_eq!(unwrap_fence(&format!("```\n{DRAFT}```")), DRAFT);
    assert_eq!(unwrap_fence(&format!("```md\n{DRAFT}\n```\n")), DRAFT);
    // A fence inside the instructions stays, and so does the text around a block.
    let inside = format!("{DRAFT}\n```sh\ncargo test\n```\n");
    assert_eq!(unwrap_fence(&inside), inside.trim().to_string() + "\n");
    let two = "```\na\n```\ntext\n```\nb\n```";
    assert_eq!(unwrap_fence(two), format!("{two}\n"));
}

#[tokio::test]
async fn an_error_an_exit_or_an_empty_answer_fails_the_draft() {
    let failed = json!([
        {"emit": {"type": "result", "subtype": "error_during_execution", "is_error": true,
                  "result": "Invalid API key", "usage": {}}},
    ]);
    let t = start(failed).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("Claude could not write the skill"), "{err}");
    assert!(err.contains("Invalid API key"), "{err}");

    let exited = json!([{"stderr": "Please run /login"}, {"exit": 1}]);
    let t = start(exited).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("exited with code 1"), "{err}");
    assert!(err.contains("Please run /login"), "{err}");

    let t = start(answering("  \n")).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("no text"), "{err}");

    // A fence with nothing inside is an empty answer too.
    let t = start(answering("```\n```")).await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("no text"), "{err}");
}

/// Waits until the fake CLI has recorded its start.
async fn started(t: &Rt) {
    let end = Instant::now() + Duration::from_secs(20);
    while !t.args_out.exists() {
        assert!(Instant::now() < end, "the CLI did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn cancel_ends_the_cli_and_the_draft() {
    let t = start(json!([{"sleep_ms": 30000}, {"emit": {"type": "result",
        "subtype": "success", "is_error": false, "result": DRAFT}}]))
    .await;
    let rt = t.rt.clone();
    let task = tokio::spawn(async move { rt.skill_drafts().draft("d1", &idea("x")).await });
    started(&t).await;
    let begun = Instant::now();
    t.rt.skill_drafts().cancel("d1");
    let err = message(task.await.unwrap().unwrap_err());
    assert!(err.contains("cancelled"), "{err}");
    assert!(begun.elapsed() < Duration::from_secs(10));
    // A cancel with nothing running does nothing.
    t.rt.skill_drafts().cancel("d1");
}

#[tokio::test]
async fn a_draft_that_takes_too_long_is_ended() {
    let t = start_with(json!([{"sleep_ms": 30000}]), |cfg| {
        cfg.draft_limit = Duration::from_millis(500)
    })
    .await;
    let begun = Instant::now();
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("took too long"), "{err}");
    assert!(begun.elapsed() < Duration::from_secs(15));
}

#[tokio::test]
async fn a_new_draft_of_the_same_dialog_cancels_the_one_before() {
    let t = start(json!([{"sleep_ms": 30000}])).await;
    let rt = t.rt.clone();
    let first = tokio::spawn(async move { rt.skill_drafts().draft("d1", &idea("x")).await });
    started(&t).await;
    std::fs::remove_file(&t.args_out).unwrap();
    // Another dialog's draft is left alone.
    let rt = t.rt.clone();
    let other = tokio::spawn(async move { rt.skill_drafts().draft("d2", &idea("y")).await });
    started(&t).await;
    std::fs::remove_file(&t.args_out).unwrap();
    let rt = t.rt.clone();
    let second = tokio::spawn(async move { rt.skill_drafts().draft("d1", &idea("z")).await });
    let err = message(first.await.unwrap().unwrap_err());
    assert!(err.contains("cancelled"), "{err}");
    assert!(!other.is_finished());
    t.rt.skill_drafts().cancel("d1");
    t.rt.skill_drafts().cancel("d2");
    assert!(second.await.unwrap().is_err());
    assert!(other.await.unwrap().is_err());
}

#[tokio::test]
async fn a_request_that_cannot_be_sent_is_refused_before_the_cli_starts() {
    let t = start(answering(DRAFT)).await;
    let drafts = t.rt.skill_drafts();
    let refused = |r: Result<String, Error>| match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    };
    assert!(refused(drafts.draft("d", &idea("  ")).await).contains("Describe the skill"));
    let mut no_model = idea("x");
    no_model.model = " ".into();
    assert!(refused(drafts.draft("d", &no_model).await).contains("Pick a model"));
    let mut lone = idea("x");
    lone.change = Some("shorter".into());
    assert!(refused(drafts.draft("d", &lone).await).contains("what should change"));
    let mut blank = idea("x");
    blank.current = Some(DRAFT.into());
    blank.change = Some(" ".into());
    assert!(refused(drafts.draft("d", &blank).await).contains("what should change"));
    let long = idea(&"x".repeat(MAX_REQUEST_CHARS + 1));
    assert!(refused(drafts.draft("d", &long).await).contains("at most 4000"));
    assert!(!t.args_out.exists());
}

#[test]
fn the_instructions_name_the_rules_a_draft_is_checked_against() {
    for rule in [
        "complete SKILL.md and nothing else",
        "the first line is ---",
        "name: <name>",
        "lowercase letters, digits and hyphens",
        "at most 64 characters",
        "description: \"",
        "double quotes",
        "No lists, no indented lines, no other keys",
        "whole revised file",
    ] {
        assert!(DRAFTING_INSTRUCTIONS.contains(rule), "{rule}");
    }
    let req = idea("Check a PR");
    assert_eq!(
        draft_prompt(&req).unwrap(),
        "Write a skill for this idea:\n\nCheck a PR\n"
    );
}

#[tokio::test]
async fn without_the_cli_a_draft_says_how_to_get_it() {
    let t = start_with(answering(DRAFT), |cfg| {
        cfg.claude_path = Some(PathBuf::from("C:/no/such/claude.exe"))
    })
    .await;
    let err = message(
        t.rt.skill_drafts()
            .draft("d1", &idea("x"))
            .await
            .unwrap_err(),
    );
    assert!(err.contains("claude CLI not found"), "{err}");
}

#[tokio::test]
async fn drafts_work_while_the_local_servers_port_is_taken() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let t = start_with(answering(DRAFT), |cfg| {
        cfg.port = port;
        cfg.bind_wait = Duration::from_millis(100);
    })
    .await;
    assert!(t.rt.status().await.server_error.is_some());
    let text = t.rt.skill_drafts().draft("d1", &idea("x")).await.unwrap();
    assert_eq!(text, DRAFT);
}

#[tokio::test]
async fn starting_dots_removes_drafts_folders_a_crash_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let left = data.join("drafts").join("01J0LEFT");
    std::fs::create_dir_all(&left).unwrap();
    let mut cfg = Config::new(&data);
    cfg.port = 0;
    let _rt = Runtime::start(cfg).await.unwrap();
    assert!(!left.exists());
    assert!(!Path::new(&data.join("drafts")).exists());
}

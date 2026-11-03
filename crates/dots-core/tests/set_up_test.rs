//! The setup agent's runner and `Runtime::setup_ask / setup_cancel / setup_apply` (setup agent
//! spec §6, §9), against the fake CLI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dots_core::library::Uses;
use dots_core::model::{folder_name, DotSpec, WorkspaceMode};
use dots_core::policy::{Action, Preset, Rule};
use dots_core::schedule::Schedule;
use dots_core::setup::{SetupAnswer, SetupProposal, SetupRequest};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

const SKILL: &str = "---\nname: pdf-tools\ndescription: \"Reads and splits PDF files.\"\n---\n\
                     # PDF tools\n\nSplit the PDF.\n";

struct Rt {
    _dir: TempDir,
    rt: Arc<Runtime>,
    args_out: PathBuf,
    root: PathBuf,
}

impl Rt {
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
    let root = dir.path().join("Dots");
    cfg.new_dot_root = root.clone();
    change(&mut cfg);
    let rt = Arc::new(Runtime::start(cfg).await.unwrap());
    Rt {
        _dir: dir,
        rt,
        args_out,
        root,
    }
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

fn req(description: &str) -> SetupRequest {
    SetupRequest {
        description: description.into(),
        rounds: vec![],
        current: None,
        change: None,
        skip: false,
    }
}

fn proposal(name: &str) -> SetupProposal {
    SetupProposal {
        name: name.into(),
        instructions: "1. List the files.".into(),
        folder: None,
        schedule: None,
        preset: Preset::ReadOnly,
        extras: vec!["web".into()],
        uses: Uses::default(),
        first_message: "Tell me what is in Downloads.".into(),
        notes: String::new(),
    }
}

fn message(err: Error) -> String {
    err.to_string()
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
async fn ask_returns_questions() {
    let answer = json!({"questions": ["Which folder?", "How often?"]}).to_string();
    let t = start(answering(&answer)).await;
    let got = t.rt.setup_ask("s1", req("Sort my files")).await.unwrap();
    assert_eq!(
        got,
        SetupAnswer::Questions {
            questions: vec!["Which folder?".into(), "How often?".into()]
        }
    );
}

#[tokio::test]
async fn ask_returns_checked_proposal() {
    let answer = json!({"proposal": {
        "name": "Downloads digest",
        "instructions": "1. List the files.",
        "folder": null,
        "schedule": null,
        "preset": "read-only",
        "uses": {"skills": ["no-such-skill"]},
        "first_message": "Go.",
        "notes": ""
    }})
    .to_string();
    let t = start(answering(&answer)).await;
    let got = t.rt.setup_ask("s1", req("Sort my files")).await.unwrap();
    let SetupAnswer::Proposal {
        proposal,
        new_dot_root,
    } = got
    else {
        panic!("expected a proposal, got {got:?}");
    };
    assert_eq!(new_dot_root, t.root.to_string_lossy());
    assert_eq!(proposal.name, "Downloads digest");
    assert!(proposal.uses.skills.is_empty());
    assert!(
        proposal
            .notes
            .contains("Left out skill no-such-skill: it is not in your Library."),
        "{}",
        proposal.notes
    );
}

#[tokio::test]
async fn ask_sends_context() {
    let t = start(answering(r#"{"questions": ["Which folder?"]}"#)).await;
    t.rt.library().create_skill(SKILL).await.unwrap();
    t.rt.create_dot(DotSpec::new("Inbox helper", "Read mail.", ""), None, None)
        .await
        .unwrap();
    let ctx = t.rt.setup_context().await.unwrap();
    assert_eq!(
        ctx.library.skills,
        vec![(
            "pdf-tools".to_string(),
            "Reads and splits PDF files.".to_string()
        )]
    );
    assert_eq!(ctx.dot_names, vec!["Inbox helper".to_string()]);
    assert_eq!(ctx.new_dot_root, t.root.to_string_lossy());

    t.rt.setup_ask("s1", req("Sort my files")).await.unwrap();
    let rec = t.recorded();
    let prompt = rec["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("- pdf-tools: Reads and splits PDF files."),
        "{prompt}"
    );
    assert!(prompt.contains("- Inbox helper"), "{prompt}");
    assert!(prompt.contains("Sort my files"), "{prompt}");
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
    assert_eq!(pair("--max-turns"), "1");
    assert_eq!(pair("--tools"), "");
    assert_eq!(pair("--model"), "sonnet");
    assert_eq!(
        pair("--system-prompt"),
        dots_core::setup::SETUP_INSTRUCTIONS
    );
}

#[tokio::test]
async fn ask_cancel() {
    let t = start(json!([{"sleep_ms": 10000}])).await;
    let rt = t.rt.clone();
    let task = tokio::spawn(async move { rt.setup_ask("s1", req("x")).await });
    started(&t).await;
    let begun = Instant::now();
    t.rt.setup_cancel("s1");
    let err = message(task.await.unwrap().unwrap_err());
    assert_eq!(err, "The setup was cancelled.");
    assert!(begun.elapsed() < Duration::from_secs(8));
    // A cancel with nothing running does nothing.
    t.rt.setup_cancel("s1");

    // A second ask with the same id cancels the first.
    std::fs::remove_file(&t.args_out).unwrap();
    let rt = t.rt.clone();
    let first = tokio::spawn(async move { rt.setup_ask("s1", req("x")).await });
    started(&t).await;
    let rt = t.rt.clone();
    let second = tokio::spawn(async move { rt.setup_ask("s1", req("y")).await });
    let err = message(first.await.unwrap().unwrap_err());
    assert_eq!(err, "The setup was cancelled.");
    t.rt.setup_cancel("s1");
    assert!(second.await.unwrap().is_err());
}

#[tokio::test]
async fn ask_times_out() {
    let t = start_with(json!([{"sleep_ms": 10000}]), |cfg| {
        cfg.draft_limit = Duration::from_millis(200)
    })
    .await;
    let err = message(t.rt.setup_ask("s1", req("x")).await.unwrap_err());
    assert_eq!(err, "Claude took too long to set up the dot.");
}

#[tokio::test]
async fn ask_fails_with_the_cli_error() {
    let failed = json!([
        {"emit": {"type": "result", "subtype": "error_during_execution", "is_error": true,
                  "result": "Invalid API key", "usage": {}}},
    ]);
    let t = start(failed).await;
    let err = message(t.rt.setup_ask("s1", req("x")).await.unwrap_err());
    assert!(
        err.starts_with("Claude could not set up the dot: "),
        "{err}"
    );
    assert!(err.contains("Invalid API key"), "{err}");
}

#[tokio::test]
async fn ask_without_claude() {
    let t = start_with(answering("{}"), |cfg| {
        cfg.claude_path = Some(PathBuf::from("C:/no/such/claude.exe"))
    })
    .await;
    let err = message(t.rt.setup_ask("s1", req("x")).await.unwrap_err());
    assert!(err.contains("claude CLI not found"), "{err}");
}

#[tokio::test]
async fn ask_refuses_a_bad_request_before_the_cli_starts() {
    let t = start(answering("{}")).await;
    let err = t.rt.setup_ask("s1", req("  ")).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert!(!t.args_out.exists());
}

#[tokio::test]
async fn apply_creates_dot_and_chat() {
    let t = start(answering("Done.")).await;
    let applied =
        t.rt.setup_apply(proposal("Downloads digest"), None, None)
            .await
            .unwrap();
    let dot = t.rt.get_dot(&applied.dot.id).await.unwrap();
    assert_eq!(dot.spec.name, "Downloads digest");
    assert_eq!(dot.spec.instructions, "1. List the files.");
    assert_eq!(
        PathBuf::from(&dot.spec.workdir),
        t.root.join(folder_name("Downloads digest"))
    );
    assert!(PathBuf::from(&dot.spec.workdir).is_dir());
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
    assert_eq!(dot.spec.policy.preset, Preset::Custom);
    assert!(dot
        .spec
        .policy
        .rules
        .contains(&Rule::new("WebFetch", Action::Allow)));
    let run_id = applied.run_id.expect("a chat run");
    assert!(applied.run_error.is_none());
    let run = t.rt.get_run(&run_id).await.unwrap();
    assert_eq!(run.dot_id, dot.id);
    assert_eq!(
        run.payload.unwrap()["message"],
        "Tell me what is in Downloads."
    );
}

#[tokio::test]
async fn apply_git_folder_gets_worktree() {
    let t = start(answering("Done.")).await;
    let repo = tempfile::tempdir().unwrap();
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let mut p = proposal("Repo helper");
    p.folder = Some(repo.path().to_string_lossy().into_owned());
    let applied = t.rt.setup_apply(p, None, None).await.unwrap();
    assert_eq!(applied.dot.spec.workspace_mode, WorkspaceMode::Worktree);
    assert_eq!(
        PathBuf::from(&applied.dot.spec.workdir),
        repo.path().to_path_buf()
    );
    // No default folder was made.
    assert!(!t.root.join(folder_name("Repo helper")).exists());
}

#[tokio::test]
async fn apply_twice_same_name_conflicts() {
    let t = start(answering("Done.")).await;
    t.rt.setup_apply(proposal("Downloads digest"), None, None)
        .await
        .unwrap();
    let err =
        t.rt.setup_apply(proposal("downloads DIGEST"), None, None)
            .await
            .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(t.rt.list_dots().await.unwrap().len(), 1);
    let folders = std::fs::read_dir(&t.root).unwrap().count();
    assert_eq!(folders, 1);
}

#[tokio::test]
async fn apply_twice_at_once_makes_one_dot() {
    let t = start(answering("Done.")).await;
    let (a, b) = tokio::join!(
        t.rt.setup_apply(proposal("Downloads digest"), None, None),
        t.rt.setup_apply(proposal("Downloads digest"), None, None),
    );
    assert!(a.is_ok() != b.is_ok(), "{a:?} {b:?}");
    let dots = t.rt.list_dots().await.unwrap();
    assert_eq!(dots.len(), 1);
    // The dot that was made keeps its folder.
    assert!(PathBuf::from(&dots[0].spec.workdir).is_dir());
}

#[tokio::test]
async fn apply_create_fails_removes_folder() {
    let t = start(answering("Done.")).await;
    let err =
        t.rt.setup_apply(
            proposal("Downloads digest"),
            Some("no-such-org".into()),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert!(!t.root.join(folder_name("Downloads digest")).exists());
    assert!(t.rt.list_dots().await.unwrap().is_empty());
}

#[tokio::test]
async fn apply_refuses_a_folder_that_is_not_there() {
    let t = start(answering("Done.")).await;
    let gone = tempfile::tempdir().unwrap();
    let path = gone.path().to_string_lossy().into_owned();
    drop(gone);
    let mut p = proposal("Gone folder");
    p.folder = Some(path.clone());
    let err = t.rt.setup_apply(p, None, None).await.unwrap_err();
    match err {
        Error::Invalid(m) => assert_eq!(m, format!("{path} is not a folder on this PC.")),
        e => panic!("expected Invalid, got {e:?}"),
    }
    assert!(t.rt.list_dots().await.unwrap().is_empty());
    assert!(!t.root.exists());
}

#[tokio::test]
async fn apply_plain_folder_gets_folder_mode_and_keeps_uses_and_schedule() {
    let t = start(answering("Done.")).await;
    t.rt.library().create_skill(SKILL).await.unwrap();
    let plain = tempfile::tempdir().unwrap();
    let mut p = proposal("Plain helper");
    p.folder = Some(plain.path().to_string_lossy().into_owned());
    p.extras = vec![];
    p.preset = Preset::Sandboxed;
    p.uses.skills = vec!["pdf-tools".into()];
    let daily: Schedule =
        serde_json::from_value(json!({"rule": {"kind": "daily", "time": "09:00"}})).unwrap();
    p.schedule = Some(daily.clone());
    let applied = t.rt.setup_apply(p, None, None).await.unwrap();
    let spec = &applied.dot.spec;
    assert_eq!(spec.workspace_mode, WorkspaceMode::Folder);
    assert_eq!(
        spec.policy,
        dots_core::policy::Policy::preset(Preset::Sandboxed)
    );
    assert_eq!(spec.uses.skills, vec!["pdf-tools".to_string()]);
    assert_eq!(
        spec.schedule.as_ref().map(|s| s.rule.clone()),
        Some(daily.rule)
    );
}

#[tokio::test]
async fn apply_default_folder_gets_a_suffix_when_taken() {
    let t = start(answering("Done.")).await;
    let taken = t.root.join(folder_name("Downloads digest"));
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("keep.txt"), "mine").unwrap();
    let applied =
        t.rt.setup_apply(proposal("Downloads digest"), None, None)
            .await
            .unwrap();
    assert_eq!(
        PathBuf::from(&applied.dot.spec.workdir),
        t.root
            .join(format!("{}-2", folder_name("Downloads digest")))
    );
}

#[tokio::test]
async fn apply_default_folder_inside_git_gets_worktree() {
    let repo = tempfile::tempdir().unwrap();
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let root = repo.path().join("Dots");
    let t = start_with(answering("Done."), |cfg| cfg.new_dot_root = root.clone()).await;
    let applied =
        t.rt.setup_apply(proposal("Downloads digest"), None, None)
            .await
            .unwrap();
    assert_eq!(
        PathBuf::from(&applied.dot.spec.workdir),
        root.join(folder_name("Downloads digest"))
    );
    assert_eq!(applied.dot.spec.workspace_mode, WorkspaceMode::Worktree);
}

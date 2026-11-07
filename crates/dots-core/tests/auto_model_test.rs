//! Auto (auto model spec): the routing table, a thread's tier, the fallback's error check, the
//! tier models of Settings and Auto in the model list.

mod common;

use common::{fake_dot, fake_run};
use dots_core::auto_model::{
    choose, fallback_tier, is_auto, model_unavailable, route, signals, AutoModels, Signals,
    AUTO_MODEL,
};
use dots_core::model::{EngineKind, ModelTier, TriggerKind, WorkspaceMode};
use dots_core::models::list_models;
use dots_core::policy::{Policy, Preset};
use dots_core::settings::Settings;
use serde_json::json;

use ModelTier::{Large, Small, Standard};
use TriggerKind::{Api, Chat, Manual, Resume, Schedule, Watcher, Webhook};

/// A message to a dot that can act, in a folder, outside any thread.
fn msg(trigger: TriggerKind, text: &str) -> Signals {
    Signals {
        trigger,
        text: text.into(),
        is_message: true,
        attachments: 0,
        can_act: true,
        worktree: false,
        thread: None,
    }
}

fn read_only(s: Signals) -> Signals {
    Signals {
        can_act: false,
        ..s
    }
}

fn worktree(s: Signals) -> Signals {
    Signals {
        worktree: true,
        ..s
    }
}

fn instructions(trigger: TriggerKind, text: &str) -> Signals {
    Signals {
        is_message: false,
        ..msg(trigger, text)
    }
}

fn files(s: Signals, n: usize) -> Signals {
    Signals {
        attachments: n,
        ..s
    }
}

fn in_thread(s: Signals, tier: ModelTier) -> Signals {
    Signals {
        thread: Some(tier),
        ..s
    }
}

#[test]
fn the_routing_table() {
    let long = |words: &str, n: usize| format!("{words} {}", "lorem ipsum ".repeat(n));
    let cases: Vec<(Signals, ModelTier, &str)> = vec![
        // Light requests stay small, even for a dot that can act.
        (read_only(msg(Chat, "hi there")), Small, "light task: hi"),
        (
            msg(Chat, "Summarise this article for me"),
            Small,
            "light task: summarise",
        ),
        (
            msg(Chat, "Translate 'good morning' to French"),
            Small,
            "light task: translate",
        ),
        (
            msg(Api, "list my open pull requests"),
            Small,
            "light task: list",
        ),
        (
            msg(Chat, "Remind me to call Sam at 4"),
            Small,
            "light task: remind",
        ),
        // Hard words: Standard, or Large for code work with more to it.
        (
            msg(Chat, "Refactor the auth module"),
            Standard,
            "hard task: refactor",
        ),
        (
            worktree(msg(Chat, "Write code for a CSV parser")),
            Standard,
            "hard task: code",
        ),
        (
            worktree(msg(
                Chat,
                "Refactor the auth module and debug the login flow",
            )),
            Large,
            "code work: refactor, debug",
        ),
        (
            worktree(msg(Chat, &long("Implement this:", 140))),
            Large,
            "code work: implement",
        ),
        (
            msg(Chat, &long("Design a schema for:", 520)),
            Large,
            "long, hard task: design",
        ),
        (
            msg(Webhook, "Investigate the failing deploy"),
            Standard,
            "hard task: investigate",
        ),
        // A hard word beats a light one.
        (
            msg(Chat, "Check the logs and fix the bug"),
            Standard,
            "hard task: bug",
        ),
        // Background checks lean small.
        (
            instructions(Schedule, "Check my inbox and list new mail"),
            Small,
            "light task: check",
        ),
        (
            read_only(instructions(Schedule, "Tell me the news in my feed")),
            Small,
            "background check",
        ),
        (
            read_only(instructions(Watcher, "Tell me what arrived")),
            Small,
            "background check",
        ),
        (
            instructions(Schedule, "Tell me the news in my feed"),
            Standard,
            "can edit files or run commands",
        ),
        // Attachments and length.
        (
            files(msg(Chat, "Take a look"), 1),
            Standard,
            "has attachments",
        ),
        (
            files(msg(Chat, "Take a look"), 3),
            Standard,
            "many attachments",
        ),
        (msg(Chat, &long("summarise", 140)), Standard, "long message"),
        (msg(Chat, &long("Thoughts?", 520)), Standard, "long task"),
        // A quick question.
        (
            read_only(msg(Chat, "What do you think about tomorrow?")),
            Small,
            "short question",
        ),
        (
            msg(Chat, "What do you think about tomorrow?"),
            Standard,
            "can edit files or run commands",
        ),
        (
            read_only(instructions(Manual, "Tell me the news")),
            Standard,
            "general task",
        ),
        // The person names a model.
        (
            msg(Chat, "use opus to fix the typo"),
            Large,
            "you asked for Opus",
        ),
        (
            worktree(msg(Chat, "Use Haiku: refactor and debug the parser")),
            Small,
            "you asked for Haiku",
        ),
        (
            read_only(msg(Chat, "please use sonnet")),
            Standard,
            "you asked for Sonnet",
        ),
        // Only a message names a model, not the dot's instructions.
        (
            read_only(instructions(Schedule, "Never use opus. Tell me the news")),
            Small,
            "background check",
        ),
        // Whole words only.
        (
            read_only(msg(Chat, "Name a planet")),
            Small,
            "short question",
        ),
    ];
    for (signals, tier, reason) in cases {
        let r = route(&signals);
        assert_eq!(
            (r.tier, r.reason.as_str()),
            (tier, reason),
            "{:?}",
            signals.text
        );
    }
}

#[test]
fn the_same_run_always_gets_the_same_pick() {
    let s = worktree(msg(Chat, "Refactor and debug the scheduler"));
    assert_eq!(route(&s), route(&s));
}

#[test]
fn a_thread_keeps_its_tier() {
    let r = route(&in_thread(msg(Chat, "thanks!"), Large));
    assert_eq!(
        (r.tier, r.reason.as_str()),
        (Large, "kept from earlier in this conversation")
    );
    let r = route(&in_thread(read_only(msg(Chat, "ok")), Standard));
    assert_eq!(r.tier, Standard);
    // A long message is no reason to move up; hard work is.
    let r = route(&in_thread(msg(Chat, &"words ".repeat(400)), Small));
    assert_eq!(r.tier, Small);
}

#[test]
fn a_thread_moves_up_for_hard_work_and_never_down_on_its_own() {
    let r = route(&in_thread(msg(Chat, "now refactor the module"), Small));
    assert_eq!(
        (r.tier, r.reason.as_str()),
        (Standard, "escalated: hard task: refactor")
    );
    let r = route(&in_thread(
        worktree(msg(Chat, "Refactor and debug the parser")),
        Standard,
    ));
    assert_eq!(r.tier, Large);
    let r = route(&in_thread(msg(Chat, "summarise that"), Large));
    assert_eq!(r.tier, Large);
}

#[test]
fn the_person_overrides_a_thread_and_a_resume_keeps_it() {
    let r = route(&in_thread(msg(Chat, "use haiku for this one"), Large));
    assert_eq!(r.tier, Small);
    let r = route(&in_thread(msg(Resume, "approved"), Large));
    assert_eq!(
        (r.tier, r.reason.as_str()),
        (Large, "continues the run it resumes")
    );
    // A resume of a run Auto did not pick is picked afresh.
    assert_eq!(route(&read_only(msg(Resume, "hi"))).tier, Small);
}

#[test]
fn signals_are_read_from_the_dot_and_the_run() {
    let mut dot = fake_dot(
        "d",
        std::path::Path::new("C:/tmp/repo"),
        WorkspaceMode::Worktree,
    );
    dot.spec.instructions = "Check the inbox.".into();
    let chat = fake_run(
        Chat,
        Some(json!({"message": "Refactor this", "attachments": [{"name": "a.txt", "size": 1}]})),
        None,
    );
    let s = signals(&dot, &chat, Some(Small));
    assert_eq!(s.text, "Refactor this");
    assert!(s.is_message && s.can_act && s.worktree);
    assert_eq!((s.attachments, s.thread), (1, Some(Small)));
    // No message: the dot's instructions are the task.
    let scheduled = fake_run(Schedule, None, None);
    let s = signals(&dot, &scheduled, None);
    assert_eq!(s.text, "Check the inbox.");
    assert!(!s.is_message);
    // A workflow step's section is its message.
    let step = fake_run(
        TriggerKind::Workflow,
        Some(json!({"workflow": {"section": "## Workflow\n\nDesign the API"}})),
        None,
    );
    assert!(signals(&dot, &step, None).text.contains("Design the API"));
    // A read-only dot cannot act.
    dot.spec.policy = Policy::preset(Preset::ReadOnly);
    assert!(!signals(&dot, &scheduled, None).can_act);
    dot.spec.policy = Policy::preset(Preset::Trusted);
    assert!(signals(&dot, &scheduled, None).can_act);
}

#[test]
fn choose_maps_the_tier_with_settings_and_leaves_other_dots_alone() {
    let mut dot = fake_dot(
        "d",
        std::path::Path::new("C:/tmp/repo"),
        WorkspaceMode::Folder,
    );
    let run = fake_run(Chat, Some(json!({"message": "Summarise the notes"})), None);
    let mut settings = Settings::default();
    // A dot with its own model keeps it.
    dot.spec.model = "opus".into();
    let c = choose(&dot, &run, None, &settings);
    assert_eq!((c.model.as_str(), c.auto), ("opus", None));
    // Auto: the small tier's model, as Settings name it.
    dot.spec.model = "Auto".into();
    let c = choose(&dot, &run, None, &settings);
    assert_eq!(c.model, "haiku");
    assert_eq!(c.auto.unwrap().tier, Small);
    settings.auto_models.small = "claude-haiku-4-5".into();
    assert_eq!(
        choose(&dot, &run, None, &settings).model,
        "claude-haiku-4-5"
    );
    // A local dot's model is never read as Auto.
    dot.spec.engine = EngineKind::Local;
    assert!(choose(&dot, &run, None, &settings).auto.is_none());
}

#[test]
fn the_tier_models_default_to_haiku_sonnet_and_opus_and_are_checked() {
    let models = Settings::default().auto_models;
    assert_eq!(models, AutoModels::default());
    assert_eq!(
        [
            models.model(Small),
            models.model(Standard),
            models.model(Large)
        ],
        ["haiku", "sonnet", "opus"]
    );
    for bad in ["", "  ", "auto", "AUTO", "two words"] {
        let mut s = Settings::default();
        s.auto_models.large = bad.into();
        assert!(s.validate().is_err(), "{bad:?}");
    }
    // Auto may be the default model of new dots.
    let s = Settings {
        default_model: AUTO_MODEL.into(),
        ..Settings::default()
    };
    s.validate().unwrap();
    // Settings stored before the field existed read the defaults.
    let old: Settings = serde_json::from_value(json!({"port": 47321})).unwrap();
    assert_eq!(old.auto_models, AutoModels::default());
}

#[test]
fn auto_is_for_claude_dots_only() {
    let mut spec = common::spec("d");
    spec.model = "auto".into();
    spec.validate().unwrap();
    spec.engine = EngineKind::Local;
    spec.endpoint_url = Some("http://127.0.0.1:1234".into());
    let err = spec.validate().unwrap_err().to_string();
    assert!(err.contains("Auto is only for Claude dots"), "{err}");
    assert!(is_auto(" AUTO ") && !is_auto("autopilot"));
}

#[test]
fn the_fallback_goes_one_tier_down_or_to_standard() {
    assert_eq!(fallback_tier(Large), Standard);
    assert_eq!(fallback_tier(Standard), Small);
    assert_eq!(fallback_tier(Small), Standard);
}

#[test]
fn model_errors_are_told_from_other_failures() {
    for e in [
        "claude exited with code 1: There's an issue with the selected model (claude-opus-9). \
         It may not exist or you may not have access to it. Run /model to pick a different model.",
        r#"error_during_execution: API Error: 529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        r#"API Error: 404 {"type":"error","error":{"type":"not_found_error","message":"model: claude-x"}}"#,
        "Claude Opus is not available with the Claude Pro plan",
        "Invalid model name: opux",
        "the model is not supported for your account",
    ] {
        assert!(model_unavailable(e), "{e}");
    }
    for e in [
        "timeout after 1800s",
        "claude exited with code 1 without a result",
        "error_max_turns",
        "Claude AI usage limit reached|1700000000",
        r#"API Error: 429 {"type":"error","error":{"type":"rate_limit_error","message":"model overloaded"}}"#,
        "File not found: src/model.rs",
        "engine exited without a result",
    ] {
        assert!(!model_unavailable(e), "{e}");
    }
}

#[tokio::test]
async fn auto_is_listed_first_for_claude_and_not_for_a_local_endpoint() {
    let models = list_models(EngineKind::Claude, None).await.unwrap();
    assert_eq!(models[0].id, AUTO_MODEL);
    assert_eq!(models[0].label, "Auto");
    assert!(!models[0].hint.is_empty());
    let err = list_models(EngineKind::Local, None).await.unwrap_err();
    assert!(err.to_string().contains("endpoint"));
}

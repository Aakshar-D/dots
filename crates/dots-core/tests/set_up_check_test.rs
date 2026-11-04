//! The setup agent's prompt, the check of its answer, and the extras a proposal may add to its
//! preset (setup agent spec §6, §6.1, §6.2, §6.3).

use dots_core::library::Uses;
use dots_core::policy::{Action, Policy, Preset};
use dots_core::setup::{
    check_proposal, check_setup_answer, must_propose, setup_policy, setup_prompt, SetupAnswer,
    SetupContext, SetupLibrary, SetupProposal, SetupRequest, SetupRound, SETUP_INSTRUCTIONS,
};
use dots_core::Error;
use serde_json::{json, Value};

const UNREADABLE: &str = "The setup agent's answer could not be read. Try again.";
const FAILED: &str = "Setup could not finish. Try describing the task again, or fill in the form.";
const DESCRIPTION: &str = r"Résumé der Dateien in C:\Users\pat\My Files";

fn ctx() -> SetupContext {
    SetupContext {
        today: "2026-11-03 (Tuesday)".into(),
        utc_offset: "+01:00".into(),
        home: r"C:\Users\pat".into(),
        new_dot_root: r"C:\Users\pat\Dots".into(),
        library: SetupLibrary {
            skills: vec![("pdf-tools".into(), "Reads and splits PDF files.".into())],
            mcp_servers: vec!["gmail".into()],
            plugins: vec![],
            context: vec![],
        },
        dot_names: vec!["Inbox helper".into()],
    }
}

fn req() -> SetupRequest {
    SetupRequest {
        description: DESCRIPTION.into(),
        rounds: vec![],
        current: None,
        change: None,
        skip: false,
    }
}

fn round(n: usize) -> SetupRound {
    SetupRound {
        questions: vec![format!("Question {n}?")],
        answers: vec![format!("Answer {n}.")],
    }
}

fn proposal() -> SetupProposal {
    SetupProposal {
        name: "Downloads digest".into(),
        instructions: "1. List the files.".into(),
        folder: None,
        schedule: None,
        preset: Preset::ReadOnly,
        extras: vec![],
        uses: Uses::default(),
        first_message: "Run it now.".into(),
        notes: String::new(),
    }
}

fn proposal_json(fields: Value) -> String {
    let mut p = json!({
        "name": "Downloads digest",
        "instructions": "1. List the files.",
        "folder": null,
        "schedule": null,
        "preset": "read-only",
        "first_message": "Run it now.",
        "notes": ""
    });
    for (k, v) in fields.as_object().unwrap() {
        p[k] = v.clone();
    }
    json!({ "proposal": p }).to_string()
}

fn checked(fields: Value) -> SetupProposal {
    match check_setup_answer(&proposal_json(fields), &ctx(), &req()).unwrap() {
        SetupAnswer::Proposal {
            proposal,
            new_dot_root,
        } => {
            assert_eq!(new_dot_root, r"C:\Users\pat\Dots");
            proposal
        }
        other => panic!("expected a proposal, got {other:?}"),
    }
}

fn other_text(e: Error) -> String {
    match e {
        Error::Other(m) => m,
        e => panic!("expected Other, got {e:?}"),
    }
}

#[test]
fn prompt_carries_context_and_rounds() {
    for n in 0..2 {
        let mut r = req();
        r.rounds = (1..=n).map(round).collect();
        let prompt = setup_prompt(&ctx(), &r).unwrap();
        for part in [
            "2026-11-03 (Tuesday)",
            "+01:00",
            r"C:\Users\pat",
            "pdf-tools",
            "Reads and splits PDF files.",
            "gmail",
            "Inbox helper",
            DESCRIPTION,
        ] {
            assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
        }
        for i in 1..=n {
            assert!(prompt.contains(&format!("Question {i}?")), "{prompt}");
            assert!(prompt.contains(&format!("Answer {i}.")), "{prompt}");
        }
        assert!(!prompt.contains("Answer with a proposal now"), "{prompt}");
        assert!(!must_propose(&r));
    }
}

#[test]
fn prompt_leaves_out_empty_library_sections() {
    let prompt = setup_prompt(&ctx(), &req()).unwrap();
    assert!(!prompt.contains("Plugins"), "{prompt}");
    assert!(!prompt.contains("Context collections"), "{prompt}");
}

#[test]
fn prompt_must_propose() {
    let mut two = req();
    two.rounds = vec![round(1), round(2)];
    let mut skip = req();
    skip.skip = true;
    let mut change = req();
    change.current = Some(proposal());
    change.change = Some("Run it at 10 instead.".into());
    for r in [&two, &skip, &change] {
        assert!(must_propose(r));
        let prompt = setup_prompt(&ctx(), r).unwrap();
        assert!(
            prompt.contains("Answer with a proposal now. Do not ask questions."),
            "{prompt}"
        );
    }
    let prompt = setup_prompt(&ctx(), &change).unwrap();
    assert!(prompt.contains("Run it at 10 instead."), "{prompt}");
    let current = serde_json::to_string_pretty(&proposal()).unwrap();
    assert!(prompt.contains(&current), "{prompt}");
}

#[test]
fn prompt_rejects() {
    let invalid = |r: &SetupRequest| {
        assert!(
            matches!(setup_prompt(&ctx(), r), Err(Error::Invalid(_))),
            "{r:?}"
        )
    };
    let mut empty = req();
    empty.description = "   ".into();
    invalid(&empty);
    let mut long = req();
    long.description = "x".repeat(4001);
    invalid(&long);
    let mut ok = req();
    ok.description = "x".repeat(4000);
    assert!(setup_prompt(&ctx(), &ok).is_ok());
    let mut long_change = req();
    long_change.current = Some(proposal());
    long_change.change = Some("x".repeat(4001));
    invalid(&long_change);
    let mut orphan = req();
    orphan.change = Some("Run it at 10.".into());
    invalid(&orphan);
}

#[test]
fn answer_questions() {
    let plain = r#"{"questions":["Which folder?"]}"#;
    let fenced = "```json\n{\"questions\":[\"Which folder?\"]}\n```";
    for text in [plain, fenced] {
        assert_eq!(
            check_setup_answer(text, &ctx(), &req()).unwrap(),
            SetupAnswer::Questions {
                questions: vec!["Which folder?".into()]
            }
        );
    }
    let bad = [
        json!({ "questions": [] }),
        json!({ "questions": ["a?", "b?", "c?", "d?"] }),
        json!({ "questions": ["  "] }),
        json!({ "questions": ["x".repeat(301)] }),
        json!({ "questions": [3] }),
    ];
    for answer in bad {
        let e = check_setup_answer(&answer.to_string(), &ctx(), &req()).unwrap_err();
        assert_eq!(other_text(e), UNREADABLE, "{answer}");
    }
    let three = json!({ "questions": ["a?", "b?", "x".repeat(300)] }).to_string();
    assert!(matches!(
        check_setup_answer(&three, &ctx(), &req()).unwrap(),
        SetupAnswer::Questions { .. }
    ));
}

#[test]
fn answer_unreadable() {
    let both = json!({ "questions": ["a?"], "proposal": {} }).to_string();
    let neither = json!({ "answer": "hi" }).to_string();
    for text in [
        "Here is my proposal.".to_string(),
        r#"Sure! {"questions":["Which folder?"]}"#.to_string(),
        both,
        neither,
        "[1, 2]".to_string(),
        json!({ "proposal": { "name": "x" } }).to_string(),
    ] {
        let e = check_setup_answer(&text, &ctx(), &req()).unwrap_err();
        assert_eq!(other_text(e), UNREADABLE, "{text}");
    }
}

#[test]
fn questions_when_must_propose() {
    let mut skip = req();
    skip.skip = true;
    let e = check_setup_answer(r#"{"questions":["Which folder?"]}"#, &ctx(), &skip).unwrap_err();
    assert_eq!(other_text(e), FAILED);
}

#[test]
fn proposal_fixups() {
    // A valid proposal comes back as it was.
    let p = checked(json!({}));
    assert_eq!(p.name, "Downloads digest");
    assert_eq!(p.preset, Preset::ReadOnly);
    assert_eq!(p.notes, "");
    assert_eq!(p.first_message, "Run it now.");

    // An unknown or custom preset becomes sandboxed, with a note.
    for preset in ["admin", "custom"] {
        let p = checked(json!({ "preset": preset }));
        assert_eq!(p.preset, Preset::Sandboxed);
        assert!(!p.notes.is_empty(), "{preset}");
    }

    // Library items that are not there are dropped, each named in notes.
    let p = checked(json!({
        "uses": { "skills": ["pdf-tools", "nope"], "mcp_servers": ["gmail"] }
    }));
    assert_eq!(p.uses.skills, vec!["pdf-tools".to_string()]);
    assert_eq!(p.uses.mcp_servers, vec!["gmail".to_string()]);
    assert!(p.notes.contains("nope"), "{}", p.notes);

    // Extras: `mcp:` needs its server in uses; unknown ones are dropped; duplicates removed.
    let p = checked(json!({
        "uses": { "mcp_servers": ["gmail"] },
        "extras": ["web", "mcp:slack", "mcp:gmail", "root", "web"]
    }));
    assert_eq!(p.extras, vec!["web".to_string(), "mcp:gmail".to_string()]);
    assert!(p.notes.contains("mcp:slack"), "{}", p.notes);
    assert!(p.notes.contains("root"), "{}", p.notes);
    let p = checked(json!({ "extras": ["mcp:gmail"] }));
    assert!(p.extras.is_empty());

    // A schedule that fails validation, or does not parse, is dropped with a note.
    for schedule in [
        json!({ "rule": { "kind": "every", "minutes": 0 } }),
        json!({ "rule": { "kind": "daily", "time": "9am" } }),
    ] {
        let p = checked(json!({ "schedule": schedule }));
        assert_eq!(p.schedule, None);
        assert!(!p.notes.is_empty());
    }
    let p = checked(json!({
        "schedule": { "rule": { "kind": "weekly", "days": ["mon", "fri"], "time": "09:00" } }
    }));
    assert!(p.schedule.is_some());
    assert_eq!(p.notes, "");

    // Instructions over 4000 characters are cut, with a note.
    let p = checked(json!({ "instructions": "é".repeat(5000) }));
    assert_eq!(p.instructions.chars().count(), 4000);
    assert!(!p.notes.is_empty());

    // A folder that does not exist becomes the default, with a note; one that does stays.
    let p = checked(json!({ "folder": r"C:\does\not\exist" }));
    assert_eq!(p.folder, None);
    assert!(!p.notes.is_empty());
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().to_string_lossy().to_string();
    let p = checked(json!({ "folder": real }));
    assert_eq!(p.folder.as_deref(), Some(real.as_str()));

    // A blank first message becomes the description.
    let p = checked(json!({ "first_message": "  " }));
    assert_eq!(p.first_message, DESCRIPTION);
    let mut no_first = serde_json::from_str::<Value>(&proposal_json(json!({}))).unwrap();
    no_first["proposal"]
        .as_object_mut()
        .unwrap()
        .remove("first_message");
    match check_setup_answer(&no_first.to_string(), &ctx(), &req()).unwrap() {
        SetupAnswer::Proposal { proposal, .. } => assert_eq!(proposal.first_message, DESCRIPTION),
        other => panic!("{other:?}"),
    }

    // The agent's own notes are kept, and the core's are added after them.
    let p = checked(json!({ "notes": "I assumed 9:00.", "preset": "admin" }));
    assert!(p.notes.starts_with("I assumed 9:00.\n"), "{}", p.notes);
}

#[test]
fn proposal_name_clash() {
    let p = checked(json!({ "name": "inbox helper" }));
    assert_eq!(p.name, "inbox helper 2");
    let mut c = ctx();
    c.dot_names.push("Inbox Helper 2".into());
    let mut clash = proposal();
    clash.name = "inbox helper".into();
    assert_eq!(
        check_proposal(clash.clone(), &c, true).unwrap().name,
        "inbox helper 3"
    );
    assert!(matches!(
        check_proposal(clash, &ctx(), false),
        Err(Error::Conflict(_))
    ));
    // A long name keeps 64 characters with its number.
    let mut c = ctx();
    let long = "a".repeat(64);
    c.dot_names.push(long.clone());
    let mut p = proposal();
    p.name = long;
    let renamed = check_proposal(p, &c, true).unwrap().name;
    assert_eq!(renamed, format!("{} 2", "a".repeat(62)));
}

#[test]
fn proposal_name_repair() {
    let mut p = proposal();
    p.name = "Résumé  bot!".into();
    let p = check_proposal(p, &ctx(), true).unwrap();
    assert_eq!(p.name, "Rsum bot");
    assert!(!p.notes.is_empty());

    let mut p = proposal();
    p.name = "!!!".into();
    assert_eq!(check_proposal(p, &ctx(), true).unwrap().name, "New dot");

    let mut p = proposal();
    p.name = format!("{} b", "a".repeat(63));
    assert_eq!(
        check_proposal(p, &ctx(), true).unwrap().name,
        "a".repeat(63)
    );

    let p = check_proposal(proposal(), &ctx(), true).unwrap();
    assert_eq!(p.name, "Downloads digest");
    assert_eq!(p.notes, "");
}

#[test]
fn proposal_blank_instructions_are_unreadable() {
    let e = check_setup_answer(
        &proposal_json(json!({ "instructions": " " })),
        &ctx(),
        &req(),
    )
    .unwrap_err();
    assert_eq!(other_text(e), UNREADABLE);
}

#[test]
fn policy_extras() {
    assert_eq!(
        setup_policy(Preset::ReadOnly, &[]),
        Policy::preset(Preset::ReadOnly)
    );
    let web = setup_policy(Preset::ReadOnly, &["web".to_string()]);
    assert_eq!(web.preset, Preset::Custom);
    assert_eq!(web.resolve("WebFetch", &json!({})), Action::Allow);
    assert_eq!(web.resolve("WebSearch", &json!({})), Action::Allow);
    assert_eq!(
        web.resolve("Bash", &json!({ "command": "ls" })),
        Action::Deny
    );
    let mcp = setup_policy(Preset::ReadOnly, &["mcp:gmail".to_string()]);
    assert_eq!(mcp.preset, Preset::Custom);
    assert_eq!(mcp.resolve("mcp__gmail__send", &json!({})), Action::Allow);
    assert_ne!(mcp.resolve("mcp__slack__send", &json!({})), Action::Allow);
}

#[test]
fn instructions_name_the_rules() {
    for part in [
        "May read files. Never writes, runs commands, reads websites or uses MCP servers.",
        "May edit files in its folder and run a few safe git commands. Asks before anything else.",
        "May run commands without asking.",
        "\"web\"",
        "\"mcp:<server>\"",
        "\"questions\"",
        "\"proposal\"",
        "\"kind\": \"weekly\"",
        "4000",
    ] {
        assert!(SETUP_INSTRUCTIONS.contains(part), "missing {part:?}");
    }
}

#[test]
fn prompt_says_what_each_preset_lets_the_dot_do() {
    for part in [
        // A read-only dot's tools, and what it cannot do with them.
        "A \"read-only\" dot can only use Read, Glob and Grep",
        "It cannot run commands, so it cannot see file dates or sizes",
        "Glob lists files newest first",
        // Instructions keep to the preset.
        "Write instructions that use only what the chosen preset allows",
        // A task that needs more: sandboxed with a note, or a note of what it cannot do.
        "When the task needs file dates, sizes or commands, pick \"sandboxed\"",
        "say in \"notes\" that each command will ask for approval",
        "say in \"notes\" what the dot cannot do",
    ] {
        assert!(SETUP_INSTRUCTIONS.contains(part), "missing {part:?}");
    }
}

#[test]
fn proposal_folder_must_be_a_full_local_path() {
    for folder in [
        ".",
        r"..\x",
        r"\host\share",
        r"\?\C:\Windows",
        "//host/share",
    ] {
        let mut p = proposal();
        p.folder = Some(folder.into());
        let p = check_proposal(p, &ctx(), true).unwrap();
        assert_eq!(p.folder, None, "{folder}");
        assert!(p.notes.contains(folder), "{folder}: {}", p.notes);
    }
    let mut p = proposal();
    p.folder = Some("  ".into());
    let p = check_proposal(p, &ctx(), true).unwrap();
    assert_eq!(p.folder, None);
    assert_eq!(p.notes, "");
}

#[test]
fn proposal_custom_preset_becomes_sandboxed() {
    let mut p = proposal();
    p.preset = Preset::Custom;
    let p = check_proposal(p, &ctx(), false).unwrap();
    assert_eq!(p.preset, Preset::Sandboxed);
    assert!(!p.notes.is_empty());
    let p = check_proposal(proposal(), &ctx(), false).unwrap();
    assert_eq!(p.preset, Preset::ReadOnly);
}

#[test]
fn extras_are_exact() {
    let mut c = ctx();
    c.library.mcp_servers.push("Bad Name".into());
    let mut p = proposal();
    p.uses.mcp_servers = vec!["gmail".into(), "Bad Name".into()];
    p.extras = vec![
        " web".into(),
        "web ".into(),
        "mcp:Bad Name".into(),
        "mcp: gmail".into(),
        "mcp:gmail".into(),
    ];
    let p = check_proposal(p, &c, true).unwrap();
    assert_eq!(p.extras, vec!["mcp:gmail".to_string()]);
    assert!(
        p.notes.contains("\" web\"") || p.notes.contains(" web"),
        "{}",
        p.notes
    );

    let policy = |extras: &[&str]| {
        let extras: Vec<String> = extras.iter().map(|e| e.to_string()).collect();
        setup_policy(Preset::ReadOnly, &extras)
    };
    for odd in [
        " web",
        "WEB",
        "mcp:",
        "mcp:Bad Name",
        "mcp:a__b",
        "mcp: gmail",
        "root",
    ] {
        assert_eq!(
            policy(&[odd]),
            Policy::preset(Preset::ReadOnly),
            "{odd:?} must add nothing"
        );
    }
}

#[test]
fn proposal_schedule_must_still_run() {
    let past = json!({ "rule": { "kind": "once", "at": "2020-01-01T09:00" } });
    let p = checked(json!({ "schedule": past }));
    assert_eq!(p.schedule, None);
    assert!(!p.notes.is_empty());
    let ended = json!({
        "rule": { "kind": "daily", "time": "09:00" },
        "until": { "date": "2020-01-01T08:00" }
    });
    let p = checked(json!({ "schedule": ended }));
    assert_eq!(p.schedule, None);
    let future = json!({ "rule": { "kind": "once", "at": "2099-01-01T09:00" } });
    let p = checked(json!({ "schedule": future }));
    assert!(p.schedule.is_some());
    assert_eq!(p.notes, "");
}

#[test]
fn prompt_rejects_bad_rounds() {
    let invalid = |r: &SetupRequest| {
        assert!(
            matches!(setup_prompt(&ctx(), r), Err(Error::Invalid(_))),
            "{r:?}"
        )
    };
    let mut three = req();
    three.rounds = vec![round(1), round(2), round(3)];
    invalid(&three);
    let mut extra_answer = req();
    let mut r = round(1);
    r.answers.push("Another answer.".into());
    extra_answer.rounds = vec![r];
    invalid(&extra_answer);
    let mut long_answer = req();
    let mut r = round(1);
    r.answers = vec!["x".repeat(4001)];
    long_answer.rounds = vec![r];
    invalid(&long_answer);
    // Fewer answers than questions is fine: a question may be left blank.
    let mut fewer = req();
    let mut r = round(1);
    r.questions.push("Another?".into());
    fewer.rounds = vec![r];
    assert!(setup_prompt(&ctx(), &fewer).is_ok());
}

//! The local engine's `skill` tool and the system prompt's skills section (MCP/skills spec
//! section 7): a skill's body with `${CLAUDE_SKILL_DIR}` replaced and its files listed, a file
//! of the skill read only inside its folder, results capped at 30 KB.

mod common;
mod llm;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::tools::{self, skill, MAX_OUTPUT};
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::kit;
use dots_core::library::{LibraryConfig, PluginConfig, SkillConfig};
use dots_core::library_service::LibraryService;
use dots_core::model::{EngineKind, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::{Policy, Preset};
use dots_core::run_tools::RunSkill;
use dots_core::runner::{Runner, RunnerConfig};
use llm::{call, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A skill folder `pdf` with a body that names its folder, two files and a `.git` folder.
fn pdf_skill(root: &Path) -> PathBuf {
    let dir = root.join("pdf");
    write(
        &dir.join("SKILL.md"),
        "---\nname: pdf\ndescription: Read PDFs.\nwhen_to_use: When a PDF is named.\n---\n\
         # PDF\n\nRun ${CLAUDE_SKILL_DIR}/scripts/x.py on the file.\n",
    );
    write(&dir.join("scripts/x.py"), "print('x')\n");
    write(&dir.join("ref.md"), "Reference.\n");
    write(&dir.join(".git/HEAD"), "ref: refs/heads/main\n");
    write(&root.join("outside.txt"), "secret\n");
    dir
}

fn skills(dir: &Path) -> Vec<RunSkill> {
    vec![RunSkill {
        name: "pdf".into(),
        info: kit::read_skill(dir).unwrap(),
        plugin: None,
    }]
}

async fn ask(skills: &[RunSkill], input: Value) -> tools::ToolOutput {
    skill::run(skills, &input).await
}

#[tokio::test]
async fn without_a_file_the_body_comes_with_the_skills_files() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    let out = ask(&skills(&dir), json!({"name": "pdf"})).await;
    assert!(!out.is_error, "{}", out.output);
    assert_eq!(
        out.output,
        format!(
            "# PDF\n\nRun {}/scripts/x.py on the file.\n\n\
             Files in this skill (read one with `file`):\n- ref.md\n- scripts/x.py",
            dir.display()
        )
    );
    // An empty `file` is no file.
    let blank = ask(&skills(&dir), json!({"name": "pdf", "file": " "})).await;
    assert_eq!(blank, out);
}

#[tokio::test]
async fn a_file_is_read_only_inside_the_skill_folder() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    let s = skills(&dir);
    let read = ask(&s, json!({"name": "pdf", "file": "scripts/x.py"})).await;
    assert_eq!(
        (read.output.as_str(), read.is_error),
        ("print('x')\n", false)
    );
    let raw = ask(&s, json!({"name": "pdf", "file": "SKILL.md"})).await;
    assert!(
        raw.output.contains("${CLAUDE_SKILL_DIR}"),
        "the file as it is"
    );

    let outside = root.path().join("outside.txt");
    for file in [
        "../outside.txt".to_string(),
        "scripts/../../outside.txt".to_string(),
        outside.to_string_lossy().into_owned(),
    ] {
        let out = ask(&s, json!({"name": "pdf", "file": file})).await;
        assert!(out.is_error, "{file}: {}", out.output);
        assert!(!out.output.contains("secret"), "{file}");
        assert!(
            out.output.contains("inside the skill folder"),
            "{file}: {}",
            out.output
        );
    }
    let missing = ask(&s, json!({"name": "pdf", "file": "nope.md"})).await;
    assert!(
        missing.is_error && missing.output.contains("nope.md"),
        "{}",
        missing.output
    );
    let folder = ask(&s, json!({"name": "pdf", "file": "scripts"})).await;
    assert!(
        folder.is_error && folder.output.contains("is a folder"),
        "{}",
        folder.output
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_out_of_the_skill_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    std::os::unix::fs::symlink(root.path().join("outside.txt"), dir.join("link.txt")).unwrap();
    let out = ask(&skills(&dir), json!({"name": "pdf", "file": "link.txt"})).await;
    assert!(
        out.is_error && !out.output.contains("secret"),
        "{}",
        out.output
    );
}

#[tokio::test]
async fn an_unknown_skill_names_the_skills_there_are() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    let out = ask(&skills(&dir), json!({"name": "word"})).await;
    assert!(out.is_error);
    assert_eq!(out.output, "unknown skill 'word'. Skills: pdf");
    let none = ask(&[], json!({"name": "word"})).await;
    assert_eq!(none.output, "unknown skill 'word'. This run has no skills.");
}

#[tokio::test]
async fn results_are_capped_at_30_kb() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    write(&dir.join("big.md"), &"z".repeat(40_000));
    let out = ask(&skills(&dir), json!({"name": "pdf", "file": "big.md"})).await;
    assert!(!out.is_error);
    assert!(out.output.len() < MAX_OUTPUT + 100, "{}", out.output.len());
    assert!(out.output.contains("[truncated:"));
}

#[tokio::test]
async fn the_prompt_section_lists_each_skill_on_one_line() {
    let root = tempfile::tempdir().unwrap();
    let dir = pdf_skill(root.path());
    assert_eq!(skill::prompt_section(&[]), "");
    let mut list = skills(&dir);
    list.push(RunSkill {
        name: "kit:fill".into(),
        plugin: Some("kit".into()),
        ..list[0].clone()
    });
    assert_eq!(
        skill::prompt_section(&list),
        "Skills you can use. Call the tool `skill` with a name to read one before you follow \
         it.\n- pdf: Read PDFs. When a PDF is named.\n- kit:fill: Read PDFs. When a PDF is named."
    );
}

// ---- In a run ----

#[tokio::test]
async fn a_local_run_lists_its_skills_and_reads_one_with_the_skill_tool() {
    let (dir, store) = temp_store().await;
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());

    let lib = LibraryService::new(store.clone(), bus);
    let skills_root: TempDir = tempfile::tempdir().unwrap();
    let pdf = pdf_skill(skills_root.path());
    let plugin = skills_root.path().join("kit");
    write(
        &plugin.join(".claude-plugin/plugin.json"),
        r#"{"name": "kit"}"#,
    );
    write(
        &plugin.join("skills/fill/SKILL.md"),
        "---\nname: fill\ndescription: Fill forms.\n---\nAsk for the form.\n",
    );
    let path = |p: &Path| p.to_string_lossy().into_owned();
    lib.add(
        "pdf",
        LibraryConfig::Skill(SkillConfig { path: path(&pdf) }),
        None,
    )
    .await
    .unwrap();
    lib.add(
        "kit",
        LibraryConfig::Plugin(PluginConfig {
            path: path(&plugin),
            installed_id: None,
            run_hooks: false,
        }),
        None,
    )
    .await
    .unwrap();

    let llm = Llm::start(vec![
        call("c1", "skill", json!({"name": "kit:fill"})),
        text("Read it."),
    ])
    .await;
    let mut s = spec("skilled");
    s.workdir = path(&ws);
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "test-model".into();
    // Every preset allows `Skill`, Read-only too.
    s.policy = Policy::preset(Preset::ReadOnly);
    s.uses.skills = vec!["pdf".into()];
    s.uses.plugins = vec!["kit".into()];
    let dot = store.create_dot(&s, None).await.unwrap();
    let r = runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    wait_status(&store, &r.id, RunStatus::Succeeded, 20).await;

    let reqs = llm.requests();
    let system = messages(&reqs[0])[0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        system.ends_with(
            "\n\nSkills you can use. Call the tool `skill` with a name to read one before you \
             follow it.\n- pdf: Read PDFs. When a PDF is named.\n- kit:fill: Fill forms."
        ),
        "{system}"
    );
    let names: Vec<&str> = reqs[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"skill"));
    assert_eq!(tool_result(&reqs[1], "c1"), "Ask for the form.");
    let shown = store
        .list_events(&r.id, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|ev| ev.kind == "tool_call")
        .unwrap();
    assert_eq!(shown.data["tool"], "Skill");
    assert_eq!(shown.data["input"], json!({"name": "kit:fill"}));
}

#[test]
fn skill_is_a_local_tool_with_the_policy_name_skill() {
    assert_eq!(tools::alias("skill"), Some("Skill"));
    assert_eq!(tools::local_name("Skill"), Some("skill"));
    let ws = tempfile::tempdir().unwrap();
    let prepared = tools::prepare(
        "skill",
        &json!({"name": "pdf", "file": "a.md", "extra": 1}),
        ws.path(),
    )
    .unwrap();
    assert_eq!(prepared.input, json!({"name": "pdf", "file": "a.md"}));
    let err = tools::prepare("skill", &json!({}), ws.path()).unwrap_err();
    assert!(err.contains("'name'"), "{err}");
    assert_eq!(
        skill::schema()["function"]["parameters"]["required"],
        json!(["name"])
    );
}

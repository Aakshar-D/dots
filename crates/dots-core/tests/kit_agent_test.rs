//! Claude Code agent files: reading one, and what its name, model and tools become in a dot.

use std::fs;
use std::path::Path;

use dots_core::kit::{agent_files, agent_model, agent_policy, dot_name, read_agent, AgentTools};
use dots_core::policy::{Action, Policy, Preset};
use serde_json::json;
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn tools(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn an_agent_file_gives_its_fields_and_prompt() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("reviewer.md");
    write(
        &path,
        "---\nname: code-reviewer\ndescription: Reviews code\ntools: Read, Grep\nmodel: opus\ncolor: red\n---\n\nYou review code.\n",
    );
    let agent = read_agent(&path).unwrap();
    assert_eq!(agent.name, "code-reviewer");
    assert_eq!(agent.description.as_deref(), Some("Reviews code"));
    assert_eq!(agent.tools, AgentTools::Listed(tools(&["Read", "Grep"])));
    assert_eq!(agent.model.as_deref(), Some("opus"));
    assert_eq!(agent.prompt, "You review code.");
}

#[test]
fn tools_may_be_a_yaml_list_and_the_name_falls_back_to_the_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("helper.md");
    write(
        &path,
        "---\ndescription: d\ntools:\n  - Read\n  - Bash\n---\nHelp.\n",
    );
    let agent = read_agent(&path).unwrap();
    assert_eq!(agent.name, "helper");
    assert_eq!(agent.tools, AgentTools::Listed(tools(&["Read", "Bash"])));
    assert_eq!(agent.model, None);
}

#[test]
fn a_file_without_a_prompt_or_frontmatter_or_too_large_is_refused() {
    let dir = tempdir().unwrap();
    let empty = dir.path().join("empty.md");
    write(&empty, "---\nname: x\n---\n   \n");
    assert!(read_agent(&empty).unwrap_err().contains("no instructions"));
    let plain = dir.path().join("plain.md");
    write(&plain, "# Just notes\n");
    assert!(read_agent(&plain).unwrap_err().contains("frontmatter"));
    let broken = dir.path().join("broken.md");
    write(&broken, "---\nname: x\nname: y\n---\nbody\n");
    assert!(read_agent(&broken).unwrap_err().contains("frontmatter"));
    let big = dir.path().join("big.md");
    write(
        &big,
        &format!("---\nname: big\n---\n{}", "a".repeat(300 * 1024)),
    );
    assert!(read_agent(&big).unwrap_err().contains("larger"));
}

#[test]
fn a_name_becomes_a_valid_dot_name() {
    assert_eq!(dot_name("code-reviewer"), "code-reviewer");
    assert_eq!(dot_name("Code Reviewer (v2)"), "Code-Reviewer-v2");
    assert_eq!(dot_name("  é!  "), "agent");
    assert_eq!(dot_name("a__b"), "a__b");
    assert_eq!(dot_name(&"x".repeat(80)).len(), 64);
}

#[test]
fn models_map_to_dots_ids() {
    assert_eq!(agent_model(Some("Sonnet")), Some("sonnet".into()));
    assert_eq!(agent_model(Some("opus")), Some("opus".into()));
    assert_eq!(agent_model(Some("haiku")), Some("haiku".into()));
    assert_eq!(agent_model(Some("inherit")), None);
    assert_eq!(agent_model(Some(" ")), None);
    assert_eq!(agent_model(None), None);
    assert_eq!(
        agent_model(Some("claude-opus-4-1-20250805")),
        Some("claude-opus-4-1-20250805".into())
    );
}

#[test]
fn no_tools_listed_gives_the_sandboxed_preset() {
    let (policy, warnings) = agent_policy(&AgentTools::All);
    assert_eq!(policy, Policy::preset(Preset::Sandboxed));
    assert!(warnings.is_empty());
}

#[test]
fn listed_tools_are_allowed_and_risky_ones_ask() {
    let listed = tools(&[
        "Read",
        "Grep",
        "WebSearch",
        "Bash",
        "Write",
        "Frobnicate",
        "mcp__gh__issue",
    ]);
    let (policy, warnings) = agent_policy(&AgentTools::Listed(listed));
    policy.validate().unwrap();
    assert_eq!(policy.preset, Preset::Custom);
    assert_eq!(policy.default, Action::Ask);
    let any = json!({});
    assert_eq!(
        policy.resolve("Read", &json!({"file_path": "a.txt"})),
        Action::Allow
    );
    assert_eq!(
        policy.resolve("WebSearch", &json!({"query": "x"})),
        Action::Allow
    );
    assert_eq!(
        policy.resolve("Bash", &json!({"command": "ls"})),
        Action::Ask
    );
    assert_eq!(
        policy.resolve("Write", &json!({"file_path": "a.txt"})),
        Action::Ask
    );
    // Not listed: the preset's own allows are gone.
    assert_eq!(
        policy.resolve("Glob", &json!({"pattern": "*"})),
        Action::Ask
    );
    assert_eq!(
        policy.resolve("Edit", &json!({"file_path": "a.txt"})),
        Action::Ask
    );
    assert_eq!(
        policy.resolve("Bash", &json!({"command": "git status"})),
        Action::Ask
    );
    // The preset's deny rules and Dots' own tools stay.
    assert_eq!(
        policy.resolve("Write", &json!({"file_path": ".git/config"})),
        Action::Deny
    );
    assert_eq!(policy.resolve("Notify", &any), Action::Allow);
    assert!(warnings.iter().any(|w| w.contains("Frobnicate")));
    assert!(warnings.iter().any(|w| w.contains("mcp__gh__issue")));
    assert!(warnings.iter().any(|w| w.contains("Bash")));
    assert!(policy.rules.iter().all(|r| r.tool != "Frobnicate"));
}

#[test]
fn agent_files_are_found_in_nested_folders() {
    let dir = tempdir().unwrap();
    write(&dir.path().join("a.md"), "x");
    write(&dir.path().join("team/b.md"), "x");
    write(&dir.path().join("notes.txt"), "x");
    let found: Vec<String> = agent_files(dir.path())
        .iter()
        .map(|p| {
            p.strip_prefix(dir.path())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(found, ["a.md", "team/b.md"]);
    assert!(agent_files(&dir.path().join("missing")).is_empty());
}

/// Every allow rule of `policy` is one of Dots' own tools: no file, command or web tool runs
/// without asking.
fn allows_only_dots_tools(policy: &Policy) {
    let file_tools = [
        "Read", "LS", "Glob", "Grep", "Write", "Edit", "Bash", "WebFetch",
    ];
    for rule in policy.rules.iter().filter(|r| r.action == Action::Allow) {
        assert!(
            !file_tools
                .iter()
                .any(|t| rule.tool == *t || rule.tool.starts_with(&format!("{t}("))),
            "{rule:?}"
        );
    }
    assert_eq!(policy.default, Action::Ask);
    assert_eq!(
        policy.resolve("Write", &json!({"file_path": "a.txt"})),
        Action::Ask
    );
    assert_eq!(
        policy.resolve("Bash", &json!({"command": "git status"})),
        Action::Ask
    );
    assert_eq!(
        policy.resolve("Read", &json!({"file_path": "a.txt"})),
        Action::Ask
    );
}

#[test]
fn tools_dots_cannot_read_give_the_most_careful_policy() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("odd.md");
    write(&path, "---\nname: odd\ntools: [Read, [Bash]]\n---\nDo.\n");
    let agent = read_agent(&path).unwrap();
    assert_eq!(agent.tools, AgentTools::Unreadable);
    let (policy, warnings) = agent_policy(&agent.tools);
    policy.validate().unwrap();
    allows_only_dots_tools(&policy);
    assert!(
        warnings.iter().any(|w| w.contains("could not be read")),
        "{warnings:?}"
    );
}

#[test]
fn an_empty_tools_key_allows_no_tool() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("bare.md");
    write(&path, "---\nname: bare\ntools:\n---\nDo.\n");
    let agent = read_agent(&path).unwrap();
    assert_eq!(agent.tools, AgentTools::Listed(Vec::new()));
    allows_only_dots_tools(&agent_policy(&agent.tools).0);
}

#[test]
fn an_empty_frontmatter_takes_the_file_name() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("plain-helper.md");
    write(&path, "---\n---\nHelp out.\n");
    let agent = read_agent(&path).unwrap();
    assert_eq!(agent.name, "plain-helper");
    assert_eq!(agent.tools, AgentTools::All);
}

#[test]
fn windows_reserved_names_get_a_suffix() {
    for raw in [
        "con", "PRN", "Aux", "nul", "com1", "COM9", "lpt1", "LPT9", "con.txt", "nul.md",
    ] {
        let name = dot_name(raw);
        assert!(name.ends_with("-agent"), "{raw} -> {name}");
    }
    assert_eq!(dot_name("console"), "console");
    assert_eq!(dot_name("com10"), "com10");
}

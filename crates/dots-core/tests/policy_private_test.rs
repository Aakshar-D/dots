//! The private-folder guard (approval guards spec section 3).

use std::path::Path;

use dots_core::policy::{pattern_touches_private, register_vault, touches_private};
use serde_json::{json, Value};

fn read(path: &str) -> Value {
    json!({ "file_path": path })
}

fn bash(cmd: &str) -> Value {
    json!({ "command": cmd })
}

fn home() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .expect("a home folder")
}

#[test]
fn every_listed_place_is_private_for_a_file_tool() {
    for path in [
        "~/.ssh/id_rsa",
        "~/.aws/credentials",
        "~/.gnupg/secring.gpg",
        "~/.kube/config",
        "~/.docker/config.json",
        "~/.config/gh/hosts.yml",
        "~/.azure/accessTokens.json",
        "~/.claude/.credentials.json",
        "~/.netrc",
        "~/.npmrc",
        "~/.pypirc",
        "~/.git-credentials",
        ".env",
        "app/.env.local",
        "C:/Users/me/AppData/Roaming/Microsoft/Credentials/ABC",
        "C:/Users/me/AppData/Roaming/Microsoft/Protect/S-1-5/key",
        "~/.ssh",
    ] {
        assert!(touches_private("Read", &read(path), None), "{path}");
    }
}

#[test]
fn matching_ignores_case_slashes_and_trailing_dots() {
    for path in [
        r"C:\Users\Me\.SSH\id_rsa",
        r"C:\Users\me\.ssh.\id_rsa",
        "/home/me/.Aws/credentials",
        r"..\..\.ssh\config",
        r"%USERPROFILE%\.aws\config",
        "$HOME/.kube/config",
        r"C:\Users\me\appdata\roaming\microsoft\credentials\x",
    ] {
        assert!(touches_private("Read", &read(path), None), "{path}");
    }
}

#[test]
fn ordinary_files_are_not_private() {
    for path in [
        "src/main.rs",
        ".envrc",
        "environment.ts",
        "docs/ssh.md",
        ".config/other/x",
        "C:/Users/me/notes.txt",
        "Claude/.credentials",
    ] {
        assert!(!touches_private("Read", &read(path), None), "{path}");
    }
}

#[test]
fn every_file_tool_and_its_path_fields_count() {
    assert!(touches_private("Write", &read("~/.ssh/authorized_keys"), None));
    assert!(touches_private("Edit", &read(".env"), None));
    assert!(touches_private(
        "NotebookEdit",
        &json!({ "notebook_path": ".aws/x.ipynb" }),
        None
    ));
    assert!(touches_private("LS", &json!({ "path": "~/.ssh" }), None));
    assert!(touches_private("Glob", &json!({ "pattern": "**/.env*" }), None));
    assert!(touches_private("Glob", &json!({ "pattern": "~/.ss?/*" }), None));
    assert!(touches_private(
        "Grep",
        &json!({ "pattern": "x", "glob": ".env.*" }),
        None
    ));
    // A search pattern is a regular expression, not a path.
    assert!(!touches_private("Grep", &json!({ "pattern": ".ssh" }), None));
    // A wildcard that does not start with a dot skips hidden files.
    assert!(!touches_private("Glob", &json!({ "pattern": "**/*" }), None));
    assert!(!touches_private("Grep", &json!({ "pattern": "x", "glob": "*.rs" }), None));
}

#[test]
fn shell_commands_that_mention_a_private_place_are_private() {
    for cmd in [
        "cat ~/.ssh/id_rsa",
        "type %USERPROFILE%\\.aws\\credentials",
        "Get-Content $env:USERPROFILE\\.npmrc",
        "cat .env",
        "cp .env.production /tmp/x",
        "cat ~/.ss''h/id_rsa",
        "gc ~/.ss`h/id_rsa",
        "gh auth token > x; cat ~/.config/gh/hosts.yml",
        "dir %APPDATA%\\Microsoft\\Credentials",
    ] {
        assert!(touches_private("Bash", &bash(cmd), None), "{cmd}");
        assert!(touches_private("PowerShell", &bash(cmd), None), "{cmd}");
    }
    for cmd in [
        "node -e \"console.log(process.env.HOME)\"",
        "git status",
        "cargo test",
        "echo $env:PATH",
        "direnv allow .envrc",
    ] {
        assert!(!touches_private("Bash", &bash(cmd), None), "{cmd}");
    }
}

#[test]
fn other_tools_are_not_judged() {
    assert!(!touches_private(
        "WebFetch",
        &json!({ "url": "https://x.test/.env" }),
        None
    ));
    assert!(!touches_private("TodoWrite", &json!({ "todos": [".env"] }), None));
}

#[test]
fn the_vault_is_private_by_its_absolute_path() {
    let data = std::env::temp_dir().join("dots-private-test-data");
    register_vault(&data.join("vault"));
    let vault = data.join("vault").join("org").join("copy.age");
    let vault = vault.to_string_lossy();
    assert!(touches_private("Read", &read(&vault), None));
    let upper = vault.to_uppercase();
    assert!(touches_private("Read", &read(&upper), None));
    let through = data.join("worktrees").join("..").join("vault").join("x");
    assert!(touches_private("Read", &read(&through.to_string_lossy()), None));
    let cmd = format!("cat \"{}\"", data.join("vault").join("x").display());
    assert!(touches_private("Bash", &bash(&cmd), None));
    // The data folder's other parts are not private.
    let other = data.join("worktrees").join("run").join("a.txt");
    assert!(!touches_private("Read", &read(&other.to_string_lossy()), None));
    // A relative path is also judged inside the run's folder.
    let ws = data.join("vault");
    assert!(touches_private("Read", &read("x.txt"), Some(Path::new(&ws))));
    let ws = data.join("worktrees").join("run");
    assert!(!touches_private("Read", &read("x.txt"), Some(Path::new(&ws))));
}

#[test]
fn a_rule_pattern_that_names_or_covers_a_private_place_is_private() {
    for pattern in [
        "Read(~/.ssh/**)",
        "Read(**/.env)",
        "Read(**/.env*)",
        "Bash(cat ~/.aws/credentials)",
        "Edit(.npmrc)",
    ] {
        assert!(pattern_touches_private(pattern), "{pattern}");
    }
    let home = home().replace('\\', "/");
    assert!(pattern_touches_private(&format!("Read({home}/**)")));
    let parent = home.rsplit_once('/').unwrap().0;
    assert!(pattern_touches_private(&format!("Read({parent}/**)")));
    if let Ok(appdata) = std::env::var("APPDATA") {
        let appdata = appdata.replace('\\', "/");
        assert!(pattern_touches_private(&format!("Read({appdata}/**)")));
    }
    for pattern in [
        "Read(src/**)",
        "Bash(git status:*)",
        "WebFetch(domain:example.com)",
        "Read",
        "mcp__github__*",
    ] {
        assert!(!pattern_touches_private(pattern), "{pattern}");
    }
    assert!(!pattern_touches_private(&format!("Read({home}/projects/**)")));
}

mod resolve {
    use dots_core::policy::{
        allow_candidates, Action, Policy, Preset, Rule, PLUMBING_TOOLS,
    };
    use serde_json::json;

    use super::{bash, home, read};

    #[test]
    fn a_private_call_asks_under_every_preset_that_would_allow_it() {
        let ssh = read("~/.ssh/id_rsa");
        assert_eq!(Policy::preset(Preset::Trusted).resolve("Read", &ssh), Action::Ask);
        assert_eq!(Policy::preset(Preset::Sandboxed).resolve("Read", &ssh), Action::Ask);
        assert_eq!(Policy::preset(Preset::ReadOnly).resolve("Read", &ssh), Action::Ask);
        assert_eq!(
            Policy::preset(Preset::Trusted).resolve("Bash", &bash("cat ~/.aws/credentials")),
            Action::Ask
        );
        // `Edit(./**)` allows inside the folder, but not a `.env` there.
        assert_eq!(
            Policy::preset(Preset::Sandboxed).resolve("Edit", &read(".env")),
            Action::Ask
        );
        let mut p = Policy::preset(Preset::Custom);
        p.default = Action::Allow;
        p.rules.push(Rule::new("Read(**)", Action::Allow));
        p.rules.push(Rule::new("Bash(cat:*)", Action::Allow));
        assert_eq!(p.resolve("Read", &read("app/.env.local")), Action::Ask);
        assert_eq!(p.resolve("Bash", &bash("cat .npmrc")), Action::Ask);
        // Ordinary calls are unchanged.
        assert_eq!(p.resolve("Read", &read("src/a.rs")), Action::Allow);
        assert_eq!(p.resolve("Bash", &bash("cat README.md")), Action::Allow);
    }

    #[test]
    fn a_deny_still_wins_over_a_private_call() {
        let mut p = Policy::preset(Preset::Trusted);
        p.rules.push(Rule::new("Read(**/.env)", Action::Deny));
        assert_eq!(p.resolve("Read", &read(".env")), Action::Deny);
        // ReadOnly denies writes by rule.
        assert_eq!(
            Policy::preset(Preset::ReadOnly).resolve("Write", &read("~/.ssh/config")),
            Action::Deny
        );
        // A default deny stays deny.
        let mut p = Policy::preset(Preset::Custom);
        p.rules.clear();
        p.default = Action::Deny;
        assert_eq!(p.resolve("Read", &read("~/.ssh/id_rsa")), Action::Deny);
    }

    #[test]
    fn plumbing_tools_are_allowed_under_every_preset_unless_denied() {
        for tool in [
            "TodoWrite",
            "Task",
            "Agent",
            "ExitPlanMode",
            "EnterPlanMode",
            "ToolSearch",
            "ListMcpResourcesTool",
            "ReadMcpResourceTool",
            "BashOutput",
            "KillShell",
            "KillBash",
        ] {
            assert!(PLUMBING_TOOLS.contains(&tool), "{tool}");
            for preset in [Preset::ReadOnly, Preset::Sandboxed, Preset::Trusted, Preset::Custom] {
                assert_eq!(
                    Policy::preset(preset).resolve(tool, &json!({})),
                    Action::Allow,
                    "{tool} under {preset:?}"
                );
            }
            let mut p = Policy::preset(Preset::Sandboxed);
            p.rules.push(Rule::new(tool, Action::Ask));
            assert_eq!(p.resolve(tool, &json!({})), Action::Allow, "{tool} with an ask rule");
            p.rules.push(Rule::new(tool, Action::Deny));
            assert_eq!(p.resolve(tool, &json!({})), Action::Deny, "{tool} with a deny rule");
        }
        for tool in ["WebSearch", "WebFetch", "Workflow"] {
            assert!(!PLUMBING_TOOLS.contains(&tool), "{tool}");
        }
        assert_eq!(
            Policy::preset(Preset::ReadOnly).resolve("WebSearch", &json!({})),
            Action::Deny
        );
    }

    #[test]
    fn no_rule_is_offered_for_a_private_call() {
        assert!(allow_candidates("Read", &read("~/.ssh/id_rsa"), None).is_empty());
        assert!(allow_candidates("Read", &read("C:/Users/me/.ssh/id_rsa"), None).is_empty());
        assert!(allow_candidates("Edit", &read("app/.env"), None).is_empty());
        assert!(allow_candidates("Bash", &bash("cat C:/Users/me/.aws/credentials"), None).is_empty());
        assert!(allow_candidates("Bash", &bash("cat .env"), None).is_empty());
        // Ordinary calls still get their rules.
        assert!(!allow_candidates("Read", &read("src/a.rs"), None).is_empty());
        assert!(!allow_candidates("Bash", &bash("git log --oneline"), None).is_empty());
    }

    #[test]
    fn no_offered_rule_covers_a_private_place() {
        // A file in the home folder: "everything under" the home folder would cover `~/.ssh`.
        let home = home().replace('\\', "/");
        let file = format!("{home}/notes.txt");
        let offered = allow_candidates("Read", &read(&file), None);
        let patterns: Vec<&str> = offered.iter().map(|c| c.pattern.as_str()).collect();
        assert_eq!(patterns, vec![format!("Read({file})")]);
    }
}

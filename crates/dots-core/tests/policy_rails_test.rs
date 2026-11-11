//! What a call hits of a workspace's guardrails (guardrails spec §4.1): file writes, git and
//! `gh` actions, protected commands, paths in shell lines, the fail-closed cases and MCP names.

use std::path::Path;

use dots_core::accounts::guardrails::{GitAction, Guardrails};
use dots_core::policy::rails::{check, RailKind};
use serde_json::json;

const WS: &str = "C:/work/runs/R1";

fn g() -> Guardrails {
    Guardrails::defaults()
}

fn bash(command: &str) -> Option<String> {
    check(
        &g(),
        "Bash",
        &json!({ "command": command }),
        Some(Path::new(WS)),
    )
    .map(|h| h.what)
}

fn kind(command: &str) -> Option<RailKind> {
    check(
        &g(),
        "Bash",
        &json!({ "command": command }),
        Some(Path::new(WS)),
    )
    .map(|h| h.kind)
}

fn write(tool: &str, path: &str) -> Option<String> {
    check(
        &g(),
        tool,
        &json!({ "file_path": path }),
        Some(Path::new(WS)),
    )
    .map(|h| h.what)
}

#[test]
fn git_push_to_main_and_its_near_misses() {
    for c in [
        "git push origin main",
        "git push origin master",
        "git push origin HEAD:main",
        "git push origin feature:refs/heads/main",
        "git push",
        "git push origin",
        "git push origin HEAD",
        "git push --all origin",
        "cd repo && git push -u origin main",
        "git -C repo push origin main",
        "sudo git push origin main",
        "bash -c 'git push origin main'",
    ] {
        assert_eq!(bash(c).as_deref(), Some("pushing to main or master"), "{c}");
    }
    for c in [
        "git push origin feature",
        "git push -u origin dots/fix-icon",
        "git status",
        "git log main",
        "git push --tags origin",
    ] {
        assert_ne!(bash(c).as_deref(), Some("pushing to main or master"), "{c}");
    }
    assert_eq!(bash("git push origin feature"), None);
    assert_eq!(kind("git push origin main"), Some(RailKind::Git));
}

#[test]
fn force_delete_and_tags() {
    for c in [
        "git push -f origin feature",
        "git push --force origin feature",
        "git push --force-with-lease origin feature",
        "git push origin +feature",
    ] {
        assert_eq!(bash(c).as_deref(), Some("force pushing"), "{c}");
    }
    for c in [
        "git branch -D old",
        "git branch -d old",
        "git branch --delete old",
        "git push origin --delete old",
        "git push origin :old",
    ] {
        assert_eq!(bash(c).as_deref(), Some("deleting a branch"), "{c}");
    }
    for c in [
        "git tag v1.0",
        "git tag -a v1.0 -m x",
        "git tag -d v1.0",
        "git push --tags origin",
        "git push origin --follow-tags feature",
        "git push origin refs/tags/v1",
        "git push origin tag v1",
    ] {
        assert_eq!(bash(c).as_deref(), Some("making or pushing tags"), "{c}");
    }
    for c in [
        "git tag",
        "git tag -l",
        "git tag --list 'v*'",
        "git branch",
        "git branch -a",
    ] {
        assert_eq!(bash(c), None, "{c}");
    }
}

#[test]
fn gh_merge_and_release() {
    for c in [
        "gh pr merge 12 --squash",
        "gh -R acme/app pr merge 12",
        "gh api -X PUT repos/acme/app/pulls/12/merge",
    ] {
        assert_eq!(bash(c).as_deref(), Some("merging a pull request"), "{c}");
    }
    for c in [
        "gh release create v1",
        "gh release delete v1",
        "gh release upload v1 a.zip",
        "gh api repos/acme/app/releases -f tag_name=v1",
    ] {
        assert_eq!(
            bash(c).as_deref(),
            Some("publishing a GitHub release"),
            "{c}"
        );
    }
    for c in [
        "gh pr view 12",
        "gh release list",
        "gh release view v1",
        "gh api repos/acme/app/releases",
    ] {
        assert_eq!(bash(c), None, "{c}");
    }
}

#[test]
fn a_git_action_not_listed_is_not_protected() {
    let mut rails = g();
    rails.protected_git = vec![GitAction::ForcePush];
    let hit = |c: &str| check(&rails, "Bash", &json!({ "command": c }), None).map(|h| h.what);
    assert_eq!(hit("git push origin main"), None);
    assert_eq!(
        hit("git push -f origin x").as_deref(),
        Some("force pushing")
    );
}

#[test]
fn protected_commands() {
    for c in [
        "rm -rf build",
        "rm -fr build",
        "rm -r -f build",
        "rm --recursive --force build",
        "/bin/rm -Rf build",
        "Remove-Item -Recurse -Force build",
        "ri build -r -fo",
        "rd /s /q build",
        "find . -name x -exec rm -rf {} ;",
        "xargs rm -rf",
    ] {
        assert_eq!(bash(c).as_deref(), Some("the command rm -rf"), "{c}");
    }
    for c in ["rm build/x.o", "rm -r build", "Remove-Item x"] {
        assert_eq!(bash(c), None, "{c}");
    }
    assert_eq!(
        bash("kubectl -n prod apply -f x.yaml").as_deref(),
        Some("the command kubectl apply")
    );
    assert_eq!(
        bash("npx vercel deploy --prod").as_deref(),
        Some("the command vercel --prod")
    );
    assert_eq!(
        bash("TF_LOG=1 terraform.exe apply -auto-approve").as_deref(),
        Some("the command terraform apply")
    );
    assert_eq!(bash("terraform plan"), None);
    assert_eq!(bash("kubectl get pods"), None);
    assert_eq!(kind("rm -rf x"), Some(RailKind::Command));
}

#[test]
fn powershell_and_encoded() {
    use base64::Engine as _;
    let script: Vec<u8> = "git push origin main"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(script);
    assert_eq!(
        bash(&format!("powershell -EncodedCommand {b64}")).as_deref(),
        Some("pushing to main or master")
    );
    let hit = check(
        &g(),
        "PowerShell",
        &json!({ "command": "Remove-Item -Recurse -Force C:/x" }),
        None,
    );
    assert_eq!(hit.map(|h| h.what).as_deref(), Some("the command rm -rf"));
}

#[test]
fn file_writes_against_protected_paths() {
    for (tool, path, shown) in [
        ("Edit", "migrations/x.sql", "migrations/x.sql"),
        (
            "Write",
            "C:/work/runs/R1/db/migrations/0001.sql",
            "db/migrations/0001.sql",
        ),
        (
            "Write",
            "C:\\work\\runs\\R1\\.github\\workflows\\ci.yml",
            ".github/workflows/ci.yml",
        ),
        ("Edit", ".env", ".env"),
        ("Edit", "app/.env.local", "app/.env.local"),
        ("Write", "Cargo.lock", "Cargo.lock"),
        ("NotebookEdit", "infra/main.tf", "infra/main.tf"),
        ("MultiEdit", "infra", "infra"),
    ] {
        assert_eq!(
            write(tool, path),
            Some(format!("the file {shown}")),
            "{tool} {path}"
        );
    }
    for (tool, path) in [
        ("Edit", "src/main.rs"),
        ("Write", "src/infra/x.rs"),
        ("Read", "migrations/x.sql"),
        ("Grep", ".github/x"),
        ("Edit", "environment.ts"),
    ] {
        assert_eq!(write(tool, path), None, "{tool} {path}");
    }
    // Outside the run's folder: matched at every depth (fails closed).
    assert_eq!(
        write("Write", "D:/other/infra/x.tf"),
        Some("the file D:/other/infra/x.tf".into())
    );
    assert_eq!(
        write("Edit", "../sibling/.github/x.yml"),
        Some("the file ../sibling/.github/x.yml".into())
    );
    assert_eq!(
        check(
            &g(),
            "Edit",
            &json!({"file_path": "migrations/x.sql"}),
            None
        )
        .map(|h| h.kind),
        Some(RailKind::Path)
    );
}

#[test]
fn paths_in_a_shell_line() {
    for c in [
        "echo DROP > migrations/x.sql",
        "sed -i s/a/b/ .github/workflows/ci.yml",
        "cp new.lock Cargo.lock",
        "mv x infra/y",
        "Set-Content -Path .env -Value x",
    ] {
        assert!(
            bash(c).is_some_and(|w| w.starts_with("the file ")),
            "{c}: {:?}",
            bash(c)
        );
    }
    for c in [
        "cat migrations/x.sql",
        "grep -r TODO .github",
        "git diff migrations/x.sql",
        "head -n 5 Cargo.lock | wc -l",
        "Get-Content .env",
        "ls infra",
    ] {
        assert_eq!(bash(c), None, "{c}");
    }
}

#[test]
fn a_line_dots_cannot_read_fails_closed_when_it_mentions_something() {
    assert_eq!(kind("$cmd push origin main"), Some(RailKind::Unclear));
    assert_eq!(kind("eval \"$X migrations\""), Some(RailKind::Unclear));
    assert_eq!(
        kind("powershell -EncodedCommand !!!notbase64 push"),
        Some(RailKind::Unclear)
    );
    // Unreadable, but naming nothing protected.
    assert_eq!(bash("$cmd --version"), None);
    assert_eq!(bash("eval \"$X\""), None);
}

#[test]
fn mcp_tools_by_name() {
    let hit = |t: &str| check(&g(), t, &json!({}), None).map(|h| h.what);
    assert_eq!(
        hit("mcp__github__merge_pull_request").as_deref(),
        Some("merging a pull request")
    );
    assert_eq!(
        hit("mcp__github__create_release").as_deref(),
        Some("publishing a GitHub release")
    );
    assert_eq!(hit("mcp__github__get_pull_request"), None);
    assert_eq!(hit("mcp__github__list_releases"), None);
    // Other tools are not read.
    assert_eq!(hit("WebFetch"), None);
}

#[test]
fn empty_guardrails_protect_nothing() {
    let none = Guardrails {
        protected_paths: vec![],
        protected_git: vec![],
        protected_commands: vec![],
        member_max_preset: dots_core::policy::Preset::Trusted,
    };
    for c in ["git push origin main", "rm -rf /", "$cmd push"] {
        assert!(
            check(&none, "Bash", &json!({ "command": c }), None).is_none(),
            "{c}"
        );
    }
}

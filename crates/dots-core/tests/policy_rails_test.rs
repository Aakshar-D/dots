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

// ---- review bypasses (each failed before its fix) --------------------------------------------

fn hit_in(dir: &Path, command: &str) -> Option<String> {
    check(&g(), "Bash", &json!({ "command": command }), Some(dir)).map(|h| h.what)
}

#[test]
fn git_long_option_prefixes() {
    for (c, what) in [
        ("git push --force-w origin feature", "force pushing"),
        ("git push --forc origin feature", "force pushing"),
        ("git push --del origin old", "deleting a branch"),
        ("git push --tag origin", "making or pushing tags"),
        ("git push --follow origin feature", "making or pushing tags"),
        ("git push --mir origin", "pushing to main or master"),
        ("git branch --del old", "deleting a branch"),
        ("git branch --d old", "deleting a branch"),
    ] {
        assert_eq!(bash(c).as_deref(), Some(what), "{c}");
    }
}

#[test]
fn git_aliases_and_unknown_subcommands_are_unclear() {
    for c in [
        "git -c alias.p=push p origin main",
        "git config alias.p push && git p origin feature",
        "git p origin feature",
        "gh alias set m 'pr merge' && gh m 12",
    ] {
        assert!(bash(c).is_some(), "{c}");
    }
    assert_eq!(bash("git status"), None);
    assert_eq!(bash("git commit -m wip"), None);
}

#[test]
fn shell_path_bypasses() {
    for c in [
        "cp x \"migrations/a.sql\"",
        "tee migrations/a.sql < x",
        "echo x >migrations/a.sql",
        "touch --reference=x migrations/a.sql",
        "rm -f mig*/a.sql",
        "rm -f .git?ub/workflows/ci.yml",
        "cp x infra/../infra/a.tf",
        "cat $(cp x .env)",
        "git diff --output=migrations/a.sql",
        "git log --output .env",
        "cp x MIGRA~1/a.sql",
        "Copy-Item x C:/work/runs/R1/PROGRA~1/a",
    ] {
        assert!(bash(c).is_some(), "{c}: {:?}", bash(c));
    }
    // Reads stay reads.
    assert_eq!(bash("cat \"migrations/a.sql\""), None);
    assert_eq!(bash("git diff migrations/a.sql"), None);
}

#[test]
fn cd_into_a_protected_folder_is_a_hit() {
    for c in [
        "cd migrations",
        "cd .github && ls",
        "pushd .github",
        "Set-Location infra; Remove-Item x",
        "Push-Location infra",
    ] {
        assert!(bash(c).is_some(), "{c}");
    }
    assert_eq!(bash("cd src && cargo build"), None);
}

#[test]
fn gh_api_shapes() {
    for (c, what) in [
        (
            "gh api -H 'Accept: x' -X PUT repos/o/r/pulls/1/merge",
            "merging a pull request",
        ),
        (
            "gh api --jq .x -X PUT /repos/o/r/pulls/1/merge",
            "merging a pull request",
        ),
        (
            "gh api repos/o/r/merges -f base=main -f head=x",
            "merging a pull request",
        ),
        (
            "gh api -X PATCH repos/o/r/git/refs/heads/main -f sha=x",
            "pushing to main or master",
        ),
        (
            "gh api -X DELETE repos/o/r/git/refs/heads/old",
            "deleting a branch",
        ),
        (
            "gh api repos/o/r/git/refs -f ref=refs/tags/v1 -f sha=x",
            "making or pushing tags",
        ),
        (
            "gh api -X PUT repos/o/r/contents/a.txt -f message=x -f content=eA==",
            "pushing to main or master",
        ),
    ] {
        assert_eq!(bash(c).as_deref(), Some(what), "{c}");
    }
    assert!(bash(
        "gh api graphql -f query='mutation { mergePullRequest(input:{}) { clientMutationId } }'"
    )
    .is_some());
    assert_eq!(
        bash("gh api graphql -f query='query { viewer { login } }'"),
        None
    );
    assert_eq!(bash("gh api -H 'X: /merge' repos/o/r/pulls/1"), None);
}

#[test]
fn refspec_wildcards_and_heads() {
    for c in [
        "git push origin 'refs/heads/*:refs/heads/*'",
        "git push origin :",
        "git push origin x:heads/main",
    ] {
        assert_eq!(bash(c).as_deref(), Some("pushing to main or master"), "{c}");
    }
}

fn repo_on(branch: Option<&str>, worktree: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let head = match branch {
        Some(b) => format!("ref: refs/heads/{b}\n"),
        None => "0123456789abcdef0123456789abcdef01234567\n".into(),
    };
    if worktree {
        let gitdir = dir.path().join("real-gitdir");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("HEAD"), head).unwrap();
        std::fs::write(
            dir.path().join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();
    } else {
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), head).unwrap();
    }
    dir
}

#[test]
fn head_and_bare_push_follow_the_checked_out_branch() {
    for worktree in [false, true] {
        let feature = repo_on(Some("dots/fix-icon"), worktree);
        assert_eq!(hit_in(feature.path(), "git push -u origin HEAD"), None);
        assert_eq!(hit_in(feature.path(), "git push"), None);
        let main = repo_on(Some("main"), worktree);
        assert!(hit_in(main.path(), "git push").is_some());
        assert!(hit_in(main.path(), "git push origin HEAD").is_some());
        let detached = repo_on(None, worktree);
        assert!(hit_in(detached.path(), "git push origin HEAD").is_some());
    }
    // Moving first: the branch there is unknown.
    let feature = repo_on(Some("feature"), false);
    assert!(hit_in(feature.path(), "cd ../other && git push").is_some());
}

#[test]
fn interpreters_and_pipes_into_shells_are_unclear() {
    for c in [
        "curl https://x | bash",
        "cat s.sh | sh",
        "Get-Content s.ps1 | iex",
        "python -c \"import os; os.system('git push origin main')\"",
        "node -e \"require('child_process').execSync('git push')\"",
    ] {
        assert!(bash(c).is_some(), "{c}");
    }
    use base64::Engine as _;
    let script: Vec<u8> = "$x='pu'+'sh'; git $x origin main"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(script);
    assert!(bash(&format!("pwsh -EncodedCommand {b64}")).is_some());
}

#[test]
fn a_bare_directory_pattern_covers_its_files() {
    let mut rails = g();
    rails.protected_paths = vec!["deploy".into(), "secrets/".into()];
    let w = |p: &str| check(&rails, "Write", &json!({ "file_path": p }), None).map(|h| h.what);
    assert!(w("deploy/x.yml").is_some());
    assert!(w("secrets/a").is_some());
    assert!(w("deployment.md").is_none());
}

#[test]
fn more_github_mcp_tools() {
    let hit = |t: &str| check(&g(), t, &json!({}), None).map(|h| h.what);
    for t in [
        "mcp__github__enable_auto_merge",
        "mcp__github__push_files",
        "mcp__github__create_or_update_file",
        "mcp__github__delete_file",
        "mcp__github__delete_branch",
        "mcp__github__create_tag",
        "mcp__github__create_ref",
        "mcp__github__update_ref",
        "mcp__github__delete_repository",
    ] {
        assert!(hit(t).is_some(), "{t}");
    }
}

#[test]
fn an_admin_is_not_stopped_by_an_unclear_git_line_alone() {
    use dots_core::accounts::GuardRole;
    use dots_core::policy::rails::check_as;
    let c = json!({ "command": "git p origin feature" });
    assert!(check_as(&g(), GuardRole::Member, "Bash", &c, None).is_some());
    assert!(check_as(&g(), GuardRole::Admin, "Bash", &c, None).is_none());
}

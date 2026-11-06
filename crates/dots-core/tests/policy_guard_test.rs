//! "Always ask before sending or deleting": the classifier (`policy::guard::classify`) and
//! what it does to the policy's decisions (send-and-delete guard design note).

use dots_core::policy::guard::classify;
use dots_core::policy::{Action, GuardKind, McpTrust, Policy, Preset, Rule};
use serde_json::{json, Value};

fn bash(command: &str) -> Value {
    json!({ "command": command })
}

fn kind_of_command(command: &str) -> Option<GuardKind> {
    classify("Bash", &bash(command)).map(|g| g.kind)
}

/// Commands that delete, as the guard must see them.
const DELETES: &[&str] = &[
    "rm notes.txt",
    "rm -rf build",
    "rm -rf /",
    "/bin/rm -f x",
    "\\rm x",
    "r'm' x",
    "\"rm\" x",
    "RM.EXE x",
    "cd src && rm old.rs",
    "cargo build; rm -rf target",
    "ls | xargs rm",
    "find . -name '*.log' -delete",
    "find . -name '*.tmp' -exec rm {} \\;",
    "echo $(rm secrets.txt)",
    "echo `rm secrets.txt`",
    "echo \"$(rm -rf x)\"",
    "sudo rm -rf /var/log",
    "sudo -u bob rm x",
    "timeout 5 rm x",
    "env FOO=1 rm x",
    "FOO=1 rm x",
    "nohup rm -rf x &",
    "bash -c 'rm -rf x'",
    "sh -c \"cd a && rm b\"",
    "bash -lc 'rm x'",
    "cmd /c del /q x.txt",
    "cmd.exe /C \"rd /s /q build\"",
    "del /s /q build",
    "erase x.txt",
    "rmdir build",
    "rd /s /q build",
    "unlink x",
    "shred -u secret",
    "truncate -s 0 log.txt",
    "npx rimraf dist",
    "rimraf node_modules",
    "Remove-Item -Recurse -Force build",
    "remove-item x",
    "ri x",
    "Get-ChildItem *.log | Remove-Item",
    "gci | % { Remove-Item $_ }",
    "Get-ChildItem | ForEach-Object { rm $_.FullName }",
    "Clear-Content log.txt",
    "Clear-RecycleBin -Force",
    "Remove-ADUser bob",
    "Remove-AzResourceGroup -Name prod",
    "$x = Remove-Item a",
    "$x=Remove-Item a",
    "powershell -Command \"Remove-Item x\"",
    "powershell.exe -NoProfile -c Remove-Item x",
    "pwsh -nop -command \"del x\"",
    "powershell Remove-Item x",
    "& \"C:\\Windows\\System32\\cmd.exe\" /c del x",
    "iex 'Remove-Item x'",
    "Invoke-Expression \"rm x\"",
    "eval 'rm -rf x'",
    "Start-Process cmd -ArgumentList '/c del x'",
    "[System.IO.File]::Delete('x.txt')",
    "[IO.Directory]::Delete('build', $true)",
    "python -c \"import shutil; shutil.rmtree('x')\"",
    "python3 -c 'import os; os.remove(\"x\")'",
    "node -e \"require('fs').rmSync('x', {recursive: true})\"",
    "git clean -fdx",
    "git reset --hard HEAD~1",
    "git push --force origin main",
    "git push -f",
    "git push origin --delete feature",
    "git push origin :feature",
    "git push origin +main",
    "git push --force-with-lease",
    "git branch -D feature",
    "git branch -d feature",
    "git branch --delete feature",
    "git tag -d v1",
    "git rm src/old.rs",
    "git stash drop",
    "git stash clear",
    "git checkout -- .",
    "git checkout .",
    "git restore src/main.rs",
    "git -C repo clean -fd",
    "git worktree remove ../wt",
    "gh repo delete me/x --yes",
    "gh release delete v1",
    "gh api -X DELETE repos/me/x",
    "gh api --method=DELETE repos/me/x",
    "curl -X DELETE https://api.example.com/items/1",
    "curl --request DELETE https://api.example.com/items/1",
    "Invoke-RestMethod -Method Delete -Uri https://api.example.com/x",
    "sqlite3 app.db \"DELETE FROM users\"",
    "psql -c 'DROP TABLE users'",
    "mysql -e \"truncate table logs\"",
    "sqlite3 app.db <<EOF\nDELETE FROM users;\nEOF",
    "redis-cli flushall",
    "docker rm -f web",
    "docker system prune -af",
    "docker volume rm data",
    "kubectl delete pod web",
    "aws s3 rm s3://bucket/key",
    "aws s3 rb s3://bucket --force",
    "aws ec2 terminate-instances --instance-ids i-1",
    "gcloud compute instances delete vm1",
    "az group delete -n prod",
    "terraform destroy -auto-approve",
    "helm uninstall web",
    "heroku apps:destroy myapp",
    "npm unpublish my-pkg",
    "rsync -a --delete src/ dst/",
    "ssh server 'rm -rf /srv/app'",
    "ssh -i key.pem user@host rm -rf /srv",
    "wsl rm -rf ~/x",
    "mkfs.ext4 /dev/sdb1",
    "format D: /q",
    "dd if=/dev/zero of=/dev/sda",
    "rm\tx",
    "r\"m\" x",
    "cd /d C:\\x & del *.*",
    "\"/usr/bin/rm\" -rf x",
    "command rm x",
    "exec rm x",
    "true || rm x",
    "(cd a; rm b)",
    "{ rm a; }",
    "if true; then rm x; fi",
    "for f in *.log; do rm \"$f\"; done",
    "x=$(rm y)",
    "cat <(rm y)",
];

/// Commands that send something out of the machine.
const SENDS: &[&str] = &[
    "git push",
    "git push origin main",
    "git -C repo push",
    "git -c user.name=x push origin main",
    "git push -u origin feature",
    "git send-email 0001.patch",
    "gh pr create --fill",
    "gh pr comment 12 --body hi",
    "gh pr merge 12",
    "gh issue create --title x",
    "gh issue comment 3 -b ok",
    "gh release create v1",
    "gh gist create notes.md",
    "gh api repos/me/x/issues -f title=x",
    "gh api -X POST repos/me/x/issues",
    "curl -X POST https://hooks.slack.com/x -d '{}'",
    "curl -d 'a=1' https://example.com",
    "curl --data-binary @file https://example.com",
    "curl -F file=@x https://example.com/upload",
    "curl --json '{}' https://example.com",
    "curl -XPOST https://example.com",
    "curl -T file https://example.com",
    "wget --post-data 'a=1' https://example.com",
    "Invoke-WebRequest -Uri https://x.com -Method Post -Body '{}'",
    "iwr https://x.com -Method:POST",
    "Invoke-RestMethod https://x.com -Method Put",
    "irm https://x.com -Body $b",
    "curl https://x.com -Method Post",
    "http POST example.com a=1",
    "Send-MailMessage -To a@b.c -Subject hi",
    "sendmail a@b.c < msg.txt",
    "mail -s hi a@b.c",
    "npm publish",
    "cargo publish",
    "docker push me/image",
    "twine upload dist/*",
    "scp report.pdf user@host:/tmp/",
    "rsync -a ./ user@host:/srv/",
    "python -c \"import requests; requests.post('https://x', json={})\"",
    "python -c 'import smtplib'",
    "cd x && git push",
];

/// Commands that neither send nor delete.
const NEITHER: &[&str] = &[
    "ls -la",
    "git status",
    "git diff",
    "git add .",
    "git commit -m 'remove old code and delete unused files'",
    "git log --oneline",
    "git branch",
    "git branch -a",
    "git branch -vv",
    "git push --help",
    "git restore --staged src/main.rs",
    "git checkout main",
    "git pull",
    "git fetch --prune",
    "echo rm -rf x",
    "echo 'delete everything'",
    "grep -r 'rm -rf' .",
    "cat rm.txt",
    "ls > rm",
    "cargo build",
    "cargo test",
    "cargo remove serde",
    "npm install",
    "npm uninstall left-pad",
    "npm run build",
    "docker run --rm alpine echo hi",
    "curl https://example.com",
    "curl -I https://example.com",
    "curl -X GET https://example.com",
    "Invoke-WebRequest https://example.com -OutFile x.html",
    "Get-ChildItem",
    "Get-Content x.txt",
    "gh pr list",
    "gh pr view 12",
    "gh issue list",
    "gh api repos/me/x",
    "python script.py",
    "node build.js",
    "mkdir -p build && cp a b",
    "mv a b",
    "Copy-Item a b",
    "Write-Output 'Remove-Item'",
    "psql -c 'select * from users'",
    "kubectl get pods",
    "aws s3 ls",
    "docker ps",
    "terraform plan",
    "echo 'a && rm b'",
    "echo \"a; del b\"",
    "git stash",
    "git stash pop",
    "npm run lint -- --fix",
    "Select-String -Pattern 'Remove-Item' -Path *.ps1",
    "",
];

#[test]
fn commands_that_delete_are_found() {
    let missed: Vec<_> = DELETES
        .iter()
        .filter(|c| kind_of_command(c) != Some(GuardKind::Delete))
        .map(|c| (c, kind_of_command(c)))
        .collect();
    assert!(missed.is_empty(), "not found as deletes: {missed:#?}");
}

#[test]
fn commands_that_send_are_found() {
    let missed: Vec<_> = SENDS
        .iter()
        .filter(|c| kind_of_command(c) != Some(GuardKind::Send))
        .map(|c| (c, kind_of_command(c)))
        .collect();
    assert!(missed.is_empty(), "not found as sends: {missed:#?}");
}

#[test]
fn commands_that_neither_send_nor_delete_are_left_alone() {
    let flagged: Vec<_> = NEITHER
        .iter()
        .filter_map(|c| classify("Bash", &bash(c)).map(|g| (c, g)))
        .collect();
    assert!(flagged.is_empty(), "flagged though harmless: {flagged:#?}");
}

#[test]
fn powershell_is_read_like_bash() {
    let found = classify("PowerShell", &bash("Remove-Item -Recurse C:\\temp\\x")).unwrap();
    assert_eq!(found.kind, GuardKind::Delete);
    assert_eq!(found.summary, "delete C:\\temp\\x");
}

#[test]
fn an_encoded_powershell_command_is_decoded_or_asks() {
    use base64::Engine as _;
    let utf16: Vec<u8> = "Remove-Item x"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(utf16);
    assert_eq!(
        kind_of_command(&format!("powershell -EncodedCommand {b64}")),
        Some(GuardKind::Delete)
    );
    assert_eq!(
        kind_of_command("powershell -enc not-base64!!"),
        Some(GuardKind::Unclear)
    );
}

#[test]
fn a_command_built_from_a_variable_is_unclear() {
    assert_eq!(kind_of_command("iex $payload"), Some(GuardKind::Unclear));
    assert_eq!(kind_of_command("eval \"$CMD\""), Some(GuardKind::Unclear));
}

#[test]
fn a_delete_names_its_files() {
    let found = classify("Bash", &bash("rm a.txt b.txt c.txt")).unwrap();
    assert_eq!(found.summary, "delete 3 files: a.txt, b.txt, c.txt");
    let one = classify("Bash", &bash("rm -f notes.md")).unwrap();
    assert_eq!(one.summary, "delete notes.md");
    let many = classify("Bash", &bash("rm 1 2 3 4 5 6 7")).unwrap();
    assert_eq!(many.summary, "delete 7 files: 1, 2, 3, 4, 5 and 2 more");
}

#[test]
fn a_line_that_sends_and_deletes_is_a_delete_and_says_both() {
    let found = classify("Bash", &bash("git push && rm -rf build")).unwrap();
    assert_eq!(found.kind, GuardKind::Delete);
    assert!(
        found.summary.contains("push commits with git"),
        "{}",
        found.summary
    );
    assert!(found.summary.contains("delete build"), "{}", found.summary);
}

fn mcp_kind(tool: &str) -> Option<GuardKind> {
    classify(tool, &json!({})).map(|g| g.kind)
}

#[test]
fn mcp_tools_that_send_are_found() {
    let tools = [
        "mcp__gmail__send_email",
        "mcp__gmail__send_draft",
        "mcp__gmail__reply_to_email",
        "mcp__gmail__forward_message",
        "mcp__slack__chat.postMessage",
        "mcp__slack__slack_post_message",
        "mcp__slack__post_message",
        "mcp__slack__reply_to_thread",
        "mcp__slack__add_reaction",
        "mcp__github__create_issue",
        "mcp__github__add_issue_comment",
        "mcp__github__create_pull_request",
        "mcp__github__merge_pull_request",
        "mcp__github__push_files",
        "mcp__github__create_pull_request_review",
        "mcp__google_calendar__create_event",
        "mcp__notion__create_comment",
        "mcp__twitter__tweet",
        "mcp__drive__share_file",
        "mcp__teams__sendMessage",
        "mcp__x__publish_post",
        "mcp__zapier__webhook_post",
        "mcp__outlook__SendMail",
        "mcp__discord__send-message",
        "mcp__linear__create_comment",
        "mcp__jira__add_comment",
        "mcp__mailchimp__send_campaign",
        "mcp__claude_ai_Gmail__send_email",
    ];
    let missed: Vec<_> = tools
        .iter()
        .filter(|t| mcp_kind(t) != Some(GuardKind::Send))
        .map(|t| (t, mcp_kind(t)))
        .collect();
    assert!(missed.is_empty(), "not found as sends: {missed:#?}");
}

#[test]
fn mcp_tools_that_delete_are_found() {
    for tool in [
        "mcp__gmail__delete_email",
        "mcp__gmail__trash_message",
        "mcp__gmail__archive_thread",
        "mcp__github__delete_file",
        "mcp__github__delete_branch",
        "mcp__notion__archive_page",
        "mcp__drive__remove_file",
        "mcp__db__drop_table",
        "mcp__fs__deleteFile",
        "mcp__aws__terminate_instance",
        "mcp__files__purge",
        "mcp__x__destroy_repo",
    ] {
        assert_eq!(mcp_kind(tool), Some(GuardKind::Delete), "{tool} deletes");
    }
}

#[test]
fn mcp_tools_that_read_or_draft_are_left_alone() {
    for tool in [
        "mcp__gmail__search_emails",
        "mcp__gmail__read_email",
        "mcp__gmail__get_message",
        "mcp__gmail__list_messages",
        "mcp__gmail__create_draft",
        "mcp__gmail__draft_reply",
        "mcp__gmail__update_draft",
        "mcp__slack__list_channels",
        "mcp__slack__get_thread_replies",
        "mcp__slack__search_messages",
        "mcp__github__get_pull_request",
        "mcp__github__list_issues",
        "mcp__github__get_issue_comments",
        "mcp__github__search_code",
        "mcp__github__get_file_contents",
        "mcp__notion__search",
        "mcp__notion__query_database",
        "mcp__drive__get_shared_drives",
        "mcp__posts__get_post",
        "mcp__x__list_deleted_items",
        "mcp__sf__query",
        // Dots' own tools and the browser are not the guard's.
        "mcp__dots__remember",
        "mcp__dots__notify",
        "mcp__browser__browser_click",
    ] {
        assert_eq!(mcp_kind(tool), None, "{tool} is harmless");
    }
}

#[test]
fn an_mcp_summary_names_the_server_and_tool() {
    let found = classify("mcp__gmail__send_email", &json!({"to": "a@b.c"})).unwrap();
    assert_eq!(found.summary, "send an email with Gmail (send_email)");
    let found = classify("mcp__google_drive__delete_file", &json!({})).unwrap();
    assert_eq!(found.summary, "delete file with Google Drive (delete_file)");
}

#[test]
fn plumbing_file_and_dots_tools_are_never_guarded() {
    for tool in [
        "TodoWrite",
        "Read",
        "Write",
        "Edit",
        "Glob",
        "Grep",
        "Task",
        "WebFetch",
        "WebSearch",
        "Notify",
        "Memory",
        "Workflow",
        "Skill",
    ] {
        assert_eq!(
            classify(
                tool,
                &json!({"file_path": "rm -rf x", "command": "rm -rf x"})
            ),
            None,
            "{tool}"
        );
    }
}

#[test]
fn an_app_tool_that_deletes_is_found() {
    let input = dots_core::apps::policy_input("projects", "delete_task", true, &json!({}));
    let found = classify("App", &input).unwrap();
    assert_eq!(found.kind, GuardKind::Delete);
    assert_eq!(
        found.summary,
        "delete task with the app projects (delete_task)"
    );
    let read = dots_core::apps::policy_input("projects", "list_tasks", false, &json!({}));
    assert_eq!(classify("App", &read), None);
    // An app's data stays on this machine: a comment sends nothing.
    let comment = dots_core::apps::policy_input("projects", "comment", true, &json!({}));
    assert_eq!(classify("App", &comment), None);
}

// ---- policy decisions ----------------------------------------------------------------------

fn with_rules(preset: Preset, extra: &[(&str, Action)]) -> Policy {
    let mut p = Policy::preset(preset);
    p.rules.extend(extra.iter().map(|(t, a)| Rule::new(t, *a)));
    p
}

const TRUSTED_SERVER: McpTrust = McpTrust {
    held: false,
    read_only: false,
};

#[test]
fn every_preset_starts_with_the_guard_on() {
    for preset in [
        Preset::ReadOnly,
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::Custom,
    ] {
        assert!(Policy::preset(preset).ask_send_delete, "{preset:?}");
    }
}

#[test]
fn a_policy_stored_before_the_switch_reads_as_on() {
    let stored = json!({
        "preset": "trusted",
        "rules": [],
        "default": "allow",
        "mcp_default": "ask",
    });
    let p: Policy = serde_json::from_value(stored).unwrap();
    assert!(p.ask_send_delete);
    // And a policy saved with it off stays off.
    let off: Policy = serde_json::from_value(json!({
        "preset": "trusted", "rules": [], "default": "allow", "mcp_default": "ask",
        "ask_send_delete": false,
    }))
    .unwrap();
    assert!(!off.ask_send_delete);
}

#[test]
fn an_allow_rule_for_a_delete_asks_while_the_guard_is_on() {
    let mut p = with_rules(Preset::Sandboxed, &[("Bash(rm:*)", Action::Allow)]);
    assert_eq!(p.resolve("Bash", &bash("rm notes.txt")), Action::Ask);
    p.ask_send_delete = false;
    assert_eq!(p.resolve("Bash", &bash("rm notes.txt")), Action::Allow);
}

#[test]
fn the_trusted_default_asks_for_sends_and_deletes() {
    let mut p = Policy::preset(Preset::Trusted);
    assert_eq!(p.resolve("Bash", &bash("rm -rf build")), Action::Ask);
    assert_eq!(p.resolve("PowerShell", &bash("Remove-Item x")), Action::Ask);
    assert_eq!(
        p.resolve("Bash", &bash("curl -X POST https://x.com")),
        Action::Ask
    );
    assert_eq!(p.resolve("Bash", &bash("gh pr create --fill")), Action::Ask);
    // Everything else stays allowed.
    assert_eq!(p.resolve("Bash", &bash("cargo test")), Action::Allow);
    assert_eq!(
        p.resolve("Write", &json!({"file_path": "a.txt"})),
        Action::Allow
    );
    assert_eq!(p.resolve("TodoWrite", &json!({})), Action::Allow);
    // Switched off, Trusted allows them as it did.
    p.ask_send_delete = false;
    assert_eq!(p.resolve("Bash", &bash("rm -rf build")), Action::Allow);
    assert_eq!(
        p.resolve("Bash", &bash("curl -X POST https://x.com")),
        Action::Allow
    );
}

#[test]
fn a_deny_still_denies() {
    let p = with_rules(Preset::Trusted, &[("Bash(rm:*)", Action::Deny)]);
    assert_eq!(p.resolve("Bash", &bash("rm x")), Action::Deny);
    let ro = Policy::preset(Preset::ReadOnly);
    assert_eq!(ro.resolve("Bash", &bash("rm x")), Action::Deny);
    let mcp = with_rules(Preset::Trusted, &[("mcp__gmail__send_email", Action::Deny)]);
    assert_eq!(
        mcp.resolve_mcp("mcp__gmail__send_email", &json!({}), TRUSTED_SERVER),
        Action::Deny
    );
}

#[test]
fn an_mcp_allow_rule_for_a_send_asks_while_the_guard_is_on() {
    let mut p = with_rules(Preset::Trusted, &[("mcp__gmail__*", Action::Allow)]);
    assert_eq!(
        p.resolve_mcp("mcp__gmail__send_email", &json!({}), TRUSTED_SERVER),
        Action::Ask
    );
    assert_eq!(
        p.resolve_mcp("mcp__gmail__search_emails", &json!({}), TRUSTED_SERVER),
        Action::Allow
    );
    p.ask_send_delete = false;
    assert_eq!(
        p.resolve_mcp("mcp__gmail__send_email", &json!({}), TRUSTED_SERVER),
        Action::Allow
    );
}

#[test]
fn a_read_only_hint_does_not_exempt_a_tool_named_to_delete() {
    let p = Policy::preset(Preset::Trusted);
    let read_only = McpTrust {
        held: false,
        read_only: true,
    };
    assert_eq!(
        p.resolve_mcp("mcp__fs__delete_file", &json!({}), read_only),
        Action::Ask
    );
    assert_eq!(
        p.resolve_mcp("mcp__fs__read_file", &json!({}), read_only),
        Action::Allow
    );
}

#[test]
fn the_guard_does_not_touch_plumbing_tools() {
    let p = Policy::preset(Preset::Sandboxed);
    for tool in ["TodoWrite", "Task", "ToolSearch"] {
        assert_eq!(p.resolve(tool, &json!({})), Action::Allow, "{tool}");
    }
}

#[test]
fn an_app_delete_asks_under_the_guard() {
    let p = Policy::preset(Preset::Trusted);
    let input = dots_core::apps::policy_input("projects", "delete_task", true, &json!({}));
    assert_eq!(p.resolve("App", &input), Action::Ask);
    let read = dots_core::apps::policy_input("projects", "list_tasks", false, &json!({}));
    assert_eq!(p.resolve("App", &read), Action::Allow);
}

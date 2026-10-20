mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use dots_core::engine::local::tools::{self, ToolOutput, MAX_OUTPUT, MAX_READ_BYTES};
use dots_core::policy::{Action, Policy, Preset, Rule};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "hello\nworld\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src/deep")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("src/deep/lib.rs"), "pub fn Hello() {}\n").unwrap();
    dir
}

async fn run(ws: &Path, name: &str, args: Value) -> ToolOutput {
    let call = tools::prepare(name, &args, ws).unwrap_or_else(|e| panic!("prepare: {e}"));
    tools::run(&call, ws, &CancellationToken::new()).await
}

/// Creates a directory link at `link` pointing to `target` (a junction on Windows).
fn dir_link(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn confine_accepts_paths_inside_the_workspace() {
    let ws = workspace();
    let root = std::fs::canonicalize(ws.path()).unwrap();
    assert_eq!(
        tools::confine(ws.path(), "src/main.rs").unwrap(),
        root.join("src").join("main.rs")
    );
    assert_eq!(
        tools::confine(ws.path(), "./README.md").unwrap(),
        root.join("README.md")
    );
    let abs = ws.path().join("README.md");
    assert_eq!(
        tools::confine(ws.path(), abs.to_str().unwrap()).unwrap(),
        root.join("README.md")
    );
    assert_eq!(
        tools::confine(ws.path(), "new/dir/file.txt").unwrap(),
        root.join("new").join("dir").join("file.txt")
    );
    assert_eq!(tools::confine(ws.path(), ".").unwrap(), root);
}

#[test]
fn confine_rejects_escapes() {
    let ws = workspace();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "s").unwrap();
    let outside_file = outside.path().join("secret.txt");
    for raw in [
        "",
        "  ",
        "..",
        "../secret.txt",
        "src/../../secret.txt",
        "README.md:hidden",
        outside_file.to_str().unwrap(),
    ] {
        assert!(tools::confine(ws.path(), raw).is_err(), "accepted {raw:?}");
    }
    dir_link(outside.path(), &ws.path().join("escape"));
    assert!(tools::confine(ws.path(), "escape/secret.txt").is_err());
    assert!(tools::confine(ws.path(), "escape/new.txt").is_err());
}

#[test]
fn confine_rejects_dangling_links() {
    let ws = workspace();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("gone");
    std::fs::create_dir(&target).unwrap();
    dir_link(&target, &ws.path().join("dangling"));
    std::fs::remove_dir(&target).unwrap();
    for raw in ["dangling", "dangling/new.txt"] {
        let err = tools::confine(ws.path(), raw).unwrap_err();
        assert!(err.contains("cannot be resolved"), "{raw}: {err}");
    }
}

#[cfg(windows)]
#[test]
fn confine_refuses_network_device_and_drive_relative_paths() {
    let ws = workspace();
    for raw in [
        r"\\attacker.invalid\share\x",
        r"\\?\UNC\attacker.invalid\share\x",
        r"\\.\pipe\x",
    ] {
        let err = tools::confine(ws.path(), raw).unwrap_err();
        assert!(
            err.contains("network and device paths are not allowed"),
            "{raw}: {err}"
        );
    }
    let err = tools::confine(ws.path(), "C:foo.txt").unwrap_err();
    assert!(err.contains("drive-relative"), "{err}");
}

#[test]
fn prepare_validates_calls_before_the_policy() {
    let ws = workspace();
    let p = ws.path();
    let call = tools::prepare("read_file", &json!("{\"file_path\":\"README.md\"}"), p).unwrap();
    assert_eq!(call.alias, "Read");
    assert_eq!(call.input, json!({"file_path": "README.md"}));
    let call = tools::prepare("shell", &json!({"command": "git status"}), p).unwrap();
    assert_eq!(call.alias, "Bash");
    let call = tools::prepare("list_dir", &json!(""), p).unwrap();
    assert_eq!(call.input, json!({}));
    let call = tools::prepare("glob", &json!({"pattern": "*.md", "path": ""}), p).unwrap();
    assert_eq!(call.input, json!({"pattern": "*.md"}));

    let err = tools::prepare("fly", &json!({}), p).unwrap_err();
    assert!(
        err.contains("unknown tool 'fly'") && err.contains("read_file"),
        "{err}"
    );
    let err = tools::prepare("read_file", &json!("{not json"), p).unwrap_err();
    assert!(err.contains("not valid JSON"), "{err}");
    let err = tools::prepare("read_file", &json!("[1]"), p).unwrap_err();
    assert!(err.contains("must be a JSON object"), "{err}");
    let err = tools::prepare(
        "edit_file",
        &json!({"file_path": "a", "old_string": "x"}),
        p,
    )
    .unwrap_err();
    assert!(err.contains("'new_string'"), "{err}");
    let err = tools::prepare("read_file", &json!({"file_path": "../x"}), p).unwrap_err();
    assert!(err.contains(".."), "{err}");
    assert_eq!(tools::alias("grep"), Some("Grep"));
    // Every local tool but `skill` and `context`, which a run gets only with skills
    // (`skill::schema`) and context collections (`context::schema`).
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len() - 2
    );
}

#[test]
fn a_prepared_input_passes_prepare_again_unchanged() {
    // An approval stores the prepared input under the policy alias. When Dots performs the call
    // later, it prepares that input again; what comes out must be the same call.
    let ws = workspace();
    let root = std::fs::canonicalize(ws.path()).unwrap();
    let inside = root
        .join("src")
        .join("main.rs")
        .to_string_lossy()
        .to_string();
    let calls: Vec<(&str, Value)> = vec![
        (
            "read_file",
            json!({"file_path": "./src//main.rs", "offset": 1, "limit": 5}),
        ),
        ("read_file", json!({ "file_path": inside })),
        (
            "write_file",
            json!({"file_path": "notes/new.txt", "content": "a\nb\n", "extra": 1}),
        ),
        (
            "edit_file",
            json!({"file_path": "README.md", "old_string": "hello", "new_string": "bye"}),
        ),
        ("list_dir", json!({})),
        ("list_dir", json!({"path": "src/"})),
        ("glob", json!({"pattern": "**/*.rs", "path": "."})),
        (
            "grep",
            json!({"pattern": "fn", "path": "src", "glob": "*.rs", "ignore_case": true}),
        ),
        ("shell", json!({"command": "git status", "timeout": 30})),
        ("shell", json!("{\"command\": \"git log -1\"}")),
        ("skill", json!({"name": "pdf", "file": "ref/a.md"})),
        ("skill", json!({"name": "forms:fill", "file": ""})),
        ("context", json!({"collection": "kb", "query": "style"})),
        (
            "context",
            json!({"collection": "kb", "path": "a.md", "offset": 2}),
        ),
    ];
    let mut tools_seen = std::collections::BTreeSet::new();
    for (name, args) in &calls {
        let first = tools::prepare(name, args, ws.path()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let stored_as = tools::local_name(first.alias).expect("an alias names a local tool");
        assert_eq!(stored_as, *name);
        let again = tools::prepare(stored_as, &first.input, ws.path())
            .unwrap_or_else(|e| panic!("{name} again: {e}"));
        assert_eq!(again, first, "{name} {args}");
        tools_seen.insert(*name);
    }
    // Every local tool was checked, and the two name maps are each other's inverse.
    let all: std::collections::BTreeSet<&str> = tools::TOOLS.iter().map(|(n, _)| *n).collect();
    assert_eq!(tools_seen, all);
    for (name, alias) in tools::TOOLS {
        assert_eq!(tools::alias(name), Some(*alias));
        assert_eq!(tools::local_name(alias), Some(*name));
    }
    for unknown in ["shell", "bash", "WebFetch", "mcp__x__y", ""] {
        assert_eq!(tools::local_name(unknown), None, "{unknown:?}");
    }
}

/// On Windows a process the command started is stopped when the command returns. The result
/// says so only when that process held the command's output; one started in a window of its
/// own is stopped without a word, so the model has to know it from the tool's description.
#[cfg(windows)]
#[test]
fn the_shell_description_says_that_a_background_process_is_stopped() {
    let schemas = tools::schemas();
    let shell = schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["function"]["name"] == "shell")
        .expect("the shell tool");
    let description = shell["function"]["description"].as_str().unwrap();
    assert!(
        description.ends_with(
            " A process the command starts in the background is stopped when the command returns."
        ),
        "{description}"
    );
}

/// Runs `prepare` and then the same policy resolution `ApprovalHub::check` does.
fn gate(ws: &Path, policy: &Policy, name: &str, args: Value) -> (Value, Action) {
    let call = tools::prepare(name, &args, ws).unwrap_or_else(|e| panic!("prepare: {e}"));
    let action = policy.resolve_in(call.alias, &call.input, Some(ws));
    (call.input, action)
}

#[test]
fn padded_paths_are_canonical_before_the_gate() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir(p.join(".git")).unwrap();
    std::fs::write(p.join(".git/config"), "[core]\n").unwrap();
    std::fs::create_dir(p.join(".claude")).unwrap();
    let sandboxed = Policy::preset(Preset::Sandboxed);
    for (raw, canonical) in [
        (" .git/config", ".git/config"),
        ("\t.git/config", ".git/config"),
        (".git\n", ".git"),
        (" .claude/settings.json", ".claude/settings.json"),
    ] {
        let (input, action) = gate(
            p,
            &sandboxed,
            "write_file",
            json!({"file_path": raw, "content": "x"}),
        );
        assert_eq!(input["file_path"], json!(canonical), "{raw:?}");
        assert_eq!(action, Action::Deny, "{raw:?}");
    }
    let (input, action) = gate(
        p,
        &sandboxed,
        "write_file",
        json!({"file_path": "./src/new.rs", "content": "x"}),
    );
    assert_eq!(input["file_path"], json!("src/new.rs"));
    assert_eq!(action, Action::Allow);
    let abs = p.join("README.md");
    let (input, _) = gate(
        p,
        &sandboxed,
        "read_file",
        json!({"file_path": abs.to_str().unwrap()}),
    );
    assert_eq!(input["file_path"], json!("README.md"));
    let (input, _) = gate(p, &sandboxed, "list_dir", json!({"path": " ./ "}));
    assert_eq!(input["path"], json!("."));
}

#[test]
fn links_are_resolved_before_the_gate() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir(p.join(".git")).unwrap();
    std::fs::write(p.join(".git/config"), "[core]\n").unwrap();
    dir_link(&p.join(".git"), &p.join("link"));
    let (input, action) = gate(
        p,
        &Policy::preset(Preset::Sandboxed),
        "write_file",
        json!({"file_path": "link/config", "content": "x"}),
    );
    assert_eq!(input["file_path"], json!(".git/config"));
    assert_eq!(action, Action::Deny);
}

#[test]
fn decoy_arguments_are_dropped_before_the_gate() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir(p.join("secrets")).unwrap();
    std::fs::write(p.join("secrets/key.txt"), "TOP secret\n").unwrap();
    let mut policy = Policy::preset(Preset::Sandboxed);
    // `secrets/**` does not match the directory itself, so the preset style pairs it with
    // the bare name (as it does for `.git`).
    for spec in ["Grep(secrets/**)", "Grep(secrets)"] {
        policy.rules.push(Rule::new(spec, Action::Deny));
    }
    for target in ["secrets", "secrets/key.txt"] {
        let (input, action) = gate(
            p,
            &policy,
            "grep",
            json!({"pattern": "TOP", "path": target, "file_path": "README.md"}),
        );
        assert_eq!(input, json!({"pattern": "TOP", "path": target}));
        assert_eq!(action, Action::Deny, "{target}");
    }
}

#[test]
fn prepare_keeps_only_schema_arguments_of_the_right_type() {
    let ws = workspace();
    let p = ws.path();
    let call = tools::prepare(
        "shell",
        &json!({"command": "git status", "description": "check the tree"}),
        p,
    )
    .unwrap();
    assert_eq!(call.input, json!({"command": "git status"}));
    let call = tools::prepare(
        "read_file",
        &json!({"file_path": "README.md", "offset": 2, "limit": 1, "path": "src"}),
        p,
    )
    .unwrap();
    assert_eq!(
        call.input,
        json!({"file_path": "README.md", "offset": 2, "limit": 1})
    );
    let err =
        tools::prepare("grep", &json!({"pattern": "x", "ignore_case": "yes"}), p).unwrap_err();
    assert!(err.contains("ignore_case"), "{err}");
    let err = tools::prepare(
        "read_file",
        &json!({"file_path": "README.md", "offset": -1}),
        p,
    )
    .unwrap_err();
    assert!(err.contains("offset"), "{err}");
    let err = tools::prepare("shell", &json!({"command": "x", "timeout": "5"}), p).unwrap_err();
    assert!(err.contains("timeout"), "{err}");
    let err = tools::prepare("glob", &json!({"pattern": "*", "path": 3}), p).unwrap_err();
    assert!(err.contains("'path'"), "{err}");
}

#[tokio::test]
async fn read_and_edit_refuse_files_over_the_size_limit() {
    let ws = workspace();
    let p = ws.path();
    let f = std::fs::File::create(p.join("huge.log")).unwrap();
    f.set_len(MAX_READ_BYTES + 1).unwrap();
    drop(f);
    let out = run(p, "read_file", json!({"file_path": "huge.log"})).await;
    assert!(out.is_error && out.output.contains("too large"), "{out:?}");
    let out = run(
        p,
        "edit_file",
        json!({"file_path": "huge.log", "old_string": "a", "new_string": "b"}),
    )
    .await;
    assert!(out.is_error && out.output.contains("too large"), "{out:?}");
}

#[tokio::test]
async fn read_file_returns_text_and_line_ranges() {
    let ws = workspace();
    let p = ws.path();
    assert_eq!(
        run(p, "read_file", json!({"file_path": "README.md"})).await,
        ToolOutput::ok("hello\nworld\n")
    );
    assert_eq!(
        run(
            p,
            "read_file",
            json!({"file_path": "README.md", "offset": 2, "limit": 1})
        )
        .await,
        ToolOutput::ok("world\n")
    );
    std::fs::write(p.join("bin.dat"), [0u8, 1, 2]).unwrap();
    let out = run(p, "read_file", json!({"file_path": "bin.dat"})).await;
    assert!(out.is_error && out.output.contains("binary"), "{out:?}");
    let out = run(p, "read_file", json!({"file_path": "src"})).await;
    assert!(out.is_error && out.output.contains("directory"), "{out:?}");
    let out = run(p, "read_file", json!({"file_path": "missing.txt"})).await;
    assert!(
        out.is_error && out.output.contains("cannot read"),
        "{out:?}"
    );
    std::fs::write(p.join("big.txt"), "y".repeat(MAX_OUTPUT * 2)).unwrap();
    let out = run(p, "read_file", json!({"file_path": "big.txt"})).await;
    assert!(out.output.len() < MAX_OUTPUT + 100 && out.output.contains("[truncated"));
}

#[tokio::test]
async fn write_and_edit_files() {
    let ws = workspace();
    let p = ws.path();
    let out = run(
        p,
        "write_file",
        json!({"file_path": "a/b/new.txt", "content": "one two"}),
    )
    .await;
    assert_eq!(out, ToolOutput::ok("wrote 7 bytes to a/b/new.txt"));
    assert_eq!(
        std::fs::read_to_string(p.join("a/b/new.txt")).unwrap(),
        "one two"
    );

    let edit = |file: &str, old: &str, new: &str| json!({"file_path": file, "old_string": old, "new_string": new});
    let out = run(p, "edit_file", edit("a/b/new.txt", "two", "2")).await;
    assert_eq!(out, ToolOutput::ok("edited a/b/new.txt"));
    assert_eq!(
        std::fs::read_to_string(p.join("a/b/new.txt")).unwrap(),
        "one 2"
    );
    let out = run(p, "edit_file", edit("a/b/new.txt", "zzz", "q")).await;
    assert!(out.is_error && out.output.contains("not found"), "{out:?}");
    std::fs::write(p.join("dup.txt"), "x x").unwrap();
    let out = run(p, "edit_file", edit("dup.txt", "x", "y")).await;
    assert!(out.is_error && out.output.contains("2 times"), "{out:?}");
    let out = run(p, "edit_file", edit("dup.txt", "", "y")).await;
    assert!(out.is_error, "{out:?}");
}

#[tokio::test]
async fn edit_file_matches_lf_text_in_crlf_files() {
    let ws = workspace();
    let p = ws.path();
    std::fs::write(p.join("crlf.txt"), "a\r\nb\r\nc\r\n").unwrap();
    let out = run(
        p,
        "edit_file",
        json!({"file_path": "crlf.txt", "old_string": "a\nb", "new_string": "A\nB"}),
    )
    .await;
    assert!(!out.is_error, "{out:?}");
    assert_eq!(
        std::fs::read_to_string(p.join("crlf.txt")).unwrap(),
        "A\r\nB\r\nc\r\n"
    );
}

#[tokio::test]
async fn list_dir_marks_directories_and_hides_git() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir(p.join(".git")).unwrap();
    assert_eq!(
        run(p, "list_dir", json!({})).await,
        ToolOutput::ok("README.md\nsrc/")
    );
    assert_eq!(
        run(p, "list_dir", json!({"path": "src"})).await,
        ToolOutput::ok("deep/\nmain.rs")
    );
    std::fs::create_dir(p.join("empty")).unwrap();
    assert_eq!(
        run(p, "list_dir", json!({"path": "empty"})).await,
        ToolOutput::ok("(empty directory)")
    );
}

#[tokio::test]
async fn glob_matches_relative_paths_and_honours_gitignore() {
    let ws = workspace();
    let p = ws.path();
    assert_eq!(
        run(p, "glob", json!({"pattern": "**/*.rs"})).await,
        ToolOutput::ok("src/deep/lib.rs\nsrc/main.rs")
    );
    assert_eq!(
        run(p, "glob", json!({"pattern": "*.md"})).await,
        ToolOutput::ok("README.md")
    );
    assert_eq!(
        run(p, "glob", json!({"pattern": "*.rs", "path": "src"})).await,
        ToolOutput::ok("src/main.rs")
    );
    std::fs::write(p.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(p.join("target")).unwrap();
    std::fs::write(p.join("target/gen.rs"), "").unwrap();
    let out = run(p, "glob", json!({"pattern": "**/*.rs"})).await;
    assert!(!out.output.contains("target"), "{out:?}");
    assert_eq!(
        run(p, "glob", json!({"pattern": "**/*.zip"})).await,
        ToolOutput::ok("no files match **/*.zip")
    );
}

#[tokio::test]
async fn grep_reports_path_line_and_text() {
    let ws = workspace();
    let p = ws.path();
    assert_eq!(
        run(p, "grep", json!({"pattern": "fn \\w+"})).await,
        ToolOutput::ok("src/deep/lib.rs:1: pub fn Hello() {}\nsrc/main.rs:1: fn main() {}")
    );
    assert_eq!(
        run(
            p,
            "grep",
            json!({"pattern": "hello", "ignore_case": true, "glob": "*.rs"})
        )
        .await,
        ToolOutput::ok("src/deep/lib.rs:1: pub fn Hello() {}")
    );
    assert_eq!(
        run(p, "grep", json!({"pattern": "world", "path": "README.md"})).await,
        ToolOutput::ok("README.md:2: world")
    );
    let out = run(p, "grep", json!({"pattern": "("})).await;
    assert!(
        out.is_error && out.output.contains("invalid regular expression"),
        "{out:?}"
    );
    assert_eq!(
        run(p, "grep", json!({"pattern": "nothing-here"})).await,
        ToolOutput::ok("no matches for nothing-here")
    );
}

#[cfg(windows)]
mod cmds {
    pub const ECHO: &str = "Write-Output 'hi there'";
    pub const FAIL: &str = "cmd /c exit 3";
    pub const SLEEP: &str = "Start-Sleep -Seconds 30";
    pub const BIG: &str = "'x' * 100000";
}
#[cfg(not(windows))]
mod cmds {
    pub const ECHO: &str = "echo 'hi there'";
    pub const FAIL: &str = "exit 3";
    pub const SLEEP: &str = "sleep 30";
    pub const BIG: &str = "head -c 100000 /dev/zero | tr '\\0' x";
}

#[tokio::test]
async fn shell_runs_in_the_workspace_and_reports_exit_codes() {
    let ws = workspace();
    let p = ws.path();
    let out = run(p, "shell", json!({"command": cmds::ECHO})).await;
    assert_eq!(out.output.trim(), "hi there");
    assert!(!out.is_error);
    let out = run(p, "shell", json!({"command": cmds::FAIL})).await;
    assert!(
        out.is_error && out.output.starts_with("exit code 3"),
        "{out:?}"
    );
    let out = run(p, "shell", json!({"command": cmds::BIG})).await;
    assert!(out.output.len() < MAX_OUTPUT + 100 && out.output.starts_with("[truncated"));
}

#[tokio::test]
async fn shell_stops_at_its_timeout_and_on_cancel() {
    let ws = workspace();
    let p = ws.path();
    let started = Instant::now();
    let out = run(p, "shell", json!({"command": cmds::SLEEP, "timeout": 1})).await;
    assert!(
        out.is_error && out.output.contains("timed out after 1s"),
        "{out:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(15));

    let call = tools::prepare("shell", &json!({"command": cmds::SLEEP}), p).unwrap();
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        c.cancel();
    });
    let started = Instant::now();
    let out = tools::run(&call, p, &cancel).await;
    assert_eq!(out, ToolOutput::err("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[cfg(windows)]
#[tokio::test]
async fn shell_reports_powershell_errors_as_plain_text() {
    let ws = workspace();
    let p = ws.path();
    let out = run(p, "shell", json!({"command": "Get-Item nope-missing"})).await;
    assert!(
        out.is_error && out.output.contains("Cannot find path"),
        "{out:?}"
    );
    for leak in ["CLIXML", "_x000D_", "OutputEncoding", "LASTEXITCODE"] {
        assert!(!out.output.contains(leak), "{leak}: {out:?}");
    }
    let out = run(p, "shell", json!({"command": "Write-Error boom"})).await;
    assert!(out.output.contains("boom"), "{out:?}");
    assert!(!out.output.contains("OutputEncoding"), "{out:?}");
}

#[tokio::test]
async fn shell_keeps_output_when_a_background_process_holds_the_pipes() {
    #[cfg(windows)]
    const HOLD: &str = "Write-Output built; Start-Process -NoNewWindow -FilePath ping.exe -ArgumentList '-n','30','127.0.0.1'";
    #[cfg(not(windows))]
    const HOLD: &str = "echo built; sleep 30 &";
    let ws = workspace();
    let started = Instant::now();
    let out = run(ws.path(), "shell", json!({"command": HOLD})).await;
    assert!(out.output.contains("built"), "{out:?}");
    const HELD: &str = "[output stream still held open by a background process]";
    #[cfg(windows)]
    {
        // The job ends the process, so the stream is not held for the 5 s wait. The two notes
        // prove that path; the time is only a guard against a hang, as wide as on the other
        // systems, because a tighter one failed under load.
        assert!(
            out.output.contains(
                "[a process the command left running in the background was stopped; nothing a command starts outlives it]"
            ),
            "{out:?}"
        );
        assert!(!out.output.contains(HELD), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(15), "{out:?}");
    }
    #[cfg(not(windows))]
    {
        assert!(out.output.contains(HELD), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(15));
    }
}

/// PowerShell that runs `fake-claude --orphan` and waits for it: when it is done, a grandchild
/// of the shell lingers and the process that started it has exited. `Start-Process` is used
/// because `&` would also wait for the grandchild, which inherits the pipes `&` reads from.
#[cfg(windows)]
fn orphan_script(lingerer: &common::Lingerer) -> String {
    format!(
        "$p = Start-Process -NoNewWindow -PassThru -FilePath '{}' -ArgumentList '--orphan','\"{}\"'; $p.WaitForExit()",
        env!("CARGO_BIN_EXE_fake-claude"),
        lingerer.dir()
    )
}

/// `taskkill /T` follows parent links, so it misses a process whose parent has exited.
#[cfg(windows)]
#[tokio::test]
async fn shell_cancel_kills_a_grandchild_whose_parent_has_exited() {
    let ws = workspace();
    let lingerer = common::Lingerer::new();
    // The file says that the grandchild's parent has exited.
    let script = format!(
        "{}; Set-Content -Path orphaned -Value 1; Start-Sleep -Seconds 30",
        orphan_script(&lingerer)
    );
    let call = tools::prepare("shell", &json!({ "command": script }), ws.path()).unwrap();
    let cancel = CancellationToken::new();
    let cancel_once_orphaned = async {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ws.path().join("orphaned").exists() {
            assert!(Instant::now() < deadline, "the script did not get that far");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(lingerer.alive(), "the grandchild should be running");
        cancel.cancel();
        Instant::now()
    };
    let (out, cancelled) =
        tokio::join!(tools::run(&call, ws.path(), &cancel), cancel_once_orphaned);
    assert_eq!(out, ToolOutput::err("cancelled"));
    // The grandchild holds the shell's output pipes; left alone it delays the answer by 5 s.
    assert!(
        cancelled.elapsed() < Duration::from_secs(3),
        "cancel waited for the grandchild"
    );
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after cancel"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn shell_leaves_no_process_behind_when_it_returns() {
    let ws = workspace();
    let lingerer = common::Lingerer::new();
    let script = format!("{}; Write-Output left", orphan_script(&lingerer));
    let out = run(ws.path(), "shell", json!({ "command": script })).await;
    assert!(!out.is_error && out.output.contains("left"), "{out:?}");
    // The grandchild did run: `--orphan` returns only once it has written `ready`. Without
    // this, a script that started nothing would pass the next check too.
    assert!(
        Path::new(&lingerer.dir()).join("ready").is_file(),
        "the grandchild never started: {out:?}"
    );
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after the call returned"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shell_timeout_kills_the_whole_process_group() {
    let ws = workspace();
    let started = Instant::now();
    let out = run(
        ws.path(),
        "shell",
        json!({"command": "sleep 30; echo done", "timeout": 1}),
    )
    .await;
    assert!(
        out.is_error && out.output.contains("timed out after 1s"),
        "{out:?}"
    );
    assert!(!out.output.contains("done"), "{out:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// Runs `name` with the walking tools filtered by `policy`, as the local engine does.
async fn run_filtered(ws: &Path, policy: &Policy, name: &str, args: Value) -> ToolOutput {
    let call = tools::prepare(name, &args, ws).unwrap_or_else(|e| panic!("prepare: {e}"));
    let filter = tools::PathFilter::from_policy(policy, call.alias, ws);
    tools::run_with(&call, ws, &CancellationToken::new(), &filter).await
}

#[tokio::test]
async fn walking_tools_leave_out_what_the_policy_denies() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("secret/inner")).unwrap();
    std::fs::write(p.join("secret/key.txt"), "hello key\n").unwrap();
    std::fs::write(p.join("secret/inner/more.txt"), "hello more\n").unwrap();
    std::fs::write(p.join("notes.txt"), "hello notes\n").unwrap();
    let policy = Policy {
        preset: Preset::Custom,
        rules: vec![Rule::new("Read(secret/**)", Action::Deny)],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };

    let out = run_filtered(p, &policy, "grep", json!({"pattern": "hello"})).await;
    assert!(!out.is_error, "{}", out.output);
    assert!(out.output.contains("README.md:1: hello"), "{}", out.output);
    assert!(
        out.output.contains("notes.txt:1: hello notes"),
        "{}",
        out.output
    );
    assert!(!out.output.contains("secret"), "{}", out.output);
    assert!(
        out.output.ends_with("(1 entry hidden by policy)"),
        "{}",
        out.output
    );

    let out = run_filtered(p, &policy, "glob", json!({"pattern": "**/*.txt"})).await;
    assert_eq!(out.output, "notes.txt\n(1 entry hidden by policy)");

    let out = run_filtered(p, &policy, "list_dir", json!({})).await;
    assert!(!out.output.contains("secret"), "{}", out.output);
    assert!(
        out.output.ends_with("(1 entry hidden by policy)"),
        "{}",
        out.output
    );

    // The note counts what was left out, even entries the pattern would not have matched.
    let out = run_filtered(p, &policy, "glob", json!({"pattern": "*.md"})).await;
    assert_eq!(out.output, "README.md\n(1 entry hidden by policy)");
}

#[tokio::test]
async fn walking_tools_without_a_filter_hide_nothing() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("secret")).unwrap();
    std::fs::write(p.join("secret/key.txt"), "hello key\n").unwrap();
    let out = run(p, "grep", json!({"pattern": "hello key"})).await;
    assert_eq!(out.output, "secret/key.txt:1: hello key");
}

/// A custom policy that allows the walking tools everywhere, plus `rules`.
fn walking_policy(rules: Vec<Rule>, default: Action) -> Policy {
    let mut all: Vec<Rule> = ["LS", "Glob", "Grep"]
        .iter()
        .map(|t| Rule::new(t, Action::Allow))
        .collect();
    all.extend(rules);
    Policy {
        preset: Preset::Custom,
        rules: all,
        default,
        mcp_default: Action::Ask,
    }
}

#[tokio::test]
async fn walking_tools_reach_an_allowed_subtree_under_default_deny() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("other/inner")).unwrap();
    std::fs::write(p.join("other/a.txt"), "fn other\n").unwrap();
    std::fs::write(p.join("other/inner/b.txt"), "fn inner\n").unwrap();
    let policy = walking_policy(vec![Rule::new("Read(src/**)", Action::Allow)], Action::Deny);

    // `src`, `other` and `other/inner` are denied by the default only: they are entered and not
    // counted. The three files outside `src` are denied by the default: each is hidden.
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "fn"})).await;
    assert_eq!(
        out.output,
        "src/deep/lib.rs:1: pub fn Hello() {}\nsrc/main.rs:1: fn main() {}\n\
         (3 entries hidden by policy)"
    );

    let out = run_filtered(p, &policy, "glob", json!({"pattern": "**/*"})).await;
    assert_eq!(
        out.output,
        "src/deep/lib.rs\nsrc/main.rs\n(3 entries hidden by policy)"
    );

    let out = run_filtered(p, &policy, "list_dir", json!({})).await;
    assert_eq!(out.output, "other/\nsrc/\n(1 entry hidden by policy)");
    let out = run_filtered(p, &policy, "list_dir", json!({"path": "other"})).await;
    assert_eq!(out.output, "inner/\n(1 entry hidden by policy)");
    let out = run_filtered(p, &policy, "list_dir", json!({"path": "src"})).await;
    assert_eq!(out.output, "deep/\nmain.rs");
}

#[tokio::test]
async fn a_directory_matched_by_a_deny_rule_is_not_entered() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("secret/inner")).unwrap();
    std::fs::write(p.join("secret/key.txt"), "hello key\n").unwrap();
    std::fs::write(p.join("secret/inner/more.txt"), "hello more\n").unwrap();
    // `**/secret` names the directory and nothing inside it, and `Read` is allowed: the two
    // files would be found if the walk entered `secret`.
    for &default in Action::ALL {
        let policy = walking_policy(
            vec![
                Rule::new("Read", Action::Allow),
                Rule::new("Read(**/secret)", Action::Deny),
            ],
            default,
        );
        for file in ["secret/key.txt", "secret/inner/more.txt"] {
            let input = json!({ "file_path": file });
            assert_eq!(policy.resolve_in("Read", &input, Some(p)), Action::Allow);
        }
        let out = run_filtered(p, &policy, "grep", json!({"pattern": "hello"})).await;
        assert_eq!(
            out.output, "README.md:1: hello\n(1 entry hidden by policy)",
            "{default:?}"
        );
        let out = run_filtered(p, &policy, "glob", json!({"pattern": "**/*.txt"})).await;
        assert_eq!(
            out.output, "no files match **/*.txt\n(1 entry hidden by policy)",
            "{default:?}"
        );
        let out = run_filtered(p, &policy, "list_dir", json!({})).await;
        assert_eq!(
            out.output, "README.md\nsrc/\n(1 entry hidden by policy)",
            "{default:?}"
        );
    }
}

#[tokio::test]
async fn the_hidden_count_is_one_per_denied_directory_and_one_per_hidden_file() {
    let ws = workspace();
    let p = ws.path();
    for dir in ["src/secret", "other"] {
        std::fs::create_dir_all(p.join(dir)).unwrap();
        std::fs::write(p.join(dir).join("a.txt"), "fn a\n").unwrap();
        std::fs::write(p.join(dir).join("b.txt"), "fn b\n").unwrap();
    }
    std::fs::write(p.join("src/deep/key.pem"), "fn key\n").unwrap();
    let policy = walking_policy(
        vec![
            Rule::new("Read(src/**)", Action::Allow),
            Rule::new("Read(secret/**)", Action::Deny),
            Rule::new("Read(**/*.pem)", Action::Deny),
        ],
        Action::Deny,
    );
    // README.md (default): 1. `other` is entered, its two files (default): 2. `src/secret` (deny
    // rule, with two files inside): 1. src/deep/key.pem (deny rule): 1.
    let found = "src/deep/lib.rs:1: pub fn Hello() {}\nsrc/main.rs:1: fn main() {}";
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "fn"})).await;
    assert_eq!(out.output, format!("{found}\n(5 entries hidden by policy)"));
    // Searching inside `src` counts what is hidden there only.
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "fn", "path": "src"})).await;
    assert_eq!(out.output, format!("{found}\n(2 entries hidden by policy)"));
    let out = run_filtered(p, &policy, "list_dir", json!({"path": "src"})).await;
    assert_eq!(out.output, "deep/\nmain.rs\n(1 entry hidden by policy)");
}

#[tokio::test]
async fn an_entry_that_resolves_to_ask_is_not_skipped() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir(p.join("drafts")).unwrap();
    std::fs::write(p.join("drafts/a.txt"), "hello draft\n").unwrap();
    let policy = walking_policy(
        vec![Rule::new("Read(drafts/**)", Action::Ask)],
        Action::Deny,
    );
    // README.md and the two files under `src` are denied by the default.
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "hello"})).await;
    assert_eq!(
        out.output,
        "drafts/a.txt:1: hello draft\n(3 entries hidden by policy)"
    );
    let out = run_filtered(p, &policy, "list_dir", json!({"path": "drafts"})).await;
    assert_eq!(out.output, "a.txt");

    // Under default ask nothing is denied, so nothing is hidden.
    let policy = walking_policy(vec![], Action::Ask);
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "hello"})).await;
    assert_eq!(
        out.output,
        "README.md:1: hello\ndrafts/a.txt:1: hello draft"
    );
    let out = run_filtered(p, &policy, "list_dir", json!({})).await;
    assert_eq!(out.output, "README.md\ndrafts/\nsrc/");
}

#[tokio::test]
async fn a_link_to_a_directory_is_judged_as_a_file() {
    let ws = workspace();
    let p = ws.path();
    dir_link(&p.join("src"), &p.join("jump"));
    let policy = walking_policy(vec![Rule::new("Read(src/**)", Action::Allow)], Action::Deny);
    // The link is not a directory to enter: the default denies it like any file. README.md is
    // the other hidden entry.
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "fn"})).await;
    assert_eq!(
        out.output,
        "src/deep/lib.rs:1: pub fn Hello() {}\nsrc/main.rs:1: fn main() {}\n\
         (2 entries hidden by policy)"
    );
    let out = run_filtered(p, &policy, "list_dir", json!({})).await;
    assert_eq!(out.output, "src/\n(2 entries hidden by policy)");
}

#[tokio::test]
async fn read_only_and_sandboxed_walks_skip_git_and_show_dot_claude() {
    let ws = workspace();
    let p = ws.path();
    for dir in [".git", ".claude"] {
        std::fs::create_dir(p.join(dir)).unwrap();
    }
    std::fs::write(p.join(".git/config"), "hello git\n").unwrap();
    std::fs::write(p.join(".claude/settings.json"), "hello claude\n").unwrap();
    for preset in [Preset::ReadOnly, Preset::Sandboxed] {
        let policy = Policy::preset(preset);
        // `.git` is never walked and is not counted; no read rule denies `.claude`.
        let out = run_filtered(p, &policy, "grep", json!({"pattern": "hello"})).await;
        assert_eq!(
            out.output, ".claude/settings.json:1: hello claude\nREADME.md:1: hello",
            "{preset:?}"
        );
        let out = run_filtered(p, &policy, "glob", json!({"pattern": "**/*"})).await;
        assert_eq!(
            out.output, ".claude/settings.json\nREADME.md\nsrc/deep/lib.rs\nsrc/main.rs",
            "{preset:?}"
        );
        let out = run_filtered(p, &policy, "list_dir", json!({})).await;
        assert_eq!(out.output, ".claude/\nREADME.md\nsrc/", "{preset:?}");
    }
}

/// The error a walking tool gives in place of a result when the policy denies its search root.
fn root_denied(path: &str) -> ToolOutput {
    ToolOutput::err(format!("{path} is denied by this dot's policy"))
}

#[tokio::test]
async fn grep_refuses_a_file_root_the_policy_denies() {
    let ws = workspace();
    let p = ws.path();
    std::fs::write(p.join(".env"), "PASSWORD=hunter2\n").unwrap();
    let dot_env = json!({"pattern": ".", "path": ".env"});

    // A deny rule on `Read` covers the file; `Grep` itself is allowed, as in every preset.
    let mut read_only = Policy::preset(Preset::ReadOnly);
    read_only
        .rules
        .push(Rule::new("Read(**/.env)", Action::Deny));
    let by_rule = walking_policy(
        vec![
            Rule::new("Read", Action::Allow),
            Rule::new("Read(**/.env)", Action::Deny),
        ],
        Action::Allow,
    );
    for policy in [&read_only, &by_rule] {
        assert_eq!(
            policy.resolve_in("Grep", &dot_env, Some(p)),
            Action::Allow,
            "the gate lets the call through"
        );
        let out = run_filtered(p, policy, "grep", dot_env.clone()).await;
        assert_eq!(out, root_denied(".env"));
    }

    // A file is hidden whenever it resolves to `deny`, so the default is enough.
    let by_default = walking_policy(vec![Rule::new("Read(src/**)", Action::Allow)], Action::Deny);
    let out = run_filtered(p, &by_default, "grep", dot_env.clone()).await;
    assert_eq!(out, root_denied(".env"));
    let out = run_filtered(
        p,
        &by_default,
        "grep",
        json!({"pattern": "hello", "path": "README.md"}),
    )
    .await;
    assert_eq!(out, root_denied("README.md"));
    // A root that does not exist is not a directory: it is judged as a file, like `read_file`
    // of that path at the permission gate.
    let out = run_filtered(
        p,
        &by_default,
        "grep",
        json!({"pattern": "x", "path": "nothing-here"}),
    )
    .await;
    assert_eq!(out, root_denied("nothing-here"));

    // A file root that is allowed is searched as before, under each of the three policies.
    for policy in [&read_only, &by_rule, &by_default] {
        let out = run_filtered(
            p,
            policy,
            "grep",
            json!({"pattern": "fn", "path": "src/main.rs"}),
        )
        .await;
        assert_eq!(out, ToolOutput::ok("src/main.rs:1: fn main() {}"));
    }
}

/// A call of each walking tool with `path` as its search root.
fn rooted_calls(path: &str) -> [(&'static str, Value); 3] {
    [
        ("grep", json!({"pattern": "hello", "path": path})),
        ("glob", json!({"pattern": "**/*", "path": path})),
        ("list_dir", json!({"path": path})),
    ]
}

#[tokio::test]
async fn walking_tools_refuse_a_directory_root_a_deny_rule_matches() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("secret/inner")).unwrap();
    std::fs::write(p.join("secret/key.txt"), "hello key\n").unwrap();
    std::fs::write(p.join("secret/inner/more.txt"), "hello more\n").unwrap();

    // `**/secret` names the directory only (its files resolve to allow under `Read`);
    // `secret/**` names the directory and everything in it. Neither may be the root of a walk.
    for rule in ["Read(**/secret)", "Read(secret/**)"] {
        for &default in Action::ALL {
            let policy = walking_policy(
                vec![
                    Rule::new("Read", Action::Allow),
                    Rule::new(rule, Action::Deny),
                ],
                default,
            );
            for (tool, args) in rooted_calls("secret") {
                let out = run_filtered(p, &policy, tool, args).await;
                // The whole answer: no listing, no match and no count of hidden entries.
                assert_eq!(out, root_denied("secret"), "{tool} {rule} {default:?}");
            }
        }
    }

    // `secret/**` also covers a directory deeper inside.
    let policy = walking_policy(
        vec![
            Rule::new("Read", Action::Allow),
            Rule::new("Read(secret/**)", Action::Deny),
        ],
        Action::Allow,
    );
    for (tool, args) in rooted_calls("secret/inner") {
        let out = run_filtered(p, &policy, tool, args).await;
        assert_eq!(out, root_denied("secret/inner"), "{tool}");
    }

    // A deny rule under the walking tool's own name, with `Read` allowed everywhere. The gate
    // refuses this call before the tool runs; the tool refuses it by itself too.
    for (tool, args) in rooted_calls("secret") {
        let alias = tools::alias(tool).unwrap();
        let policy = Policy {
            preset: Preset::Custom,
            rules: vec![Rule::new(&format!("{alias}(secret/**)"), Action::Deny)],
            default: Action::Allow,
            mcp_default: Action::Ask,
        };
        assert_eq!(
            policy.resolve_in("Read", &json!({"file_path": "secret/key.txt"}), Some(p)),
            Action::Allow
        );
        let out = run_filtered(p, &policy, tool, args).await;
        assert_eq!(out, root_denied("secret"), "{tool}");
    }
}

#[tokio::test]
async fn walking_tools_refuse_the_workspace_root_when_a_deny_rule_matches_it() {
    let ws = workspace();
    let p = ws.path();
    // A bare `Read` deny rule matches every path, the workspace root included.
    let policy = walking_policy(vec![Rule::new("Read", Action::Deny)], Action::Allow);
    let calls = [
        ("grep", json!({"pattern": "hello"})),
        ("glob", json!({"pattern": "**/*"})),
        ("list_dir", json!({})),
    ];
    for (tool, args) in calls.into_iter().chain(rooted_calls(".")) {
        let out = run_filtered(p, &policy, tool, args).await;
        assert_eq!(out, root_denied("the workspace root"), "{tool}");
    }
}

#[tokio::test]
async fn a_directory_root_that_only_the_default_denies_is_still_entered() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("other/inner")).unwrap();
    std::fs::write(p.join("other/a.txt"), "hello other\n").unwrap();
    std::fs::write(p.join("other/inner/b.txt"), "hello inner\n").unwrap();
    // No rule names `other`: the default denies it, and the allowed subtree lies inside it.
    let policy = walking_policy(
        vec![Rule::new("Read(other/inner/**)", Action::Allow)],
        Action::Deny,
    );
    assert_eq!(
        policy.resolve_in("Read", &json!({"file_path": "other"}), Some(p)),
        Action::Deny
    );
    let [grep, glob, list_dir] = rooted_calls("other");
    let out = run_filtered(p, &policy, grep.0, grep.1).await;
    assert_eq!(
        out,
        ToolOutput::ok("other/inner/b.txt:1: hello inner\n(1 entry hidden by policy)")
    );
    let out = run_filtered(p, &policy, glob.0, glob.1).await;
    assert_eq!(
        out,
        ToolOutput::ok("other/inner/b.txt\n(1 entry hidden by policy)")
    );
    let out = run_filtered(p, &policy, list_dir.0, list_dir.1).await;
    assert_eq!(out, ToolOutput::ok("inner/\n(1 entry hidden by policy)"));
}

#[tokio::test]
async fn a_deny_rule_elsewhere_leaves_the_search_root_alone() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir_all(p.join("secret")).unwrap();
    std::fs::write(p.join("secret/key.txt"), "hello key\n").unwrap();
    let policy = Policy {
        preset: Preset::Custom,
        rules: vec![Rule::new("Read(secret/**)", Action::Deny)],
        default: Action::Allow,
        mcp_default: Action::Ask,
    };
    // The workspace root, named or left out, is walked as before: `secret` is the hidden entry.
    for args in [
        json!({"pattern": "hello"}),
        json!({"pattern": "hello", "path": "."}),
    ] {
        let out = run_filtered(p, &policy, "grep", args).await;
        assert_eq!(
            out,
            ToolOutput::ok("README.md:1: hello\n(1 entry hidden by policy)")
        );
    }
    let out = run_filtered(p, &policy, "glob", json!({"pattern": "*", "path": "."})).await;
    assert_eq!(out, ToolOutput::ok("README.md\n(1 entry hidden by policy)"));
    let out = run_filtered(p, &policy, "list_dir", json!({"path": "."})).await;
    assert_eq!(
        out,
        ToolOutput::ok("README.md\nsrc/\n(1 entry hidden by policy)")
    );
    // A root the rule does not name is walked with nothing hidden.
    let out = run_filtered(p, &policy, "grep", json!({"pattern": "fn", "path": "src"})).await;
    assert_eq!(
        out,
        ToolOutput::ok("src/deep/lib.rs:1: pub fn Hello() {}\nsrc/main.rs:1: fn main() {}")
    );
    let out = run_filtered(
        p,
        &policy,
        "glob",
        json!({"pattern": "**/*", "path": "src"}),
    )
    .await;
    assert_eq!(out, ToolOutput::ok("src/deep/lib.rs\nsrc/main.rs"));
    let out = run_filtered(p, &policy, "list_dir", json!({"path": "src"})).await;
    assert_eq!(out, ToolOutput::ok("deep/\nmain.rs"));
}

#[tokio::test]
async fn shell_runs_scripts_longer_than_the_command_line_limit() {
    let ws = workspace();
    let mut script = String::new();
    for i in 0..800 {
        script.push_str(&format!(
            "# padding line {i:04} ..........................................\n"
        ));
    }
    script.push_str(if cfg!(windows) {
        "Write-Output 'done: héllo ✓'"
    } else {
        "echo 'done: héllo ✓'"
    });
    assert!(script.len() > 40_000);
    let out = run(ws.path(), "shell", json!({ "command": script })).await;
    assert!(!out.is_error, "{out:?}");
    assert_eq!(out.output.trim(), "done: héllo ✓");
}

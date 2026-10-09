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
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len()
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
    assert!(
        out.output
            .contains("[output stream still held open by a background process]"),
        "{out:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(15));
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

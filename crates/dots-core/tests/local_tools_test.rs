use std::path::Path;
use std::time::{Duration, Instant};

use dots_core::engine::local::tools::{self, ToolOutput, MAX_OUTPUT};
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

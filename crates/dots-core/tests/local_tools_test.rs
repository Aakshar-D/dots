use std::path::Path;

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

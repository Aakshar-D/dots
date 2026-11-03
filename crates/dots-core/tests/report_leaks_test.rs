//! The leaks a security review found in the problem report, each with a log line shaped like
//! the ones Dots writes.

mod common;

use dots_core::report::{read_log_tail, scrub_line, LogSection, Scrubber};

fn head(level: &str, target: &str) -> String {
    format!("2026-11-03T10:00:00.123456Z  {level} {target}:")
}

fn line(level: &str, target: &str, message: &str) -> String {
    format!("{} {message}", head(level, target))
}

fn tail_of(lines: &[String], scrubber: &Scrubber) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(dir.path().join("dots.2026-11-03.log"), text).unwrap();
    match read_log_tail(dir.path(), 200, scrubber) {
        LogSection::Lines { lines, .. } => lines,
        other => panic!("expected lines, got {other:?}"),
    }
}

fn plain() -> Scrubber {
    Scrubber::new(&[], None)
}

#[test]
fn dot_names_are_hidden_longest_first_in_any_case() {
    let names = vec![
        "Acme".to_string(),
        "acme-payroll-sync".to_string(),
        "x".to_string(),
    ];
    let s = Scrubber::new(&names, None);
    let out = s.line(&line(
        "INFO",
        "dots_core::scheduler",
        "missed slot skipped dot=Acme-Payroll-Sync slot=2026-11-03T09:00:00Z",
    ));
    assert!(
        out.ends_with("missed slot skipped dot=[dot] slot=2026-11-03T09:00:00Z"),
        "{out}"
    );
    let out = s.line(&line(
        "INFO",
        "dots_core::scheduler",
        "fired dot=acme run=01ABC",
    ));
    assert!(out.ends_with("fired dot=[dot] run=01ABC"), "{out}");
    // Only whole words: "x" does not eat the x in "next".
    let out = s.line(&line("INFO", "dots_core::scheduler", "next slot computed"));
    assert!(out.ends_with("next slot computed"), "{out}");
}

#[test]
fn every_kind_of_path_becomes_path_with_its_extension() {
    let cases = [
        (
            r"workspace cleanup failed: could not remove C:\Users\Asha\My Projects\client-x\src\main.rs: access denied",
            "workspace cleanup failed: could not remove [path].rs: access denied",
        ),
        (
            r"opened \\fileserver\share\clients\acme.docx for the run",
            "opened [path].docx for the run",
        ),
        (
            "reading /home/asha/work/thing.py failed",
            "reading [path].py failed",
        ),
        ("reading /Users/asha/notes failed", "reading [path] failed"),
        (
            r"temp at C:\Users\ASHAKU~1\AppData\Local\Temp\x.tmp",
            "temp at [path].tmp",
        ),
        (
            r#"error: Os { path: "C:\\Users\\Asha\\proj\\x.txt" }"#,
            r#"error: Os { path: "[path].txt" }"#,
        ),
        (
            "worktree D:/dev/client/repo created",
            "worktree [path] created",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(scrub_line(text, None), expected, "{text}");
    }
    // A URL is not a path.
    assert_eq!(
        scrub_line("GET https://api.example.com/v1/models", None),
        "GET https://api.example.com/v1/models"
    );
}

#[test]
fn a_private_key_block_is_dropped_to_its_end() {
    let lines = vec![
        line(
            "WARN",
            "dots_core::vault",
            "could not read key: -----BEGIN OPENSSH PRIVATE KEY-----",
        ),
        "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW".to_string(),
        "QyNTUxOQAAACDx1fLc2hK8cZr1qdGJk".to_string(),
        "-----END OPENSSH PRIVATE KEY-----".to_string(),
        line("INFO", "dots_core::runner", "next line"),
    ];
    let out = tail_of(&lines, &plain());
    let all = out.join("\n");
    assert!(!all.contains("b3BlbnNz"), "{all}");
    assert!(!all.contains("QyNTUxOQ"), "{all}");
    assert!(!all.contains("-----END"), "{all}");
    assert_eq!(out.len(), 2, "{all}");
    assert!(out[1].ends_with("next line"));
}

#[test]
fn more_secret_shapes_are_scrubbed() {
    let cases = [
        (
            "token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl back",
            "eyJzdWIi",
        ),
        (
            "cloning https://asha:hunter2@git.example.com/repo.git",
            "hunter2",
        ),
        (
            "cloning https://asha:hunter2@git.example.com/repo.git",
            "asha",
        ),
        (
            "GET https://x.blob.core.windows.net/c/f?sv=2024&sig=abcDEF123%2B&se=2026",
            "abcDEF123",
        ),
        ("callback /auth?code=4/0AbCdEf&state=xyz", "0AbCdEf"),
        ("url https://h/x?access_token=ya29.abc123", "ya29.abc123"),
        (
            "url https://h/x?key=AIzaSyA12345&pw=letmein",
            "AIzaSyA12345",
        ),
        ("url https://h/x?key=AIzaSyA12345&pw=letmein", "letmein"),
        ("aws ASIAABCDEFGHIJKLMNOP in use", "ASIAABCDEFGHIJKLMNOP"),
        ("key sk-ant-api03-AbCdEfGhIjKlMnOpQrSt", "AbCdEfGhIjKlMnOp"),
        (concat!("slack xox", "a-2-12345678901-abcdefghij"), "abcdefghij"),
        (concat!("slack xox", "r-12345678901-abcdefghijkl"), "abcdefghijkl"),
        (
            "app ghs_abcdefghijklmnopqrstuvwxyz0123456789",
            "ghs_abcdefgh",
        ),
        (
            "signing key 0123456789abcdef0123456789abcdef in use",
            "0123456789abcdef",
        ),
        (
            "secret ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123 set",
            "ABCDEFGHIJKLMNOP",
        ),
        ("mail to asha@localhost failed", "asha"),
    ];
    for (text, secret) in cases {
        let out = scrub_line(text, None);
        assert!(!out.contains(secret), "{secret} survived: {out}");
    }
    assert_eq!(
        scrub_line(
            "cloning https://asha:hunter2@git.example.com/repo.git",
            None
        ),
        "cloning https://[secret]@git.example.com/repo.git"
    );
}

#[test]
fn messages_that_carry_engine_or_tool_text_are_hidden() {
    let hidden = [
        line(
            "WARN",
            "dots_core::engine::claude",
            "claude: plugin acme-tools failed: secret plans run=01ABC",
        ),
        line(
            "DEBUG",
            "dots_core::engine::claude::parse",
            r#"unreadable rate_limit_event: {"note":"the user said hi"}"#,
        ),
        line(
            "WARN",
            "dots_core::runner",
            "run failed: claude exited 1: stderr: I read your file notes.md run=01ABC",
        ),
        line(
            "WARN",
            "dots_core::followups",
            "the follow-up's run could not be started: chat text here follow_up=01F attempt=1",
        ),
        line(
            "WARN",
            "dots_core::library_service",
            "plugin acme: the hook says client secrets",
        ),
        line(
            "DEBUG",
            "dots_core::mcp::http",
            "replying to the server failed: body was private server=files",
        ),
        line(
            "WARN",
            "dots_core::browser::host",
            "could not move the download quarterly-board-minutes.pdf: denied",
        ),
    ];
    let out = tail_of(&hidden, &plain());
    assert_eq!(out.len(), hidden.len());
    for (line, raw) in out.iter().zip(&hidden) {
        let (h, _) = raw.split_at(raw.find(": ").unwrap() + 1);
        assert_eq!(line, &format!("{h} [message hidden]"), "{raw}");
    }
    // Other runner lines stay.
    let kept = line("INFO", "dots_core::runner", "run started run=01ABC");
    assert_eq!(tail_of(&[kept.clone()], &plain()), vec![kept]);
}

#[test]
fn a_panic_keeps_its_first_line_only() {
    let lines = vec![
        line(
            "ERROR",
            "dots_app",
            "thread 'main' panicked at src\\x.rs:1:2: called unwrap on asha's text",
        ),
        "the dot said: please email the board".to_string(),
        "and more".to_string(),
        line("INFO", "dots_app", "starting Dots 0.3.0"),
    ];
    let out = tail_of(&lines, &plain());
    assert_eq!(out.len(), 3, "{out:?}");
    assert!(out[0].contains("panicked at"));
    assert_eq!(out[1], "[2 more lines hidden]");
    assert!(out[2].ends_with("starting Dots 0.3.0"));
}

#[tokio::test]
async fn the_report_hides_the_names_of_the_dots_in_the_store() {
    let (_d, store) = common::temp_store().await;
    store
        .create_dot(&common::spec("payroll-sync"), None, "default")
        .await
        .unwrap();
    assert_eq!(
        store.dot_names().await.unwrap(),
        vec!["payroll-sync".to_string()]
    );
    let logs = tempfile::tempdir().unwrap();
    std::fs::write(
        logs.path().join("dots.2026-11-03.log"),
        line(
            "INFO",
            "dots_core::scheduler",
            "fired dot=payroll-sync run=01ABC\n",
        ),
    )
    .unwrap();
    let text = dots_core::report::build(&store, "0.3.0", Some(logs.path()), "", true)
        .await
        .unwrap();
    assert!(!text.to_lowercase().contains("payroll"), "{text}");
    assert!(text.contains("fired dot=[dot] run=01ABC"), "{text}");
}

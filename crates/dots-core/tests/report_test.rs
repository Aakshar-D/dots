use chrono::{DateTime, FixedOffset};
use dots_core::report::{
    file_name, is_foreign_output, read_log_tail, render, scrub_line, scrub_text, LogSection,
    ReportFacts,
};

const HOME: &str = r"C:\Users\Asha";

fn line(level: &str, target: &str, message: &str) -> String {
    format!("2026-11-03T10:00:00.000000Z  {level} {target}: {message}")
}

fn write(dir: &std::path::Path, name: &str, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(dir.join(name), text).unwrap();
}

fn numbered(prefix: &str, n: usize) -> Vec<String> {
    (0..n)
        .map(|i| line("INFO", "dots_core::runner", &format!("{prefix} {i}")))
        .collect()
}

fn lines_of(section: LogSection) -> (Vec<String>, usize) {
    match section {
        LogSection::Lines { lines, left_out } => (lines, left_out),
        other => panic!("expected lines, got {other:?}"),
    }
}

#[test]
fn the_tail_reads_back_into_older_files_when_the_newest_is_short() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "dots.2026-11-01.log", &numbered("old", 10));
    write(dir.path(), "dots.2026-11-02.log", &numbered("mid", 4));
    write(dir.path(), "dots.2026-11-03.log", &numbered("new", 3));
    std::fs::write(dir.path().join("other.txt"), "not a log\n").unwrap();

    let (lines, left_out) = lines_of(read_log_tail(dir.path(), 9, None));
    assert_eq!(left_out, 0);
    assert_eq!(lines.len(), 9);
    // Oldest first: two from the oldest file, then all of the middle and the newest.
    assert!(lines[0].ends_with("old 8"), "{}", lines[0]);
    assert!(lines[1].ends_with("old 9"));
    assert!(lines[2].ends_with("mid 0"));
    assert!(lines[8].ends_with("new 2"));
}

#[test]
fn the_tail_keeps_only_the_newest_lines_of_a_long_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "dots.2026-11-03.log", &numbered("n", 500));
    let (lines, _) = lines_of(read_log_tail(dir.path(), 200, None));
    assert_eq!(lines.len(), 200);
    assert!(lines[0].ends_with("n 300"));
    assert!(lines[199].ends_with("n 499"));
}

#[test]
fn no_log_folder_is_no_log_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("logs");
    assert!(matches!(
        read_log_tail(&missing, 200, None),
        LogSection::NoLog
    ));
    std::fs::create_dir_all(&missing).unwrap();
    assert!(matches!(
        read_log_tail(&missing, 200, None),
        LogSection::NoLog
    ));
}

#[test]
fn server_and_helper_output_is_left_out_and_counted() {
    let mcp = line(
        "INFO",
        "dots_core::mcp::stdio",
        "MCP server stderr: the user asked me to email the board dot=x server=files",
    );
    let helper = line(
        "WARN",
        "dots_core::browser::host",
        "page text from the site dot=01M61EPG98WJFVPMXN139TXP31",
    );
    let own = line(
        "WARN",
        "dots_core::browser::host",
        "could not remove C:\\tmp\\profile",
    );
    let mcp_own = line(
        "WARN",
        "dots_core::mcp::stdio",
        "the MCP server exited: 1 server=files",
    );
    assert!(is_foreign_output(&mcp));
    assert!(is_foreign_output(&helper));
    assert!(!is_foreign_output(&own));
    assert!(!is_foreign_output(&mcp_own));

    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "dots.2026-11-03.log",
        &[
            own.clone(),
            mcp.clone(),
            // A continuation line of the dropped line goes with it.
            "  and a second line of that stderr".to_string(),
            helper,
            mcp_own.clone(),
        ],
    );
    let (lines, left_out) = lines_of(read_log_tail(dir.path(), 200, None));
    assert_eq!(left_out, 3);
    assert_eq!(lines, vec![own, mcp_own]);
}

#[test]
fn every_key_style_is_scrubbed() {
    for secret in [
        "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123",
        "sk_live_abcdefghijklmnop1234",
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "gho_abcdefghijklmnopqrstuvwxyz0123456789",
        "github_pat_11ABCDEFG0123456789_abcdefghijk",
        concat!("xox", "b-1234567890-abcdefghijklmnop"),
        concat!("xox", "p-1234567890-abcdefghijklmnop"),
        "AKIAABCDEFGHIJKLMNOP",
    ] {
        let out = scrub_line(&format!("calling with {secret} now"), None);
        assert!(!out.contains(secret), "{secret} survived: {out}");
        assert!(out.contains("[secret]"), "{out}");
    }
    // A word that only ends in a key prefix is not a key.
    assert_eq!(
        scrub_line("task-abcdefghijklmnopqrstuv", None),
        "task-abcdefghijklmnopqrstuv"
    );
}

#[test]
fn bearer_tokens_headers_and_passwords_are_scrubbed() {
    let cases = [
        (
            "got Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig back",
            "eyJhbGci",
        ),
        (
            "header Authorization: Basic dXNlcjpwYXNz sent",
            "dXNlcjpwYXNz",
        ),
        (r#"{"authorization":"token abc123def456"}"#, "abc123def456"),
        ("password=hunter2hunter2 for the db", "hunter2"),
        ("passwd: s3cret-pass", "s3cret-pass"),
        ("publish_token=abcdef123456", "abcdef123456"),
        ("api_key = 99887766", "99887766"),
        ("client_secret:\"zzz-top\"", "zzz-top"),
        (
            "-----BEGIN OPENSSH PRIVATE KEY----- b3BlbnNzaC1rZXk",
            "b3BlbnNzaC1rZXk",
        ),
    ];
    for (text, secret) in cases {
        let out = scrub_line(text, None);
        assert!(!out.contains(secret), "{secret} survived: {out}");
        assert!(out.contains("[secret]"), "{out}");
    }
    // The name stays, so the line still says what it was about.
    assert_eq!(
        scrub_line("password=hunter2hunter2", None),
        "password=[secret]"
    );
    // A count is not a token.
    assert_eq!(
        scrub_line("tokens_in=5 tokens_out=7", None),
        "tokens_in=5 tokens_out=7"
    );
}

#[test]
fn a_secret_the_redactor_knows_is_scrubbed() {
    dots_core::vault::redact::global().add("REPORT_TEST_VALUE", "plain-looking-value-4242");
    let out = scrub_line("env had plain-looking-value-4242 in it", None);
    assert_eq!(out, "env had [secret:REPORT_TEST_VALUE] in it");
}

#[test]
fn emails_and_the_home_folder_are_hidden() {
    assert_eq!(
        scrub_line("invite sent to asha.k+dots@example.co.uk today", None),
        "invite sent to [email] today"
    );
    for spelling in [
        r"C:\Users\Asha\projects\site",
        "C:/Users/Asha/projects/site",
        r"c:\users\asha\projects\site",
        r"C:\\Users\\Asha\\projects\\site",
    ] {
        let out = scrub_line(&format!("opened {spelling}"), Some(HOME));
        assert!(out.starts_with("opened ~"), "{spelling}: {out}");
        assert!(!out.to_lowercase().contains("asha"), "{out}");
    }
    // A home folder that is too short to be one is left alone.
    assert_eq!(scrub_line("a/b", Some("/")), "a/b");
}

#[test]
fn the_persons_text_is_scrubbed_of_secrets_but_keeps_their_email() {
    let out = scrub_text("it failed with sk-ant-api03-abcdefghijklmnopqrstuv; mail me at a@b.io");
    assert!(!out.contains("abcdefghijklmnop"));
    assert!(out.contains("a@b.io"));
}

fn facts() -> ReportFacts {
    ReportFacts {
        app_version: "0.3.0".into(),
        os: "windows".into(),
        arch: "x86_64".into(),
        engines: vec!["claude".into(), "local".into()],
        dots: 4,
        runs_24h: 12,
        failed_24h: 2,
    }
}

fn made() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-11-03T14:05:09+05:30").unwrap()
}

#[test]
fn the_report_has_its_sections() {
    let logs = LogSection::Lines {
        lines: vec!["line one".into(), "line two".into()],
        left_out: 3,
    };
    let text = render(&facts(), "  The run never ended.  ", &logs, made());
    let expected = "\
Dots problem report
Made: 2026-11-03 14:05 (+05:30)

== What went wrong ==
The run never ended.

== This copy of Dots ==
Version: 0.3.0
System: windows x86_64
Engines in use: claude, local
Dots: 4
Runs in the last 24 hours: 12 (2 failed)

== Recent log lines (2, scrubbed) ==
line one
line two
(3 lines from MCP servers and the browser helper left out)
";
    assert_eq!(text, expected);
}

#[test]
fn the_report_says_what_is_missing() {
    let mut f = facts();
    f.engines.clear();
    let text = render(&f, "   ", &LogSection::NotIncluded, made());
    assert!(text.contains("== What went wrong ==\n(not described)\n"));
    assert!(text.contains("Engines in use: none\n"));
    assert!(
        text.ends_with("== Recent log lines ==\n(not included)\n"),
        "{text}"
    );

    let text = render(&facts(), "x", &LogSection::NoLog, made());
    assert!(
        text.ends_with("== Recent log lines ==\n(no log found)\n"),
        "{text}"
    );

    let none_left = LogSection::Lines {
        lines: vec!["a".into()],
        left_out: 0,
    };
    let text = render(&facts(), "x", &none_left, made());
    assert!(
        text.ends_with("== Recent log lines (1, scrubbed) ==\na\n"),
        "{text}"
    );
}

#[test]
fn the_file_name_carries_the_local_time() {
    assert_eq!(file_name(made()), "dots-report-20261103-1405.txt");
}

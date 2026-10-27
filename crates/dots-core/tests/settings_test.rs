mod common;

use common::temp_store;
use dots_core::settings::{self, Settings, TESTED_CLAUDE_VERSION};
use dots_core::store::Store;
use dots_core::Config;

#[tokio::test]
async fn an_empty_table_gives_the_defaults() {
    let (_dir, store) = temp_store().await;
    let s = store.get_settings().await.unwrap();
    assert_eq!(s, Settings::default());
    assert_eq!(s.port, 47321);
    assert_eq!(s.max_concurrent_runs, 2);
    assert_eq!(s.claude_path, None);
    assert_eq!(s.tested_claude_version, TESTED_CLAUDE_VERSION);
    assert!(s.local_endpoints.is_empty());
    assert!(s.autostart);
    assert!(!s.schedules_paused);
    assert_eq!(s.global_instructions, "");
    assert!(s.notify_on_notes);
}

#[tokio::test]
async fn settings_round_trip() {
    let (_dir, store) = temp_store().await;
    let s = Settings {
        port: 50000,
        max_concurrent_runs: 4,
        claude_path: Some("C:/tools/claude.exe".into()),
        tested_claude_version: "2.2.0".into(),
        local_endpoints: vec!["http://127.0.0.1:1234".into()],
        autostart: false,
        schedules_paused: true,
        global_instructions: "Use British spelling.\nSign off as the team.".into(),
        notify_on_notes: false,
        edge_path: Some("D:/Apps/Edge/msedge.exe".into()),
        hold_enabled: false,
        hold_share_five_hour: 50,
        hold_share_weekly: 80,
        auto_install_updates: true,
    };
    store.put_settings(&s).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), s);
    // A second write replaces the values.
    let back = Settings::default();
    store.put_settings(&back).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), back);
}

#[tokio::test]
async fn bad_and_unknown_rows_are_ignored_one_by_one() {
    let (dir, store) = temp_store().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    for (k, v) in [
        ("port", "\"not a number\""),
        ("max_concurrent_runs", "5"),
        ("autostart", "{broken json"),
        ("theme", "\"dark\""),
        // Right type, but a value `validate` rejects.
        ("claude_path", "\"\""),
        ("tested_claude_version", "\"\""),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(k)
            .bind(v)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
    let s = store.get_settings().await.unwrap();
    assert_eq!(s.port, 47321);
    assert_eq!(s.max_concurrent_runs, 5);
    assert!(s.autostart);
    assert_eq!(s.claude_path, None);
    assert_eq!(s.tested_claude_version, TESTED_CLAUDE_VERSION);
}

#[tokio::test]
async fn invalid_settings_are_rejected_and_not_stored() {
    let (_dir, store) = temp_store().await;
    type Case = (fn(&mut Settings), &'static str);
    let cases: Vec<Case> = vec![
        (|s| s.port = 0, "port"),
        (|s| s.max_concurrent_runs = 0, "max_concurrent_runs"),
        (|s| s.max_concurrent_runs = 17, "max_concurrent_runs"),
        (|s| s.claude_path = Some("  ".into()), "claude_path"),
        (
            |s| s.tested_claude_version = String::new(),
            "tested_claude_version",
        ),
        (
            |s| s.local_endpoints = vec!["https://example.com".into()],
            "https",
        ),
        (
            |s| s.global_instructions = "x".repeat(settings::MAX_GLOBAL_INSTRUCTIONS + 1),
            "32 KB",
        ),
    ];
    for (mutate, needle) in cases {
        let mut s = Settings::default();
        mutate(&mut s);
        let err = store.put_settings(&s).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{err}");
    }
    assert_eq!(store.get_settings().await.unwrap(), Settings::default());
    // 32 KB exactly is allowed.
    let s = Settings {
        global_instructions: "x".repeat(settings::MAX_GLOBAL_INSTRUCTIONS),
        ..Settings::default()
    };
    store.put_settings(&s).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), s);
}

fn with_claude_path(path: &str) -> Settings {
    Settings {
        claude_path: Some(path.into()),
        ..Settings::default()
    }
}

#[test]
fn claude_path_accepts_ordinary_paths() {
    for path in [
        r"C:\Users\ana\.local\bin\claude.exe",
        r"C:\Program Files\Claude Code\claude.exe",
        r"C:\Users\Zoë Müller\工具\claude.exe",
        // An emoji without a variation selector.
        "D:/apps/\u{2764}/claude.cmd",
        "claude",
    ] {
        with_claude_path(path)
            .validate()
            .unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}

#[test]
fn claude_path_refuses_characters_that_hide_or_reorder_it() {
    const CONTROL: &str = "control characters such as a tab or a line break";
    const FORMAT: &str = "invisible or text-direction characters";
    let cases = [
        ("C:/tools\tclaude.exe", CONTROL, "U+0009 at character 9"),
        (
            "C:/tools/claude.exe\nD:/x.exe",
            CONTROL,
            "U+000A at character 20",
        ),
        ("C:/tools/a\r.exe", CONTROL, "U+000D at character 11"),
        ("C:/tools/a\0.exe", CONTROL, "U+0000 at character 11"),
        ("C:/tools/a\u{7f}.exe", CONTROL, "U+007F at character 11"),
        ("C:/tools/a\u{85}.exe", CONTROL, "U+0085 at character 11"),
        ("C:/tools/a\u{2028}.exe", CONTROL, "U+2028 at character 11"),
        ("C:/tools/a\u{2029}.exe", CONTROL, "U+2029 at character 11"),
        // Shown as "C:/tools/claude.exe": the override turns the rest around.
        (
            "C:/tools/\u{202e}exe.edualc",
            FORMAT,
            "U+202E at character 10",
        ),
        (
            "C:/tools/cla\u{200b}ude.exe",
            FORMAT,
            "U+200B at character 13",
        ),
        // What "Copy" in a file's Properties dialog puts around a path.
        (
            "\u{202a}C:/tools/claude.exe\u{202c}",
            FORMAT,
            "U+202A at character 1",
        ),
        (
            "\u{feff}C:/tools/claude.exe",
            FORMAT,
            "U+FEFF at character 1",
        ),
        (
            "C:/tools/\u{2066}a\u{2069}.exe",
            FORMAT,
            "U+2066 at character 10",
        ),
        // Counted in characters, not bytes.
        ("C:/工具/a\u{200f}.exe", FORMAT, "U+200F at character 8"),
        ("C:/tools/a\u{e0041}.exe", FORMAT, "U+E0041 at character 11"),
        // Not format characters, but they draw nothing as well.
        (
            "C:/tools/cla\u{34f}ude.exe",
            FORMAT,
            "U+034F at character 13",
        ),
        (
            "C:/tools/cla\u{fe00}ude.exe",
            FORMAT,
            "U+FE00 at character 13",
        ),
        (
            "C:/tools/\u{2764}\u{fe0f}/a.exe",
            FORMAT,
            "U+FE0F at character 11",
        ),
        ("C:/tools/a\u{e0100}.exe", FORMAT, "U+E0100 at character 11"),
        ("C:/tools/a\u{115f}.exe", FORMAT, "U+115F at character 11"),
        ("C:/tools/a\u{3164}.exe", FORMAT, "U+3164 at character 11"),
        ("C:/tools/a\u{ffa0}.exe", FORMAT, "U+FFA0 at character 11"),
        ("C:/tools/a\u{17b4}.exe", FORMAT, "U+17B4 at character 11"),
        ("C:/tools/a\u{180b}.exe", FORMAT, "U+180B at character 11"),
        ("C:/tools/a\u{fff0}.exe", FORMAT, "U+FFF0 at character 11"),
        // A blank that `trim` keeps, where nobody would look.
        (
            "C:/tools/claude.exe\u{2800}",
            FORMAT,
            "U+2800 at character 20",
        ),
    ];
    for (path, rule, found) in cases {
        let err = with_claude_path(path).validate().unwrap_err();
        assert!(matches!(err, dots_core::Error::Invalid(_)), "{err:?}");
        assert_eq!(
            err.to_string(),
            format!("invalid input: claude_path must not contain {rule} (found {found})"),
            "{path:?}"
        );
    }
}

#[test]
fn claude_path_refuses_every_format_character_and_nothing_next_to_them() {
    // Unicode general category Cf, as inclusive ranges.
    const FORMAT: [(u32, u32); 21] = [
        (0xAD, 0xAD),
        (0x600, 0x605),
        (0x61C, 0x61C),
        (0x6DD, 0x6DD),
        (0x70F, 0x70F),
        (0x890, 0x891),
        (0x8E2, 0x8E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    let refused = |code: u32| {
        let c = char::from_u32(code).unwrap();
        with_claude_path(&format!("C:/a{c}b")).validate().is_err()
    };
    for (first, last) in FORMAT {
        for code in first..=last {
            assert!(refused(code), "U+{code:04X} should be refused");
        }
    }
    // The default-ignorable code points that are not format characters, and the blank braille
    // pattern: first and last of each range are refused, the code point beside it is not.
    const INVISIBLE: [(u32, u32); 11] = [
        (0x34F, 0x34F),
        (0x115F, 0x1160),
        (0x17B4, 0x17B5),
        (0x180B, 0x180D),
        (0x180F, 0x180F),
        (0x2800, 0x2800),
        (0x3164, 0x3164),
        (0xFE00, 0xFE0F),
        (0xFFA0, 0xFFA0),
        (0xFFF0, 0xFFF8),
        (0xE0080, 0xE0FFF),
    ];
    for (first, last) in INVISIBLE {
        for code in [first, last] {
            assert!(refused(code), "U+{code:04X} should be refused");
        }
        // U+180E, U+FFF9 and U+E007F next to three of the ranges are format characters, refused
        // already.
        for code in [first - 1, last + 1] {
            if ![0x180E, 0xFFF9, 0xE007F].contains(&code) {
                assert!(!refused(code), "U+{code:04X} should be accepted");
            }
        }
    }
    // So are the code points not assigned yet inside two of those blocks.
    for code in [0x2065, 0xE0000, 0xE0002, 0xE001F] {
        assert!(refused(code), "U+{code:04X} should be refused");
    }
    // The neighbours of the ranges are ordinary characters, and so are the look-alikes of a
    // space (no-break, hair, narrow no-break, ideographic), which are not format characters.
    for code in [
        0xAC, 0xAE, 0x606, 0x61B, 0x61D, 0x200A, 0x2010, 0x202F, 0x205F, 0x2070, 0xFEFC, 0xFFFC,
        0xA0, 0x3000,
    ] {
        assert!(!refused(code), "U+{code:04X} should be accepted");
    }
}

#[tokio::test]
async fn a_stored_claude_path_with_a_hidden_character_is_ignored() {
    let (dir, store) = temp_store().await;
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    // Stored by a version that did not check the path.
    for (k, v) in [
        ("port", "40003".to_string()),
        (
            "claude_path",
            serde_json::to_string("C:/tools/\u{202e}exe.edualc").unwrap(),
        ),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(k)
            .bind(v)
            .execute(&pool)
            .await
            .unwrap();
    }
    // The app still starts: the path is dropped (PATH is searched), the rest is kept.
    let s = store.get_settings().await.unwrap();
    assert_eq!(s.claude_path, None);
    assert_eq!(s.port, 40003);
    // The next save replaces the row.
    store.put_settings(&s).await.unwrap();
    let raw: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'claude_path'")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(raw, "null");
}

#[test]
fn only_port_concurrency_and_claude_path_need_a_restart() {
    let base = Settings::default();
    let mut s = base.clone();
    s.autostart = false;
    s.schedules_paused = true;
    s.tested_claude_version = "9.9.9".into();
    s.local_endpoints = vec!["http://127.0.0.1:1".into()];
    assert!(!base.restart_required(&s));
    for mutate in [
        (|s: &mut Settings| s.port = 1) as fn(&mut Settings),
        |s| s.max_concurrent_runs = 3,
        |s| s.claude_path = Some("claude.exe".into()),
    ] {
        let mut s = base.clone();
        mutate(&mut s);
        assert!(base.restart_required(&s));
    }
}

#[tokio::test]
async fn load_reads_the_settings_of_a_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let store = Store::open(&data.join("dots.db")).await.unwrap();
    let s = Settings {
        port: 40001,
        ..Settings::default()
    };
    store.put_settings(&s).await.unwrap();
    store.close().await;
    assert_eq!(settings::load(&data).await.unwrap().port, 40001);
    // A data dir without a database yet gives the defaults.
    let fresh = dir.path().join("fresh");
    assert_eq!(settings::load(&fresh).await.unwrap(), Settings::default());
}

#[test]
fn config_from_settings_takes_port_concurrency_and_claude_path() {
    let s = Settings {
        port: 40002,
        max_concurrent_runs: 3,
        claude_path: Some("C:/tools/claude.exe".into()),
        ..Settings::default()
    };
    let cfg = Config::from_settings("C:/data", &s);
    assert_eq!(cfg.port, 40002);
    assert_eq!(cfg.max_concurrent_runs, 3);
    assert_eq!(
        cfg.claude_path.as_deref(),
        Some(std::path::Path::new("C:/tools/claude.exe"))
    );
    assert_eq!(cfg.data_dir, std::path::PathBuf::from("C:/data"));
}

#[tokio::test]
async fn hold_shares_default_and_bounds() {
    let (_dir, store) = temp_store().await;
    let s = store.get_settings().await.unwrap();
    assert!(s.hold_enabled);
    assert_eq!(s.hold_share_five_hour, 60);
    assert_eq!(s.hold_share_weekly, 70);
    for bad in [5u8, 105, 62] {
        for weekly in [false, true] {
            let mut s = Settings::default();
            if weekly {
                s.hold_share_weekly = bad;
            } else {
                s.hold_share_five_hour = bad;
            }
            let err = s.validate().unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("invalid input: {}", settings::SHARE_RULE)
            );
        }
    }
    assert_eq!(
        settings::SHARE_RULE,
        "A share is 10 to 100%, in steps of 5."
    );
    for good in [10u8, 100] {
        let s = Settings {
            hold_share_five_hour: good,
            hold_share_weekly: good,
            ..Settings::default()
        };
        s.validate().unwrap();
    }
}

#[test]
fn auto_install_updates_is_off_by_default_and_reads_from_old_settings() {
    assert!(!Settings::default().auto_install_updates);
    let old: Settings = serde_json::from_str(r#"{"port": 47321}"#).unwrap();
    assert!(!old.auto_install_updates);
}

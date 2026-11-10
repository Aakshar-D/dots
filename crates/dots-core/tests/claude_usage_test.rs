//! All Claude use (usage tab spec sections 3 to 6): Claude Code's transcripts in a temporary
//! folder, read incrementally, each message once, split into Dots' and the person's own.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::claude_usage::{ClaudeUsageService, DOTS_ROW, SCAN_BYTES};
use dots_core::model::{NewRun, TriggerKind, UsagePeriod};
use dots_core::store::{LimitReading, Store};
use dots_core::util::timestamp;
use tempfile::TempDir;

/// Now, to the second: the files' times are the PC's, so the clock starts at the real time.
fn now() -> DateTime<Utc> {
    Utc::now().duration_trunc(TimeDelta::seconds(1)).unwrap()
}

/// One assistant line.
fn line(id: &str, req: &str, ago: TimeDelta, session: &str, cwd: &Path, out: i64) -> String {
    serde_json::json!({
        "type": "assistant",
        "timestamp": timestamp(now() - ago),
        "sessionId": session,
        "cwd": cwd,
        "requestId": req,
        "isSidechain": false,
        "message": {
            "id": id,
            "model": "claude-opus-5-5",
            "role": "assistant",
            "content": [{ "type": "text", "text": "ok" }],
            "usage": {
                "input_tokens": 10,
                "cache_creation_input_tokens": 100,
                "cache_read_input_tokens": 1000,
                "output_tokens": out
            }
        }
    })
    .to_string()
        + "\n"
}

fn user_line() -> String {
    r#"{"type":"user","message":{"role":"user","content":"a long tool result"}}"#.to_string() + "\n"
}

struct Setup {
    _dir: TempDir,
    _store_dir: TempDir,
    store: Store,
    projects: PathBuf,
    data: PathBuf,
    clock: Arc<FakeClock>,
    usage: Arc<ClaudeUsageService>,
}

async fn setup() -> Setup {
    let dir = TempDir::new().unwrap();
    let (store_dir, store) = temp_store().await;
    let projects = dir.path().join("claude").join("projects");
    std::fs::create_dir_all(&projects).unwrap();
    let data = dir.path().join("dots-data");
    let clock = FakeClock::new(now());
    let usage =
        ClaudeUsageService::new(store.clone(), Some(projects.clone()), &data, clock.clone());
    Setup {
        _dir: dir,
        _store_dir: store_dir,
        store,
        projects,
        data,
        clock,
        usage,
    }
}

impl Setup {
    /// Appends `text` to `<projects>/<project>/<file>`.
    fn append(&self, project: &str, file: &str, text: &str) -> PathBuf {
        let path = self.projects.join(project).join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
        path
    }

    async fn scan(&self) -> dots_core::claude_usage::ClaudeScan {
        self.usage.scan(TimeDelta::zero()).await.unwrap()
    }

    async fn week(&self) -> dots_core::claude_usage::ClaudeUsage {
        self.usage.summary(UsagePeriod::Week).await.unwrap()
    }
}

fn work() -> PathBuf {
    PathBuf::from(if cfg!(windows) {
        r"C:\work\app"
    } else {
        "/work/app"
    })
}

#[tokio::test]
async fn it_counts_each_message_once_across_streamed_copies_and_files() {
    let s = setup().await;
    let h = TimeDelta::hours(1);
    // A streamed reply written once per block, the last with more output, then a user line.
    s.append(
        "C--work-app",
        "s1.jsonl",
        &[
            line("m1", "r1", h, "s1", &work(), 5),
            line("m1", "r1", h, "s1", &work(), 40),
            user_line(),
            line("m2", "r2", h, "s1", &work(), 7),
        ]
        .concat(),
    );
    // A resumed session copies m1 into its own file; a subagent has its own message.
    s.append(
        "C--work-app",
        "s2.jsonl",
        &[
            line("m1", "r1", h, "s1", &work(), 40),
            line("m3", "r3", h, "s2", &work(), 1),
        ]
        .concat(),
    );
    s.append(
        "C--work-app",
        "s2/subagents/agent-a1.jsonl",
        &line("m4", "r4", h, "s2", &work(), 2),
    );
    let scan = s.scan().await;
    assert_eq!(scan.files, 3);
    assert!(!scan.more);
    let week = s.week().await;
    assert_eq!(week.own.messages, 4);
    assert_eq!(week.own.sessions, 2);
    assert_eq!(week.own.output_tokens, 40 + 7 + 1 + 2);
    assert_eq!(week.own.input_tokens, 40);
    assert_eq!(week.own.cache_creation_tokens, 400);
    assert_eq!(week.own.cache_read_tokens, 4000);
    assert_eq!(week.dots.messages, 0);
    assert_eq!(week.projects.len(), 1);
    assert_eq!(week.projects[0].name, "app");
    assert_eq!(
        week.projects[0].cwd.as_deref(),
        Some(work().to_str().unwrap())
    );
    assert!(week.available);
    assert!(week.scanned_at.is_some());
}

#[tokio::test]
async fn a_second_scan_reads_only_the_appended_bytes_and_a_partial_line_later() {
    let s = setup().await;
    let h = TimeDelta::hours(1);
    let first = line("m1", "r1", h, "s1", &work(), 1);
    s.append("p", "s1.jsonl", &first);
    assert_eq!(s.scan().await.bytes, first.len() as u64);
    // Nothing new: the file is not read.
    let again = s.scan().await;
    assert_eq!((again.files, again.bytes), (0, 0));
    // A line and the start of another, still being written.
    let second = line("m2", "r2", h, "s1", &work(), 1);
    let third = line("m3", "r3", h, "s1", &work(), 1);
    let (head, tail) = third.split_at(30);
    s.append("p", "s1.jsonl", &(second.clone() + head));
    assert_eq!(s.scan().await.bytes, second.len() as u64);
    assert_eq!(s.week().await.own.messages, 2);
    s.append("p", "s1.jsonl", tail);
    assert_eq!(s.scan().await.bytes, third.len() as u64);
    assert_eq!(s.week().await.own.messages, 3);
}

#[tokio::test]
async fn a_rewritten_file_is_read_again_without_counting_twice() {
    let s = setup().await;
    let h = TimeDelta::hours(1);
    let path = s.append(
        "p",
        "s1.jsonl",
        &[
            line("m1", "r1", h, "s1", &work(), 1),
            line("m2", "r2", h, "s1", &work(), 1),
        ]
        .concat(),
    );
    s.scan().await;
    std::fs::write(&path, line("m1", "r1", h, "s1", &work(), 1)).unwrap();
    assert_eq!(s.scan().await.files, 1);
    assert_eq!(s.week().await.own.messages, 2);
}

#[tokio::test]
async fn a_scan_stops_at_the_cap_and_says_there_is_more() {
    let s = setup().await;
    let h = TimeDelta::hours(1);
    // Two files, each a little over the cap.
    let one = line("m1", "r1", h, "s1", &work(), 1);
    let pad = user_line();
    let lines = (SCAN_BYTES as usize / pad.len()) + 1;
    s.append("p", "a.jsonl", &(one + &pad.repeat(lines)));
    s.append(
        "p",
        "b.jsonl",
        &(pad.repeat(lines) + &line("m2", "r2", h, "s2", &work(), 1)),
    );
    let first = s.scan().await;
    assert!(first.more);
    assert_eq!(first.files, 1);
    let mut scans = 1;
    while s.scan().await.more {
        scans += 1;
        assert!(scans < 5, "the scans never caught up");
    }
    assert_eq!(s.week().await.own.messages, 2);
}

#[tokio::test]
async fn old_messages_and_old_files_are_left_out() {
    let s = setup().await;
    let path = s.append(
        "p",
        "old.jsonl",
        &line("m0", "r0", TimeDelta::days(1), "s0", &work(), 1),
    );
    // Last written 40 days ago: marked read without opening.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 24 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    s.append(
        "p",
        "new.jsonl",
        &[
            line("m1", "r1", TimeDelta::days(40), "s1", &work(), 1),
            line("m2", "r2", TimeDelta::days(20), "s1", &work(), 1),
            line("m3", "r3", TimeDelta::hours(1), "s1", &work(), 1),
        ]
        .concat(),
    );
    let scan = s.scan().await;
    assert_eq!(scan.files, 1);
    let month = s.usage.summary(UsagePeriod::Month).await.unwrap();
    assert_eq!(month.own.messages, 2);
    assert_eq!(s.week().await.own.messages, 1);
    let five = s.usage.summary(UsagePeriod::FiveHours).await.unwrap();
    assert_eq!(five.own.messages, 1);
}

#[tokio::test]
async fn dots_messages_are_told_apart_by_session_and_by_data_folder() {
    let s = setup().await;
    let h = TimeDelta::hours(1);
    let dot = s
        .store
        .create_dot(&spec("scout"), None, "default")
        .await
        .unwrap();
    let run = s
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    s.store.set_session(&run.id, "dots-session").await.unwrap();
    // A dot working in the person's own repo: its session is on a run.
    s.append(
        "C--work-app",
        "dots-session.jsonl",
        &line("d1", "r1", h, "dots-session", &work(), 3),
    );
    // A one-off check in Dots' data folder.
    s.append(
        "check",
        "c1.jsonl",
        &line("d2", "r2", h, "c1", &s.data.join("checks").join("x"), 4),
    );
    // The person's own session in the same repo, and a session Dots stores only later.
    s.append(
        "C--work-app",
        "mine.jsonl",
        &line("o1", "r3", h, "mine", &work(), 5),
    );
    s.append(
        "C--work-app",
        "late.jsonl",
        &line("l1", "r4", h, "late", &work(), 6),
    );
    s.scan().await;
    let week = s.week().await;
    assert_eq!((week.dots.messages, week.own.messages), (2, 2));
    let later = s
        .store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Chat))
        .await
        .unwrap();
    s.store.set_session(&later.id, "late").await.unwrap();
    let week = s.week().await;
    assert_eq!((week.dots.messages, week.own.messages), (3, 1));
    assert_eq!(week.dots.output_tokens, 3 + 4 + 6);
    assert_eq!(week.own.output_tokens, 5);
    // All Dots' messages are one row; the person's are by folder.
    let rows: Vec<_> = week
        .projects
        .iter()
        .map(|p| (p.name.as_str(), p.cwd.is_some()))
        .collect();
    assert_eq!(rows, vec![(DOTS_ROW, false), ("app", true)]);
    // A dot deleted later keeps its messages marked as Dots'.
    s.store.delete_dot(&dot.id).await.unwrap();
    let week = s.week().await;
    assert_eq!((week.dots.messages, week.own.messages), (2, 2));
}

#[tokio::test]
async fn the_block_counts_every_message_since_the_five_hour_window_started() {
    let s = setup().await;
    let resets = now() + TimeDelta::hours(2);
    s.store
        .save_limit_readings(&[LimitReading {
            window: "five_hour".into(),
            utilization: 0.4,
            resets_at: timestamp(resets),
            status: "allowed".into(),
            read_at: timestamp(now()),
        }])
        .await
        .unwrap();
    s.append(
        "p",
        "s.jsonl",
        &[
            line("m1", "r1", TimeDelta::hours(4), "s", &work(), 1),
            line("m2", "r2", TimeDelta::hours(2), "s", &work(), 1),
            line("m3", "r3", TimeDelta::minutes(5), "s", &s.data, 1),
        ]
        .concat(),
    );
    s.scan().await;
    let block = s.week().await.block.unwrap();
    assert_eq!(block.started_at, timestamp(resets - TimeDelta::hours(5)));
    assert_eq!(block.messages, 2);
    assert_eq!(block.tokens, 2 * (10 + 100 + 1));
    // Once the window has reset with no new reading, there is no block.
    s.clock.set(resets + TimeDelta::minutes(1));
    assert_eq!(s.week().await.block, None);
}

#[tokio::test]
async fn a_scan_asked_for_again_soon_gets_the_last_result() {
    let s = setup().await;
    s.append(
        "p",
        "s.jsonl",
        &line("m1", "r1", TimeDelta::hours(1), "s", &work(), 1),
    );
    let gap = TimeDelta::seconds(30);
    let first = s.usage.scan(gap).await.unwrap();
    s.append(
        "p",
        "s.jsonl",
        &line("m2", "r2", TimeDelta::hours(1), "s", &work(), 1),
    );
    assert_eq!(s.usage.scan(gap).await.unwrap(), first);
    s.clock.set(now() + TimeDelta::seconds(31));
    assert_eq!(s.usage.scan(gap).await.unwrap().files, 1);
}

#[tokio::test]
async fn with_no_folder_there_is_nothing_to_read() {
    let s = setup().await;
    std::fs::remove_dir_all(&s.projects).unwrap();
    let scan = s.scan().await;
    assert_eq!((scan.files, scan.bytes, scan.more), (0, 0, false));
    let week = s.week().await;
    assert!(!week.available);
    assert_eq!(week.folder.as_deref(), Some(s.projects.to_str().unwrap()));
    assert!(week.projects.is_empty());

    let (_d, store) = temp_store().await;
    let none = ClaudeUsageService::new(store, None, &s.data, s.clock.clone());
    assert_eq!(none.summary(UsagePeriod::Week).await.unwrap().folder, None);
}

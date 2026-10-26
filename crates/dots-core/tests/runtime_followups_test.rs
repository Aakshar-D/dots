//! A chat message and a due follow-up on one thread at the same moment (check-ins spec section
//! 6, "Thread lock"): whichever comes first, no run of the thread gets two children.

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, wait_status};
use dots_core::checkins::When;
use dots_core::model::{FollowUpStatus, RunStatus, WorkspaceMode};
use dots_core::{Config, Error, Runtime};

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2030-01-07T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// Review Focus 4.
#[tokio::test]
async fn a_chat_and_a_due_follow_up_on_one_thread_give_one_run() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(t0());
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.clock = clock.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    let mut s = spec("racer");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    let dot = rt.create_dot(s, None).await.unwrap();
    let first = rt.run_now(&dot.id).await.unwrap();
    let first = wait_status(rt.store(), &first.id, RunStatus::Succeeded, 15).await;
    let follow_up = rt
        .follow_ups()
        .schedule(&first, "Look again.", When::In(TimeDelta::minutes(10)))
        .await
        .unwrap();

    let (chat, ()) = tokio::join!(rt.chat(&dot.id, "and now?", Some(&first.id)), async {
        clock.set(t0() + TimeDelta::minutes(11))
    });
    match &chat {
        Ok(run) => assert_eq!(run.parent_run_id.as_deref(), Some(first.id.as_str())),
        Err(Error::Conflict(_)) => {}
        Err(e) => panic!("{e}"),
    }
    // Whichever came first, the follow-up fires: at once, or when the chat's run has ended.
    let deadline = Instant::now() + Duration::from_secs(30);
    let thread = loop {
        let row = rt.store().get_follow_up(&follow_up.id).await.unwrap();
        let thread = rt.store().list_thread(&first.root_run_id).await.unwrap();
        if row.status == FollowUpStatus::Fired && thread.iter().all(|r| r.status.is_terminal()) {
            break thread;
        }
        assert!(
            Instant::now() < deadline,
            "the follow-up did not fire: {row:?} {thread:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let parents: Vec<&str> = thread
        .iter()
        .filter_map(|r| r.parent_run_id.as_deref())
        .collect();
    let unique: HashSet<&str> = parents.iter().copied().collect();
    assert_eq!(
        unique.len(),
        parents.len(),
        "a run has two children: {thread:#?}"
    );
    assert_eq!(thread.len(), if chat.is_ok() { 3 } else { 2 });
    rt.shutdown();
}

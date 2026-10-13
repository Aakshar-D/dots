mod common;

use std::process::Stdio;
use std::time::Duration;

use common::Lingerer;
use dots_core::proc::{self, Job};
use tokio::process::{Child, Command};

/// The fake CLI in one of its process-tree modes. It starts nothing until its stdin closes.
fn fake(mode: &str, lingerer: &Lingerer) -> Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-claude"));
    cmd.args([mode, &lingerer.dir()])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    proc::hide_window(&mut cmd);
    cmd.spawn().unwrap()
}

/// A job whose only living process is a grandchild: the child that started it has exited,
/// so no parent link leads to it any more.
#[cfg(windows)]
async fn job_with_an_orphan(lingerer: &Lingerer) -> Job {
    let mut child = fake("--orphan", lingerer);
    let job = Job::assign(&child).unwrap();
    drop(child.stdin.take());
    assert!(child.wait().await.unwrap().success());
    assert!(lingerer.alive(), "the grandchild should outlive its parent");
    job
}

#[cfg(windows)]
#[tokio::test]
async fn terminate_kills_a_grandchild_whose_parent_has_exited() {
    let lingerer = Lingerer::new();
    let job = job_with_an_orphan(&lingerer).await;
    job.terminate().await;
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after terminate"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn dropping_the_job_kills_a_grandchild_whose_parent_has_exited() {
    let lingerer = Lingerer::new();
    let job = job_with_an_orphan(&lingerer).await;
    drop(job);
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after the job was dropped"
    );
}

/// Dots being killed runs no destructor: the job must end because its handle is gone.
#[cfg(windows)]
#[tokio::test]
async fn a_job_ends_when_the_process_holding_it_is_killed() {
    let lingerer = Lingerer::new();
    let mut holder = fake("--hold-job", &lingerer);
    lingerer.started().await;
    holder.kill().await.unwrap();
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after the holder of its job was killed"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn a_process_that_has_exited_cannot_be_assigned() {
    let lingerer = Lingerer::new();
    let mut child = fake("--version", &lingerer);
    child.wait().await.unwrap();
    assert!(Job::assign(&child).is_err());
}

/// Without a job object `terminate` is `kill_tree`: it ends the child, and on Windows
/// everything a parent link still leads to.
#[tokio::test]
async fn the_fallback_kills_a_tree_that_is_still_linked() {
    let lingerer = Lingerer::new();
    let mut child = fake("--tree", &lingerer);
    let job = Job::fallback(&child);
    drop(child.stdin.take());
    lingerer.started().await;
    job.terminate().await;
    let exited = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    assert!(exited.is_ok(), "the child is still running after terminate");
    #[cfg(windows)]
    assert!(
        lingerer.gone_within(Duration::from_secs(5)).await,
        "the grandchild is still running after terminate"
    );
}

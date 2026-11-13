//! Jobs (dots-make-dots design section 8): a maker gives a job to a dot it made, which starts a
//! chat run of that dot; the job follows that run through resumes and the person's answers
//! until a run ends with no question waiting; then the maker is woken exactly once, in the
//! topic that gave the job, with how it ended and the last run's final text. At most 6 jobs
//! are given in one topic between two messages the person typed.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, TimeDelta, Utc};
use common::{spec, wait_status};
use dots_core::checkins::When;
use dots_core::events::RuntimeEvent;
use dots_core::limits::{run_is_background, HoldShares, LimitReading};
use dots_core::making::jobs::{
    FirstJob, JobEnd, JobStatus, MakingJob, JOB_ID, MAX_JOBS_BETWEEN_TYPED, MAX_JOB_RESULT,
};
use dots_core::making::log::{MakingEvent, MakingEventKind};
use dots_core::making::manage::{job_went_message, made_dots_text};
use dots_core::making::reach::{Extra, Wanted};
use dots_core::making::requests::{MakeDecision, MakeRequest, RequestKind, RequestStatus};
use dots_core::making::service::{MakeDot, MakeOutcome, NOT_IN_A_STEP, RUN_ENDED};
use dots_core::model::{Dot, Making, Message, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::prompt::{build_prompt, is_typed, job_note, resent_job_note};
use dots_core::util::timestamp;
use dots_core::vault::{EntryKind, Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::ConnectOptions;
use tempfile::TempDir;

/// What `give_job` tells the maker's run.
const GAVE: &str = "Gave the job to \"Blog Watcher\". It has started (or waits behind its \
                    current run). Do not wait for it: carry on with other work or end your turn, \
                    and you will be woken with its result.";
/// What the 7th job of a topic since the person wrote is told.
const GUARD: &str = "Researcher has given 6 jobs since the person last wrote here. Stop and \
                     report to the person what is done and what is left; they can tell you to \
                     carry on.";

struct T {
    dir: TempDir,
    secrets: Arc<MemorySecrets>,
    rt: Runtime,
}

/// A runtime on the data folder below `dir`. With `runs` its queued runs start, each with the
/// fake CLI, which waits `slow_ms` before it ends with the text "fake done"; without, none
/// starts, and a test puts its runs in the state it needs by hand and calls the pass that
/// settles the jobs itself.
async fn runtime(dir: &Path, secrets: &Arc<MemorySecrets>, runs: bool, slow_ms: u64) -> Runtime {
    let mut cfg = Config::new(dir.join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.new_dot_root = dir.join("Dots");
    cfg.secrets = Some(secrets.clone());
    if !runs {
        cfg.max_concurrent_runs = 0;
    }
    if slow_ms > 0 {
        let script = json!([
            {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
            {"sleep_ms": slow_ms},
            {"emit": {"type": "result", "subtype": "success", "is_error": false,
                      "result": "fake done", "usage": {"input_tokens": 1, "output_tokens": 1}}},
        ]);
        let p = dir.join("script.json");
        std::fs::write(&p, script.to_string()).unwrap();
        cfg.claude_env
            .push(("FAKE_CLAUDE_SCRIPT".into(), p.to_string_lossy().to_string()));
    }
    Runtime::start(cfg).await.unwrap()
}

async fn start_with(runs: bool, slow_ms: u64) -> T {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let secrets = Arc::new(MemorySecrets::new());
    let rt = runtime(dir.path(), &secrets, runs, slow_ms).await;
    T { dir, secrets, rt }
}

/// A runtime whose runs do not start.
async fn quiet() -> T {
    start_with(false, 0).await
}

/// A runtime whose runs start and take `slow_ms` each.
async fn running(slow_ms: u64) -> T {
    start_with(true, slow_ms).await
}

/// A maker named `name` that works in the test's `work` folder itself.
async fn maker_named(t: &T, name: &str) -> Dot {
    let mut s = spec(name);
    s.workdir = t.dir.path().join("work").to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.making.on = true;
    t.rt.create_dot(s, None, None).await.unwrap()
}

/// A maker named Researcher.
async fn maker(t: &T) -> Dot {
    maker_named(t, "Researcher").await
}

/// Puts the queued run `run_id` in the running state by hand, with a session, as a run the
/// runner started.
async fn start(t: &T, run_id: &str) -> Run {
    assert!(t.rt.store().claim_run(run_id).await.unwrap());
    t.rt.store()
        .set_session(run_id, &format!("session-{run_id}"))
        .await
        .unwrap();
    t.rt.store().get_run(run_id).await.unwrap()
}

/// A run of `dot` that is running, put there by hand: a new topic started as `trigger`. A chat
/// is one the person started with a typed message.
async fn live_run_as(t: &T, dot: &Dot, trigger: TriggerKind) -> Run {
    let mut new = NewRun::new(&dot.id, trigger);
    if trigger == TriggerKind::Chat {
        new.payload = Some(json!({"message": "watch three blogs", "typed": true}));
    }
    let run = t.rt.store().create_run(&new).await.unwrap();
    start(t, &run.id).await
}

/// A chat of `dot` the person started with a typed message, whose run is running.
async fn live_run(t: &T, dot: &Dot) -> Run {
    live_run_as(t, dot, TriggerKind::Chat).await
}

/// A run that goes on in the topic after `end`, which has ended, with `payload`, and is
/// running: put there by hand.
async fn next_run(t: &T, end: &Run, payload: serde_json::Value) -> Run {
    let mut new = NewRun::new(&end.dot_id, TriggerKind::Chat);
    new.parent_run_id = Some(end.id.clone());
    new.root_run_id = Some(end.root_run_id.clone());
    new.session_id = end.session_id.clone();
    new.payload = Some(payload);
    let run = t.rt.store().create_run(&new).await.unwrap();
    start(t, &run.id).await
}

/// Ends the run `run_id` by hand as `status`, with `summary` as its final text and `error`.
async fn finish(
    t: &T,
    run_id: &str,
    status: RunStatus,
    summary: Option<&str>,
    error: Option<&str>,
) -> Run {
    t.rt.store()
        .finish_run(run_id, status, summary, error)
        .await
        .unwrap()
}

/// What a maker asks for a dot named `name`, with nothing but what a new dot needs.
fn ask(name: &str) -> MakeDot {
    MakeDot {
        purpose: "Reads the feeds".into(),
        wanted: Wanted {
            name: Some(name.into()),
            instructions: Some("Read the three blogs and list what is new.".into()),
            ..Wanted::default()
        },
        first_job: None,
    }
}

/// Makes the dot, which must be within reach and within the limits.
async fn make(t: &T, run: &Run, name: &str) -> Dot {
    match t.rt.makers().make_dot(&run.id, ask(name)).await.unwrap() {
        MakeOutcome::Made { dot, .. } => dot,
        other => panic!("{name} was not made: {other:?}"),
    }
}

/// The jobs given in the maker's topic `run` belongs to, oldest first.
async fn jobs(t: &T, run: &Run) -> Vec<MakingJob> {
    t.rt.store()
        .making_jobs_of_topic(&run.root_run_id)
        .await
        .unwrap()
}

/// The job as it is stored now.
async fn stored(t: &T, job: &MakingJob) -> MakingJob {
    t.rt.store().get_making_job(&job.id).await.unwrap()
}

/// The run the job is at now.
async fn run_of(t: &T, job: &MakingJob) -> Run {
    let job = stored(t, job).await;
    let id = job.last_run_id.expect("the job has a run");
    t.rt.store().get_run(&id).await.unwrap()
}

/// Gives the made dot named `name` the job `message` from the maker's run `run`, which must be
/// taken: the job.
async fn give(t: &T, run: &Run, name: &str, message: &str) -> MakingJob {
    let text = t.rt.makers().give_job(&run.id, name, message).await;
    assert_eq!(text.unwrap(), GAVE.replace("Blog Watcher", name));
    jobs(t, run).await.pop().expect("a job was stored")
}

/// Runs the pass that settles the jobs, as the end of a run does.
async fn settle(t: &T) {
    t.rt.makers().settle_jobs().await.unwrap();
}

/// Gives a job and lets it end: its run starts and ends by hand with the final text `text`,
/// and the pass settles it.
async fn done_job(t: &T, run: &Run, name: &str, text: &str) -> MakingJob {
    let job = give(t, run, name, "Read the blogs now.").await;
    let its = run_of(t, &job).await;
    start(t, &its.id).await;
    finish(t, &its.id, RunStatus::Succeeded, Some(text), None).await;
    settle(t).await;
    let job = stored(t, &job).await;
    assert_eq!(job.status, JobStatus::Ended);
    job
}

/// The text of an `Invalid`; anything else fails the test.
fn invalid<V: std::fmt::Debug>(result: Result<V, Error>) -> String {
    match result {
        Err(Error::Invalid(text)) => text,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// The messages of the wakes that wait, oldest first.
async fn wakes(t: &T) -> Vec<String> {
    let wakes = t.rt.store().waiting_making_wakes().await.unwrap();
    wakes.into_iter().map(|w| w.message).collect()
}

/// The makers' log of the topic `run` belongs to.
async fn log(t: &T, run: &Run) -> Vec<MakingEvent> {
    t.rt.store()
        .list_making_events(&run.root_run_id)
        .await
        .unwrap()
}

/// The kinds of the log's entries, oldest first.
async fn kinds(t: &T, run: &Run) -> Vec<MakingEventKind> {
    log(t, run).await.iter().map(|e| e.kind).collect()
}

fn message(run: &Run) -> &str {
    run.payload.as_ref().unwrap()["message"].as_str().unwrap()
}

/// The prompt the run would get.
fn prompt(dot: &Dot, run: &Run) -> String {
    build_prompt(dot, run, Path::new("C:/work"), Local::now(), "", &[])
}

/// The question the run `run` asks the person, stored as the `ask` tool stores it.
async fn question(t: &T, run: &Run) -> Message {
    t.rt.store()
        .add_question(&run.dot_id, &run.id, "Which of the three first?", None)
        .await
        .unwrap()
        .expect("the run's first question")
}

/// Polls until the job has ended, and returns it.
async fn ended(t: &T, job: &MakingJob) -> MakingJob {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let now = stored(t, job).await;
        if now.status == JobStatus::Ended {
            return now;
        }
        assert!(Instant::now() < deadline, "the job did not end: {now:#?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_job_starts_a_run_of_the_made_dot_that_says_who_sent_it() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let mut rx = t.rt.subscribe();

    let text =
        t.rt.makers()
            .give_job(&run.id, " blog watcher ", "  Read the blogs now.  ")
            .await
            .unwrap();

    assert_eq!(text, GAVE);
    let all = jobs(&t, &run).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    let job = &all[0];
    assert_eq!(job.maker_dot_id, maker.id);
    assert_eq!(job.maker_run_id, run.id);
    assert_eq!(job.maker_root_run_id, run.root_run_id);
    assert_eq!(job.dot_id, made.id);
    assert_eq!(job.message, "Read the blogs now.");
    assert_eq!(job.status, JobStatus::Working);
    assert_eq!(job.ended_how, None);
    assert_eq!(job.ended_at, None);
    assert_eq!(job.wake_id, None);
    assert_eq!(job.first_run_id, job.last_run_id);

    // A chat run of the made dot in a topic of its own, queued as any run is.
    let its = run_of(&t, job).await;
    assert_eq!(its.dot_id, made.id);
    assert_eq!(its.trigger, TriggerKind::Chat);
    assert_eq!(its.status, RunStatus::Queued);
    assert_eq!(its.parent_run_id, None);
    assert_eq!(
        job.dot_root_run_id.as_deref(),
        Some(its.root_run_id.as_str())
    );
    // Dots' own words, not a message the person typed; it names its job and who gave it.
    assert!(!is_typed(&its));
    assert_eq!(
        its.payload,
        Some(json!({
            "message": "Read the blogs now.",
            "job_id": job.id,
            "job_from": "Researcher",
        }))
    );
    assert_eq!(its.payload.as_ref().unwrap()[JOB_ID], job.id.as_str());
    let prompt = prompt(&made, &its);
    assert!(
        prompt.contains(
            "## Job\n\nJob from Researcher (the dot that made you). This is not a message from \
             the person. When you finish, your final message goes back to Researcher.\n\n> Read \
             the blogs now.\n\n"
        ),
        "{prompt}"
    );
    assert!(!prompt.contains("Message from the user"), "{prompt}");
    assert!(!prompt.contains("Chat with the person"), "{prompt}");

    // The maker's topic shows it: "<made dot> is working", with the job and its run.
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [MakingEventKind::Made, MakingEventKind::JobStarted]
    );
    let started = &events[1];
    assert_eq!(started.maker_dot_id, maker.id);
    assert_eq!(started.run_id, run.id);
    assert_eq!(started.root_run_id, run.root_run_id);
    assert_eq!(started.dot_id.as_deref(), Some(made.id.as_str()));
    assert_eq!(started.dot_name, "Blog Watcher");
    assert_eq!(
        started.detail,
        json!({"job_id": job.id, "message": "Read the blogs now.", "run_id": its.id})
    );
    let mut updated = false;
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::MakingUpdated {
            dot_id,
            root_run_id,
        } = ev
        {
            updated |= dot_id == maker.id && root_run_id == run.root_run_id;
        }
    }
    assert!(updated, "the topic's cards were not told");
    // Nothing has ended: nobody is woken.
    settle(&t).await;
    assert!(wakes(&t).await.is_empty());
    assert_eq!(stored(&t, job).await.status, JobStatus::Working);
}

#[tokio::test]
async fn the_maker_is_woken_once_with_the_final_text_when_the_job_ends() {
    let t = running(0).await;
    let maker = maker(&t).await;
    // The maker's run is put in the running state by hand, so it is still working when the
    // job ends and the wake waits where the test can read it.
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;

    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;

    // Nobody calls anything: the made dot's run starts and ends, and its end ends the job.
    let job = ended(&t, &job).await;
    let its = run_of(&t, &job).await;
    assert_eq!(its.status, RunStatus::Succeeded);
    assert_eq!(its.dot_id, made.id);
    assert_eq!(job.ended_how, Some(JobEnd::Succeeded));
    assert!(job.ended_at.is_some());
    let said = "\"Blog Watcher\" finished the job.\n\nIts final message:\n> fake done";
    assert_eq!(wakes(&t).await, [said]);
    let wake_id = job.wake_id.clone().expect("the job names its wake");
    let wake = t.rt.store().get_making_wake(&wake_id).await.unwrap();
    assert_eq!(wake.maker_dot_id, maker.id);
    assert_eq!(wake.root_run_id, run.root_run_id);
    assert_eq!(wake.message, said);
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Made,
            MakingEventKind::JobStarted,
            MakingEventKind::JobEnded
        ]
    );
    assert_eq!(events[2].run_id, run.id);
    assert_eq!(events[2].dot_id.as_deref(), Some(made.id.as_str()));
    assert_eq!(events[2].dot_name, "Blog Watcher");
    assert_eq!(
        events[2].detail,
        json!({"job_id": job.id, "run_id": its.id, "how": "succeeded"})
    );

    // The maker's run ends: the wake's run appears in the maker's topic, with the message.
    finish(&t, &run.id, RunStatus::Succeeded, Some("made it"), None).await;
    t.rt.makers().deliver_wakes().await.unwrap();
    let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
    assert_eq!(thread.len(), 2, "{thread:#?}");
    assert_eq!(thread[1].dot_id, maker.id);
    assert_eq!(message(&thread[1]), said);
    assert_eq!(thread[1].payload.as_ref().unwrap()["wake_id"], wake_id);
    assert!(!is_typed(&thread[1]));
    wait_status(t.rt.store(), &thread[1].id, RunStatus::Succeeded, 15).await;

    // A second pass delivers nothing more: the job has its wake.
    settle(&t).await;
    t.rt.makers().deliver_wakes().await.unwrap();
    assert!(wakes(&t).await.is_empty());
    let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
    assert_eq!(thread.len(), 2, "{thread:#?}");
    assert_eq!(stored(&t, &job).await, job);
    assert_eq!(kinds(&t, &run).await.len(), 3);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_failed_job_and_a_stopped_job_wake_the_maker_and_say_how() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    for name in ["Alpha", "Beta", "Gamma"] {
        make(&t, &run, name).await;
    }
    let failed = give(&t, &run, "Alpha", "Read the blogs now.").await;
    let stopped = give(&t, &run, "Beta", "Read the blogs now.").await;
    let capped = give(&t, &run, "Gamma", "Read the blogs now.").await;

    // One fails after it said something, one is stopped by the person before it started, and
    // one is stopped by a spending cap.
    let its = run_of(&t, &failed).await;
    start(&t, &its.id).await;
    finish(
        &t,
        &its.id,
        RunStatus::Failed,
        Some("I read two of three."),
        Some("engine exited without a result"),
    )
    .await;
    let its = run_of(&t, &stopped).await;
    t.rt.cancel_run(&its.id).await.unwrap();
    let its = run_of(&t, &capped).await;
    start(&t, &its.id).await;
    let cap = "stopped: Gamma reached its $1.00 daily cap";
    finish(&t, &its.id, RunStatus::Failed, None, Some(cap)).await;
    settle(&t).await;

    let mut said = wakes(&t).await;
    said.sort();
    assert_eq!(
        said,
        [
            "\"Alpha\" failed.\n\nThe run failed. Its error text:\n> engine exited without a \
             result\n\nIts final message:\n> I read two of three.",
            "\"Beta\" was stopped.\n\nIt left no final message.",
            "\"Gamma\" was stopped by a spend cap.\n\nIt left no final message.",
        ]
    );
    assert_eq!(stored(&t, &failed).await.ended_how, Some(JobEnd::Failed));
    assert_eq!(
        stored(&t, &stopped).await.ended_how,
        Some(JobEnd::Cancelled)
    );
    assert_eq!(stored(&t, &capped).await.ended_how, Some(JobEnd::Failed));
    let hows: Vec<String> = log(&t, &run)
        .await
        .iter()
        .filter(|e| e.kind == MakingEventKind::JobEnded)
        .map(|e| format!("{} {}", e.dot_name, e.detail["how"].as_str().unwrap()))
        .collect();
    assert_eq!(hows.len(), 3, "{hows:?}");
    for how in ["Alpha failed", "Beta cancelled", "Gamma failed"] {
        assert!(hows.iter().any(|h| h == how), "{hows:?}");
    }
    // Each is told once.
    settle(&t).await;
    assert_eq!(wakes(&t).await.len(), 3);
}

#[tokio::test]
async fn a_long_final_text_is_cut_at_8000_with_a_note() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Alpha").await;
    make(&t, &run, "Beta").await;
    assert_eq!(MAX_JOB_RESULT, 8000);

    // One character more than fits, of more than one byte each, and exactly what fits.
    done_job(&t, &run, "Alpha", &"é".repeat(8001)).await;
    done_job(&t, &run, "Beta", &"é".repeat(8000)).await;

    let said = wakes(&t).await;
    assert_eq!(said.len(), 2);
    assert_eq!(
        said[0],
        format!(
            "\"Alpha\" finished the job.\n\nIts final message:\n> {}\n[cut: the full text is in \
             the run]",
            "é".repeat(8000)
        )
    );
    assert_eq!(
        said[1],
        format!(
            "\"Beta\" finished the job.\n\nIts final message:\n> {}",
            "é".repeat(8000)
        )
    );
}

#[tokio::test]
async fn jobs_from_one_conversation_go_to_one_topic_of_the_made_dot() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let first = done_job(&t, &run, "Blog Watcher", "Three new posts.").await;
    let first_run = run_of(&t, &first).await;

    let second = give(&t, &run, "Blog Watcher", "Now sum them up.").await;

    // The made dot keeps the thread of the work: the second job goes on where the first ended,
    // as the person's next message in that topic would.
    let its = run_of(&t, &second).await;
    assert_eq!(its.parent_run_id.as_deref(), Some(first_run.id.as_str()));
    assert_eq!(its.root_run_id, first_run.root_run_id);
    assert_eq!(its.session_id, first_run.session_id);
    assert!(its.session_id.is_some());
    assert_eq!(second.dot_root_run_id, first.dot_root_run_id);
    assert_eq!(second.first_run_id.as_deref(), Some(its.id.as_str()));
    assert!(!is_typed(&its));
    // A run that goes on in a session sends only its message, which still says who sent it.
    assert_eq!(
        prompt(&made, &its),
        "Job from Researcher (the dot that made you). This is not a message from the person. \
         When you finish, your final message goes back to Researcher.\n\n> Now sum them up."
    );
    let thread = t.rt.store().list_thread(&its.root_run_id).await.unwrap();
    assert_eq!(thread.len(), 2, "{thread:#?}");
}

#[tokio::test]
async fn jobs_from_two_conversations_go_to_two_topics() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let one = live_run(&t, &maker).await;
    let two = live_run(&t, &maker).await;
    assert_ne!(one.root_run_id, two.root_run_id);
    make(&t, &one, "Blog Watcher").await;
    let first = done_job(&t, &one, "Blog Watcher", "Three new posts.").await;

    let second = give(&t, &two, "Blog Watcher", "Read the blogs now.").await;

    let its = run_of(&t, &second).await;
    assert_eq!(its.parent_run_id, None);
    assert_eq!(its.session_id, None);
    assert_ne!(second.dot_root_run_id, first.dot_root_run_id);
    assert_eq!(second.maker_root_run_id, two.root_run_id);
    // Each conversation of the maker has its own jobs, and its own log of them.
    assert_eq!(jobs(&t, &one).await.len(), 1);
    assert_eq!(jobs(&t, &two).await.len(), 1);
    assert_eq!(kinds(&t, &two).await, [MakingEventKind::JobStarted]);
}

#[tokio::test]
async fn a_job_for_a_busy_made_dot_waits_behind_its_run() {
    let t = running(1500).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    // The made dot is working on something else: the person started it.
    let other = t.rt.run_now(&made.id).await.unwrap();
    wait_status(t.rt.store(), &other.id, RunStatus::Running, 15).await;

    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;

    // The job's run waits in the queue behind the dot's run, as any run of a busy dot does.
    let its = run_of(&t, &job).await;
    assert_eq!(its.status, RunStatus::Queued);
    let other = t.rt.store().get_run(&other.id).await.unwrap();
    assert_eq!(other.status, RunStatus::Running);
    assert!(wakes(&t).await.is_empty());

    // The other run's end is not the job's: the job ends when its own run has.
    let job = ended(&t, &job).await;
    let other = t.rt.store().get_run(&other.id).await.unwrap();
    assert_eq!(other.status, RunStatus::Succeeded);
    let its = run_of(&t, &job).await;
    assert_eq!(its.status, RunStatus::Succeeded);
    assert!(its.started_at >= other.ended_at, "{its:#?} {other:#?}");
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" finished the job.\n\nIts final message:\n> fake done"]
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_job_while_the_last_one_from_this_conversation_is_open_is_refused() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;

    // Its jobs from here go to one topic, and a topic is one line of runs.
    let again = t.rt.makers().give_job(&run.id, "Blog Watcher", "And more.");
    assert_eq!(
        invalid(again.await),
        "\"Blog Watcher\" is still working on the last job from this conversation. You will be \
         woken when it ends."
    );
    // The same while that job waits for the person's answer.
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    question(&t, &its).await;
    finish(&t, &its.id, RunStatus::Succeeded, Some("Which?"), None).await;
    settle(&t).await;
    let again = t.rt.makers().give_job(&run.id, "Blog Watcher", "And more.");
    assert_eq!(
        invalid(again.await),
        "\"Blog Watcher\" is waiting for the person's answer on the last job from this \
         conversation. You will be woken when it ends."
    );
    assert_eq!(jobs(&t, &run).await.len(), 1);
    // Another conversation of the maker is not held by it: its job gets a topic of its own.
    let other = live_run(&t, &maker).await;
    give(&t, &other, "Blog Watcher", "Read the blogs now.").await;
}

#[tokio::test]
async fn a_job_that_needs_an_approval_ends_after_the_resumed_run_not_before() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;

    // The run parks on an approval: it has not ended.
    t.rt.store()
        .set_status(&its.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::Working);
    assert!(wakes(&t).await.is_empty());

    // The person approves: the store closes the parked run as succeeded and queues the run
    // that resumes it, in one transaction, as the runner does.
    let mut child = NewRun::new(&made.id, TriggerKind::Resume);
    child.payload = Some(json!({"message": "Approved.", "approvals": []}));
    child.parent_run_id = Some(its.id.clone());
    child.root_run_id = Some(its.root_run_id.clone());
    child.session_id = its.session_id.clone();
    let child =
        t.rt.store()
            .resume_parked(&its.id, &[], &[], &[], Some(&child))
            .await
            .unwrap()
            .expect("the run that resumes it");
    let parked = t.rt.store().get_run(&its.id).await.unwrap();
    assert_eq!(parked.status, RunStatus::Succeeded);
    settle(&t).await;

    // The job goes on in the resumed run: the parked run's end is not the job's.
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Working);
    assert_eq!(now.last_run_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(now.first_run_id.as_deref(), Some(its.id.as_str()));
    assert!(wakes(&t).await.is_empty());
    assert_eq!(
        kinds(&t, &run).await,
        [MakingEventKind::Made, MakingEventKind::JobStarted]
    );

    start(&t, &child.id).await;
    settle(&t).await;
    assert!(wakes(&t).await.is_empty());
    finish(&t, &child.id, RunStatus::Succeeded, Some("All read."), None).await;
    settle(&t).await;
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Ended);
    assert_eq!(now.last_run_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" finished the job.\n\nIts final message:\n> All read."]
    );
    let events = log(&t, &run).await;
    assert_eq!(events[2].kind, MakingEventKind::JobEnded);
    assert_eq!(events[2].detail["run_id"], child.id.as_str());
}

#[tokio::test]
async fn a_job_whose_approval_is_refused_ends_with_the_parked_run() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    // The run asks to fetch a page and parks on it; the person refuses, with no note.
    let approval =
        t.rt.store()
            .create_approval(&its.id, "WebFetch", &json!({"url": "https://example.com"}))
            .await
            .unwrap();
    t.rt.store()
        .finish_run(
            &its.id,
            RunStatus::AwaitingApproval,
            Some("I need the web."),
            None,
        )
        .await
        .unwrap();
    t.rt.store()
        .decide_approval_with_parked(&approval.id, false, None, true)
        .await
        .unwrap();

    // A plain refusal closes the parked run with no run after it.
    let child = t.rt.store().resume_parked(&its.id, &[], &[], &[], None);
    assert!(child.await.unwrap().is_none());
    settle(&t).await;

    // The maker is told the person refused, not that the job was done.
    let job = stored(&t, &job).await;
    assert_eq!(job.status, JobStatus::Ended);
    assert_eq!(job.ended_how, Some(JobEnd::Cancelled));
    assert_eq!(
        wakes(&t).await,
        [
            "\"Blog Watcher\" was stopped: the person refused what it asked to do.\n\nIts final \
          message:\n> I need the web."
        ]
    );
}

#[tokio::test]
async fn a_job_that_ends_on_a_question_waits_and_the_answer_carries_it_on() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;

    // The made dot asks the person and ends its turn.
    let asked = question(&t, &its).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;

    // The job waits with the question: the maker is not woken, and its topic says who waits.
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::WaitingAnswer);
    assert_eq!(now.last_run_id.as_deref(), Some(its.id.as_str()));
    assert_eq!(now.wake_id, None);
    assert!(wakes(&t).await.is_empty());
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Made,
            MakingEventKind::JobStarted,
            MakingEventKind::JobWaiting
        ]
    );
    assert_eq!(events[2].dot_id.as_deref(), Some(made.id.as_str()));
    assert_eq!(events[2].dot_name, "Blog Watcher");
    assert_eq!(events[2].run_id, run.id);
    assert_eq!(
        events[2].detail,
        json!({"job_id": job.id, "run_id": its.id})
    );
    // For as long as nobody answers, and it is said once.
    settle(&t).await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::WaitingAnswer);
    assert_eq!(kinds(&t, &run).await.len(), 3);
    assert!(wakes(&t).await.is_empty());

    // The person answers: the run that carries the answer is the job's next run.
    let answer = t.rt.reply(&asked.id, "The first").await.unwrap();
    settle(&t).await;
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Working);
    assert_eq!(now.last_run_id.as_deref(), Some(answer.id.as_str()));
    assert!(wakes(&t).await.is_empty());
    // An answer is not a message the person typed in a chat.
    assert!(!is_typed(&answer));

    // That run ends with no question waiting: the job ends, and the maker is woken once.
    start(&t, &answer.id).await;
    settle(&t).await;
    assert!(wakes(&t).await.is_empty());
    finish(
        &t,
        &answer.id,
        RunStatus::Succeeded,
        Some("Read the first."),
        None,
    )
    .await;
    settle(&t).await;
    settle(&t).await;
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Ended);
    assert_eq!(now.ended_how, Some(JobEnd::Succeeded));
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" finished the job.\n\nIts final message:\n> Read the first."]
    );
    assert_eq!(
        kinds(&t, &run).await,
        [
            MakingEventKind::Made,
            MakingEventKind::JobStarted,
            MakingEventKind::JobWaiting,
            MakingEventKind::JobEnded
        ]
    );
}

#[tokio::test]
async fn a_job_whose_answer_run_asks_again_waits_again() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;
    let asked = question(&t, &its).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;

    // The answer's run asks a second question and ends its turn.
    let answer = t.rt.reply(&asked.id, "The first").await.unwrap();
    let answer = start(&t, &answer.id).await;
    let again = question(&t, &answer).await;
    finish(
        &t,
        &answer.id,
        RunStatus::Succeeded,
        Some("And then?"),
        None,
    )
    .await;
    settle(&t).await;

    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::WaitingAnswer);
    assert_eq!(now.last_run_id.as_deref(), Some(answer.id.as_str()));
    assert!(wakes(&t).await.is_empty());
    let waiting = kinds(&t, &run).await;
    let waiting = waiting
        .iter()
        .filter(|k| **k == MakingEventKind::JobWaiting);
    assert_eq!(waiting.count(), 2);

    let last = t.rt.reply(&again.id, "The second").await.unwrap();
    start(&t, &last.id).await;
    finish(&t, &last.id, RunStatus::Succeeded, Some("Both read."), None).await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::Ended);
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" finished the job.\n\nIts final message:\n> Both read."]
    );
}

#[tokio::test]
async fn stopping_a_job_that_waits_for_an_answer_wakes_the_maker() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;
    let asked = question(&t, &its).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::WaitingAnswer);

    // The person dismisses the question in the made dot's topic: it can no longer be answered,
    // so nothing would ever end the job.
    t.rt.checkins().dismiss(&asked.id).await.unwrap();
    settle(&t).await;

    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Ended);
    assert_eq!(now.ended_how, Some(JobEnd::Cancelled));
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" was stopped.\n\nIts final message:\n> Which first?"]
    );
    let events = log(&t, &run).await;
    let last = events.last().unwrap();
    assert_eq!(last.kind, MakingEventKind::JobEnded);
    assert_eq!(last.detail["how"], "cancelled");
    settle(&t).await;
    assert_eq!(wakes(&t).await.len(), 1);
}

#[tokio::test]
async fn a_follow_up_the_made_dot_schedules_is_not_part_of_the_job() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;

    // The made dot schedules a run of its own for later, and ends its turn.
    let later =
        t.rt.follow_ups()
            .schedule(&its, "Look again", When::In(TimeDelta::hours(1)))
            .await
            .unwrap();
    let its = finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Read; I look again later."),
        None,
    )
    .await;
    settle(&t).await;

    // The job has ended: the maker does not wait for the follow-up.
    let ended = stored(&t, &job).await;
    assert_eq!(ended.status, JobStatus::Ended);
    let said = "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Read; I look again \
                later.";
    assert_eq!(wakes(&t).await, [said]);

    // The follow-up's run goes on in the made dot's topic after the job's run, and ends: the
    // job is where it was, and the maker is told nothing more.
    let mut new = NewRun::new(&made.id, TriggerKind::FollowUp);
    new.parent_run_id = Some(its.id.clone());
    new.root_run_id = Some(its.root_run_id.clone());
    new.session_id = its.session_id.clone();
    new.payload = Some(json!({"message": "Follow-up: Look again", "follow_up_id": later.id}));
    let follow = t.rt.store().create_run(&new).await.unwrap();
    settle(&t).await;
    start(&t, &follow.id).await;
    finish(
        &t,
        &follow.id,
        RunStatus::Succeeded,
        Some("Nothing new."),
        None,
    )
    .await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await, ended);
    assert_eq!(wakes(&t).await, [said]);
    assert_eq!(jobs(&t, &run).await.len(), 1);
}

#[tokio::test]
async fn a_follow_up_run_after_a_job_run_that_still_works_does_not_take_the_job() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;
    let asked = question(&t, &its).await;
    let its = finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;

    // While the job waits for the answer, a follow-up of the made dot runs in the topic.
    let mut new = NewRun::new(&made.id, TriggerKind::FollowUp);
    new.parent_run_id = Some(its.id.clone());
    new.root_run_id = Some(its.root_run_id.clone());
    new.session_id = its.session_id.clone();
    new.payload = Some(json!({"message": "Follow-up: Look again"}));
    let follow = t.rt.store().create_run(&new).await.unwrap();
    start(&t, &follow.id).await;
    finish(
        &t,
        &follow.id,
        RunStatus::Succeeded,
        Some("Nothing new."),
        None,
    )
    .await;
    settle(&t).await;

    // The job still waits, at the run that asked.
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::WaitingAnswer);
    assert_eq!(now.last_run_id.as_deref(), Some(its.id.as_str()));
    assert!(wakes(&t).await.is_empty());
    // The answer still carries it on.
    let answer = t.rt.reply(&asked.id, "The first").await.unwrap();
    settle(&t).await;
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Working);
    assert_eq!(now.last_run_id.as_deref(), Some(answer.id.as_str()));
}

/// Six jobs given from `run` to "Blog Watcher", each ended before the next.
async fn six_jobs(t: &T, run: &Run) {
    assert_eq!(MAX_JOBS_BETWEEN_TYPED, 6);
    for n in 0..6 {
        done_job(t, run, "Blog Watcher", &format!("Round {n} read.")).await;
    }
}

#[tokio::test]
async fn the_seventh_job_since_the_person_wrote_is_refused() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    six_jobs(&t, &run).await;

    let seventh =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);

    // Nothing of it exists: no job, no run of the made dot, no card.
    let all = jobs(&t, &run).await;
    assert_eq!(all.len(), 6);
    let topic = all[0].dot_root_run_id.clone().unwrap();
    assert_eq!(t.rt.store().list_thread(&topic).await.unwrap().len(), 6);
    let started = kinds(&t, &run).await;
    let started = started
        .iter()
        .filter(|k| **k == MakingEventKind::JobStarted);
    assert_eq!(started.count(), 6);
    // Another topic of the maker has its own count.
    let other = live_run(&t, &maker).await;
    give(&t, &other, "Blog Watcher", "Read the blogs now.").await;
}

#[tokio::test]
async fn deleting_the_made_dot_does_not_give_jobs_back() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    six_jobs(&t, &run).await;

    // Its jobs go with the dot; the count is of the log, which stays.
    t.rt.makers()
        .delete_dot(&run.id, "Blog Watcher")
        .await
        .unwrap();
    assert!(jobs(&t, &run).await.is_empty());
    make(&t, &run, "Feed Reader").await;
    let seventh = t.rt.makers().give_job(&run.id, "Feed Reader", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);
}

#[tokio::test]
async fn a_typed_message_resets_the_count() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    six_jobs(&t, &run).await;
    let seventh =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);

    // The maker reports, and the person writes in the topic: carry on.
    let run = finish(
        &t,
        &run.id,
        RunStatus::Succeeded,
        Some("Six rounds done."),
        None,
    )
    .await;
    let typed = next_run(&t, &run, json!({"message": "carry on", "typed": true})).await;
    assert!(is_typed(&typed));

    for n in 0..6 {
        done_job(&t, &typed, "Blog Watcher", &format!("Round {n} read.")).await;
    }
    let again =
        t.rt.makers()
            .give_job(&typed.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(again.await), GUARD);
    assert_eq!(jobs(&t, &run).await.len(), 12);
}

#[tokio::test]
async fn an_answer_or_a_wake_does_not_reset_the_count() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    six_jobs(&t, &run).await;
    let run = finish(
        &t,
        &run.id,
        RunStatus::Succeeded,
        Some("Six rounds done."),
        None,
    )
    .await;

    // The maker is woken with the first job's end: the wake's run is no message of the person.
    t.rt.makers().deliver_wakes().await.unwrap();
    let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
    assert_eq!(thread.len(), 2, "{thread:#?}");
    let woken = start(&t, &thread[1].id).await;
    assert!(woken.payload.as_ref().unwrap().get("wake_id").is_some());
    let seventh =
        t.rt.makers()
            .give_job(&woken.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);

    // The maker asks the person something, and they answer: an answer is not typed either.
    let asked = question(&t, &woken).await;
    finish(&t, &woken.id, RunStatus::Succeeded, Some("Go on?"), None).await;
    let answer = t.rt.reply(&asked.id, "Yes").await.unwrap();
    let answer = start(&t, &answer.id).await;
    assert!(!is_typed(&answer));
    let seventh =
        t.rt.makers()
            .give_job(&answer.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);

    // Nor is the answer to a card: the run that follows a decided make request is a wake.
    let mut beyond = ask("Feed Reader");
    beyond.wanted.browser = Some(true);
    let outcome = t.rt.makers().make_dot(&answer.id, beyond).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    finish(&t, &answer.id, RunStatus::Succeeded, Some("Asked."), None).await;
    t.rt.makers()
        .decide_request(&request.id, MakeDecision::Refuse, None, None)
        .await
        .unwrap();
    let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
    let last = thread.last().unwrap();
    assert_eq!(last.status, RunStatus::Queued, "{thread:#?}");
    let last = start(&t, &last.id).await;
    let seventh =
        t.rt.makers()
            .give_job(&last.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);
    assert_eq!(jobs(&t, &run).await.len(), 6);
}

#[tokio::test]
async fn a_scheduled_topic_counts_from_its_start() {
    let t = quiet().await;
    let maker = maker(&t).await;
    // A topic the schedule started: the person typed nothing in it.
    let run = live_run_as(&t, &maker, TriggerKind::Schedule).await;
    assert!(!is_typed(&run));
    make(&t, &run, "Blog Watcher").await;
    six_jobs(&t, &run).await;

    let seventh =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);

    // A wake in that topic counts on from the trigger.
    let run = finish(
        &t,
        &run.id,
        RunStatus::Succeeded,
        Some("Six rounds done."),
        None,
    )
    .await;
    let woken = next_run(&t, &run, json!({"message": "It finished.", "wake_id": "w"})).await;
    let seventh =
        t.rt.makers()
            .give_job(&woken.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(seventh.await), GUARD);
}

#[tokio::test]
async fn a_first_job_starts_when_the_dot_is_made() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut asked = ask("Blog Watcher");
    asked.first_job = Some("  Read them now and tell me what is new. ".into());

    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();

    assert_eq!(
        outcome.tool_text(),
        "Made the dot \"Blog Watcher\". It is on and ready; give it a job with give_job. Its \
         first job has started; you will be woken with the result."
    );
    let MakeOutcome::Made { dot, first_job } = outcome else {
        panic!("not made: {outcome:?}");
    };
    let all = jobs(&t, &run).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    let job = &all[0];
    assert_eq!(first_job, Some(FirstJob::Started(job.clone())));
    assert_eq!(job.dot_id, dot.id);
    assert_eq!(job.maker_run_id, run.id);
    assert_eq!(job.message, "Read them now and tell me what is new.");
    assert_eq!(job.status, JobStatus::Working);
    let its = run_of(&t, job).await;
    assert_eq!(its.dot_id, dot.id);
    assert_eq!(its.status, RunStatus::Queued);
    assert_eq!(message(&its), "Read them now and tell me what is new.");
    assert_eq!(
        kinds(&t, &run).await,
        [MakingEventKind::Made, MakingEventKind::JobStarted]
    );
    // A dot made with no first job says nothing of one.
    let outcome = t.rt.makers().make_dot(&run.id, ask("Feed Reader")).await;
    let outcome = outcome.unwrap();
    assert_eq!(
        outcome.tool_text(),
        "Made the dot \"Feed Reader\". It is on and ready; give it a job with give_job."
    );
    assert!(matches!(
        outcome,
        MakeOutcome::Made {
            first_job: None,
            ..
        }
    ));
}

#[tokio::test]
async fn a_first_job_starts_when_the_person_allows_the_request() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut asked = ask("Blog Watcher");
    asked.wanted.browser = Some(true);
    asked.first_job = Some("Read them now.".into());
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    // Nothing is made yet, so nothing is started.
    assert!(jobs(&t, &run).await.is_empty());

    let request =
        t.rt.makers()
            .decide_request(
                &request.id,
                MakeDecision::Allow,
                None,
                shown(&t, &request).await.as_deref(),
            )
            .await
            .unwrap();

    let dot_id = request.made_dot_id.clone().expect("the dot was made");
    let all = jobs(&t, &run).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    let job = &all[0];
    assert_eq!(job.dot_id, dot_id);
    assert_eq!(job.maker_run_id, run.id);
    assert_eq!(job.maker_root_run_id, run.root_run_id);
    assert_eq!(job.message, "Read them now.");
    let its = run_of(&t, job).await;
    assert_eq!(its.dot_id, dot_id);
    assert_eq!(its.status, RunStatus::Queued);
    // The maker's run is still going, so the wake with the answer waits.
    assert_eq!(
        wakes(&t).await,
        [
            "The person allowed it: the dot \"Blog Watcher\" was made as you asked. Its first job \
          has started; you will be woken with the result."
        ]
    );
    assert_eq!(
        kinds(&t, &run).await,
        [
            MakingEventKind::Asked,
            MakingEventKind::Made,
            MakingEventKind::JobStarted,
            MakingEventKind::Answered
        ]
    );
}

#[tokio::test]
async fn a_first_job_past_the_guard_leaves_the_dot_made() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    six_jobs(&t, &run).await;
    let mut asked = ask("Feed Reader");
    asked.first_job = Some("Read them now.".into());

    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();

    assert_eq!(
        outcome.tool_text(),
        format!(
            "Made the dot \"Feed Reader\". It is on and ready; give it a job with give_job. Its \
             first job was not started: {GUARD}"
        )
    );
    let MakeOutcome::Made { dot, first_job } = outcome else {
        panic!("not made: {outcome:?}");
    };
    assert_eq!(first_job, Some(FirstJob::NotStarted(GUARD.to_string())));
    assert!(t.rt.store().get_dot(&dot.id).await.is_ok());
    assert_eq!(jobs(&t, &run).await.len(), 6);
    assert!(t
        .rt
        .store()
        .list_runs(Some(&dot.id), 10)
        .await
        .unwrap()
        .is_empty());

    // The same when the person allows a request that carried a first job.
    let mut beyond = ask("Mail Reader");
    beyond.wanted.browser = Some(true);
    beyond.first_job = Some("Read them now.".into());
    let outcome = t.rt.makers().make_dot(&run.id, beyond).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    let before = wakes(&t).await.len();
    let request =
        t.rt.makers()
            .decide_request(
                &request.id,
                MakeDecision::Allow,
                None,
                shown(&t, &request).await.as_deref(),
            )
            .await
            .unwrap();
    assert!(request.made_dot_id.is_some());
    let said = wakes(&t).await;
    assert_eq!(said.len(), before + 1);
    assert_eq!(
        said.last().unwrap(),
        &format!(
            "The person allowed it: the dot \"Mail Reader\" was made as you asked. Its first \
             job was not started: {GUARD}"
        )
    );
    assert_eq!(jobs(&t, &run).await.len(), 6);
}

#[tokio::test]
async fn only_its_own_made_dots_take_jobs() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    // Another maker's made dot, and a dot the person made.
    let writer = maker_named(&t, "Writer").await;
    let writers_run = live_run(&t, &writer).await;
    make(&t, &writers_run, "Other").await;
    maker_named(&t, "Mine").await;

    for name in ["Other", "Mine", "Writer", "Researcher", "Nobody"] {
        let refused = t.rt.makers().give_job(&run.id, name, "Read the blogs now.");
        assert_eq!(
            invalid(refused.await),
            format!(
                "Researcher did not make a dot named \"{name}\". list_made_dots shows the dots it \
                 made."
            )
        );
    }
    assert!(jobs(&t, &run).await.is_empty());
    let runs = t.rt.store().list_runs(None, 50).await.unwrap();
    assert_eq!(runs.len(), 2, "{runs:#?}");
    // The other maker gives its own dot a job.
    let text = t.rt.makers().give_job(&writers_run.id, "Other", "Write.");
    assert_eq!(text.await.unwrap(), GAVE.replace("Blog Watcher", "Other"));
}

#[tokio::test]
async fn a_switched_off_made_dot_takes_no_job() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    t.rt.set_dot_enabled(&made.id, false).await.unwrap();

    let refused = t.rt.makers().give_job(&run.id, "Blog Watcher", "Read.");
    assert_eq!(
        invalid(refused.await),
        "\"Blog Watcher\" is switched off. The person turns it on."
    );
    assert!(jobs(&t, &run).await.is_empty());

    t.rt.set_dot_enabled(&made.id, true).await.unwrap();
    give(&t, &run, "Blog Watcher", "Read.").await;
}

#[tokio::test]
async fn the_job_text_is_1_to_4000_characters() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;

    for job in [String::new(), "  \n ".to_string(), "é".repeat(4001)] {
        let refused = t.rt.makers().give_job(&run.id, "Blog Watcher", &job);
        assert_eq!(invalid(refused.await), "The job is 1 to 4000 characters.");
    }
    assert!(jobs(&t, &run).await.is_empty());
    // The spaces around it do not count.
    let longest = format!("  {}  ", "é".repeat(4000));
    let job = give(&t, &run, "Blog Watcher", &longest).await;
    assert_eq!(job.message, "é".repeat(4000));
}

#[tokio::test]
async fn a_dot_that_may_not_make_dots_a_step_run_and_an_ended_run_give_no_job() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;

    // A workflow step's run.
    let step = live_run(&t, &maker).await;
    // Named a step without building a workflow: the step row itself does not matter here.
    let mut conn = SqliteConnectOptions::new()
        .filename(t.dir.path().join("data").join("dots.db"))
        .foreign_keys(false)
        .connect()
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET workflow_step_run_id = 'STEP' WHERE id = ?")
        .bind(&step.id)
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    let refused = t.rt.makers().give_job(&step.id, "Blog Watcher", "Read.");
    assert_eq!(invalid(refused.await), NOT_IN_A_STEP);

    // A run that has ended.
    let over = live_run(&t, &maker).await;
    finish(&t, &over.id, RunStatus::Succeeded, None, None).await;
    let refused = t.rt.makers().give_job(&over.id, "Blog Watcher", "Read.");
    assert_eq!(invalid(refused.await), RUN_ENDED);

    // The switch is off.
    let mut off = maker.spec.clone();
    off.making.on = false;
    t.rt.update_dot(&maker.id, off).await.unwrap();
    let refused = t.rt.makers().give_job(&run.id, "Blog Watcher", "Read.");
    assert_eq!(
        invalid(refused.await),
        "Researcher may not make dots. The person turns that on in the dot's settings."
    );
    assert!(jobs(&t, &run).await.is_empty());
}

#[tokio::test]
async fn a_job_open_at_startup_whose_run_ended_is_settled() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    // Dots closes; the run's end was stored, and nothing settled its job.
    t.rt.shutdown();
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Three new posts."),
        None,
    )
    .await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::Working);
    let T { dir, secrets, rt } = t;
    drop(rt);

    let rt = runtime(dir.path(), &secrets, false, 0).await;
    let t = T { dir, secrets, rt };

    // Nobody calls anything: the pass at start settles it.
    let job = ended(&t, &job).await;
    assert_eq!(job.ended_how, Some(JobEnd::Succeeded));
    let wake_id = job.wake_id.clone().expect("the job names its wake");
    let wake = t.rt.store().get_making_wake(&wake_id).await.unwrap();
    assert_eq!(
        wake.message,
        "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts."
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_job_run_interrupted_by_a_restart_goes_on_in_the_run_that_sends_it_again() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    // Dots closes while the job's run is running.
    t.rt.shutdown();
    let T { dir, secrets, rt } = t;
    drop(rt);

    let rt = runtime(dir.path(), &secrets, false, 0).await;
    let t = T { dir, secrets, rt };

    // At start the runner fails the interrupted run and queues its message again: that run
    // is the job's now, and the maker is not told the job failed.
    let deadline = Instant::now() + Duration::from_secs(30);
    let again = loop {
        let now = run_of(&t, &job).await;
        if now.id != its.id {
            break now;
        }
        assert!(Instant::now() < deadline, "the job did not move on");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let interrupted = t.rt.store().get_run(&its.id).await.unwrap();
    assert_eq!(interrupted.status, RunStatus::Failed);
    assert_eq!(again.retry_of.as_deref(), Some(its.id.as_str()));
    assert_eq!(again.status, RunStatus::Queued);
    let now = stored(&t, &job).await;
    assert_eq!(now.status, JobStatus::Working);
    assert_eq!(now.first_run_id.as_deref(), Some(its.id.as_str()));
    assert!(wakes(&t).await.is_empty());

    start(&t, &again.id).await;
    finish(
        &t,
        &again.id,
        RunStatus::Succeeded,
        Some("Three new posts."),
        None,
    )
    .await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::Ended);
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts."]
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn the_job_run_is_background_when_the_makers_run_was() {
    let t = quiet().await;
    let maker = maker(&t).await;
    // A run the schedule started is background; a chat the person started is not.
    let scheduled = live_run_as(&t, &maker, TriggerKind::Schedule).await;
    let chat = live_run(&t, &maker).await;
    assert!(run_is_background(&scheduled));
    assert!(!run_is_background(&chat));
    make(&t, &scheduled, "Blog Watcher").await;

    let job = give(&t, &scheduled, "Blog Watcher", "Read the blogs now.").await;
    let background = run_of(&t, &job).await;
    assert_eq!(background.trigger, TriggerKind::Chat);
    assert_eq!(background.payload.as_ref().unwrap()["background"], true);
    assert!(run_is_background(&background));
    let job = give(&t, &chat, "Blog Watcher", "Read the blogs now.").await;
    let foreground = run_of(&t, &job).await;
    assert_eq!(foreground.payload.as_ref().unwrap().get("background"), None);
    assert!(!run_is_background(&foreground));

    // The usage holds that exist treat them so: with Claude's 5-hour usage past the share kept
    // for the person, the background job's run is held and the other is not.
    t.rt.limits().set_shares(HoldShares {
        enabled: true,
        five_hour: 60,
        weekly: 70,
    });
    let now = Utc::now();
    let reading = LimitReading {
        window: "five_hour".into(),
        utilization: 0.95,
        resets_at: timestamp(now + TimeDelta::hours(3)),
        status: "allowed".into(),
        read_at: timestamp(now),
    };
    t.rt.limits()
        .record("allowed", vec![reading])
        .await
        .unwrap();
    let held = t.rt.limits_get().await.unwrap().held_run_ids;
    assert!(held.contains(&background.id), "{held:?}");
    assert!(!held.contains(&foreground.id), "{held:?}");

    // A first job is classed the same way.
    let mut asked = ask("Feed Reader");
    asked.first_job = Some("Read them now.".into());
    let outcome = t.rt.makers().make_dot(&scheduled.id, asked).await.unwrap();
    let MakeOutcome::Made {
        first_job: Some(FirstJob::Started(job)),
        ..
    } = outcome
    else {
        panic!("no first job: {outcome:?}");
    };
    assert!(run_is_background(&run_of(&t, &job).await));
}

#[tokio::test]
async fn deleting_a_made_dot_ends_its_waiting_job_and_wakes_the_maker() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;
    question(&t, &its).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::WaitingAnswer);

    // A question that waits is no run going: the maker may delete the dot.
    t.rt.makers()
        .delete_dot(&run.id, "Blog Watcher")
        .await
        .unwrap();

    assert!(jobs(&t, &run).await.is_empty());
    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" was deleted before the job ended."]
    );
    let events = log(&t, &run).await;
    let ended: Vec<&MakingEvent> = events
        .iter()
        .filter(|e| e.kind == MakingEventKind::JobEnded)
        .collect();
    assert_eq!(ended.len(), 1, "{events:#?}");
    assert_eq!(ended[0].dot_id, None);
    assert_eq!(ended[0].dot_name, "Blog Watcher");
    assert_eq!(
        ended[0].detail,
        json!({"job_id": job.id, "run_id": its.id, "how": "deleted"})
    );
    settle(&t).await;
    assert_eq!(wakes(&t).await.len(), 1);
}

#[tokio::test]
async fn the_list_says_which_made_dot_works_on_a_job_from_this_conversation() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    for name in ["Alpha", "Beta", "Gamma"] {
        make(&t, &run, name).await;
    }
    give(&t, &run, "Alpha", "Read the blogs now.").await;
    let waiting = give(&t, &run, "Beta", "Read the blogs now.").await;
    let its = run_of(&t, &waiting).await;
    let its = start(&t, &its.id).await;
    question(&t, &its).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;
    // A job from another conversation of the maker is not this one's.
    let other = live_run(&t, &maker).await;
    give(&t, &other, "Gamma", "Read the blogs now.").await;

    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();

    let said: Vec<(&str, Option<&str>)> = lines
        .iter()
        .map(|line| (line.name.as_str(), line.job.as_deref()))
        .collect();
    assert_eq!(
        said,
        [
            ("Alpha", Some("working on a job from this conversation")),
            ("Beta", Some("waiting for the person's answer")),
            ("Gamma", None),
        ]
    );
    let text = made_dots_text("Researcher", &lines, &[]);
    let text: Vec<&str> = text.lines().collect();
    assert_eq!(
        text[0],
        "\"Alpha\": Reads the feeds; no schedule; running; no run has ended yet; working on a job \
         from this conversation"
    );
    assert!(
        text[1].ends_with("; waiting for the person's answer"),
        "{text:?}"
    );
    assert!(
        text[2].ends_with("; running; no run has ended yet"),
        "{text:?}"
    );
}

/// A topic the person never typed in is unattended from start to end (dots-make-dots design
/// section 8): the wake a schedule's job brings is background, and so is the job that wake
/// gives, for the usage holds and the role hold.
#[tokio::test]
async fn a_wake_and_its_job_in_a_topic_the_person_never_typed_in_are_background() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let scheduled = live_run_as(&t, &maker, TriggerKind::Schedule).await;
    make(&t, &scheduled, "Blog Watcher").await;
    done_job(&t, &scheduled, "Blog Watcher", "Three new posts.").await;
    let scheduled = finish(
        &t,
        &scheduled.id,
        RunStatus::Succeeded,
        Some("Gave it."),
        None,
    )
    .await;

    t.rt.makers().deliver_wakes().await.unwrap();
    let thread =
        t.rt.store()
            .list_thread(&scheduled.root_run_id)
            .await
            .unwrap();
    assert_eq!(thread.len(), 2, "{thread:#?}");
    let woken = start(&t, &thread[1].id).await;
    assert!(woken.payload.as_ref().unwrap().get("wake_id").is_some());
    assert!(run_is_background(&woken), "{woken:#?}");

    let job = give(&t, &woken, "Blog Watcher", "Now sum them up.").await;
    assert!(run_is_background(&run_of(&t, &job).await));

    // A topic the person typed in is watched: its wake and its jobs are not background.
    let chat = live_run(&t, &maker).await;
    make(&t, &chat, "Feed Reader").await;
    done_job(&t, &chat, "Feed Reader", "Two new posts.").await;
    let chat = finish(&t, &chat.id, RunStatus::Succeeded, Some("Gave it."), None).await;
    t.rt.makers().deliver_wakes().await.unwrap();
    let thread = t.rt.store().list_thread(&chat.root_run_id).await.unwrap();
    let woken = start(&t, &thread[1].id).await;
    assert!(!run_is_background(&woken), "{woken:#?}");
    let job = give(&t, &woken, "Feed Reader", "Now sum them up.").await;
    assert!(!run_is_background(&run_of(&t, &job).await));
}

/// A failed job run that the person sends again after its job ended is no job any more
/// (dots-make-dots design section 8): nobody would be woken with its result, so it does not
/// say it goes back to the maker. Its text is still the maker's, not the person's: it says
/// so (`FROM_MAKER`), is quoted, and is not typed.
#[tokio::test]
async fn a_job_run_sent_again_after_its_job_ended_carries_no_job() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    // It reached the dot, then failed: Send again is the person's way to try it once more.
    t.rt.store()
        .set_delivered(&its.id, Utc::now())
        .await
        .unwrap();
    finish(&t, &its.id, RunStatus::Failed, None, Some("boom")).await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::Ended);

    let again = t.rt.resend(&its.id).await.unwrap();

    assert_eq!(again.retry_of.as_deref(), Some(its.id.as_str()));
    let payload = again.payload.clone().unwrap();
    assert_eq!(
        payload,
        json!({"message": "Read the blogs now.", "from_maker": "Researcher"})
    );
    assert_the_makers_words(&made, &again);
    settle(&t).await;
    assert_eq!(wakes(&t).await.len(), 1);
    assert_eq!(
        stored(&t, &job).await.last_run_id.as_deref(),
        Some(its.id.as_str())
    );
}

/// The run `again` carries a job's text sent again by the person after its job ended: its
/// prompt quotes the maker's words under the note that says so, and it is not typed.
fn assert_the_makers_words(made: &Dot, again: &Run) {
    assert!(!is_typed(again), "{again:#?}");
    let said = prompt(made, again);
    assert!(
        said.contains(&format!(
            "## Job\n\n{}\n\n> Read the blogs now.",
            resent_job_note("Researcher")
        )),
        "{said}"
    );
    assert!(!said.contains("## Message from the user"), "{said}");
}

/// A job run that failed before it reached the dot has Retry, not Send again: what Retry sends
/// is the maker's words too, not a message the person typed.
#[tokio::test]
async fn a_job_run_retried_after_it_did_not_reach_the_dot_is_still_the_makers_words() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    let its = finish(&t, &its.id, RunStatus::Failed, None, Some("boom")).await;
    assert!(its.undelivered_at.is_some(), "{its:#?}");
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::Ended);

    let again = t.rt.retry_delivery(&its.id).await.unwrap();

    assert_eq!(
        again.payload.clone().unwrap(),
        json!({"message": "Read the blogs now.", "from_maker": "Researcher"})
    );
    assert_the_makers_words(&made, &again);
    settle(&t).await;
    assert_eq!(wakes(&t).await.len(), 1);
}

/// A job whose run is being sent again is not ended under it (dots-make-dots design section 8):
/// a pass that read the job's run before the new run was stored cannot end the job once it is
/// stored, so the job goes on in the new run and its end wakes the maker.
#[tokio::test]
async fn a_job_is_not_ended_while_a_run_sends_its_message_again() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    t.rt.store()
        .set_delivered(&its.id, Utc::now())
        .await
        .unwrap();
    finish(&t, &its.id, RunStatus::Failed, None, Some("boom")).await;
    // Sent again before any pass settled the job: it carries the job on.
    let again = t.rt.resend(&its.id).await.unwrap();
    assert_eq!(again.payload.as_ref().unwrap()[JOB_ID], job.id);

    // A pass that read the job before the new run was stored ends it now: refused.
    let ended =
        t.rt.store()
            .end_making_job(&job.id, JobEnd::Failed, "\"Blog Watcher\" failed.")
            .await
            .unwrap();

    assert_eq!(ended, None);
    assert_eq!(stored(&t, &job).await.status, JobStatus::Working);
    assert!(wakes(&t).await.is_empty());
    settle(&t).await;
    assert_eq!(
        stored(&t, &job).await.last_run_id.as_deref(),
        Some(again.id.as_str())
    );
}

/// A job whose run waits for a spending cap that does not hold its maker, here the made dot's
/// own, tells the maker once that it has not started (dots-make-dots design section 8), so the
/// maker does not take it for working; the job stays open and its end wakes the maker as
/// before.
#[tokio::test]
async fn a_job_held_by_a_cap_is_reported_to_the_maker_once() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    // The person gives the made dot a daily cap of its own, and a run of it spends it.
    let mut s = t.rt.store().get_dot(&made.id).await.unwrap().spec;
    s.spend_cap.daily_usd = Some(0.5);
    t.rt.update_dot(&made.id, s).await.unwrap();
    let spent =
        t.rt.store()
            .create_run(&NewRun::new(&made.id, TriggerKind::Schedule))
            .await
            .unwrap();
    start(&t, &spent.id).await;
    t.rt.store()
        .add_usage(&spent.id, 10, 10, 0, Some(0.6))
        .await
        .unwrap();
    finish(&t, &spent.id, RunStatus::Succeeded, Some("Done."), None).await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;

    settle(&t).await;
    settle(&t).await;

    let held = "\"Blog Watcher\" has not started the job: Blog Watcher reached its $0.50 daily \
                cap. It starts by itself when it may; you will be woken when the job ends.";
    assert_eq!(wakes(&t).await, [held]);
    assert_eq!(stored(&t, &job).await.status, JobStatus::Working);

    // It ends later: the maker is woken with that too.
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Three new posts."),
        None,
    )
    .await;
    settle(&t).await;
    assert_eq!(
        wakes(&t).await,
        [
            held,
            "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts."
        ]
    );
}

/// A job held by the cap its maker shares with the dots it made is not told to the maker
/// (dots-make-dots design section 8): the wake would be held by the same cap until the job could
/// start anyway, and meanwhile it would keep the maker's topic busy. The person can still write
/// there, and no wake run waits in it.
#[tokio::test]
async fn a_job_held_by_the_makers_own_cap_is_not_reported() {
    let t = quiet().await;
    let mut s = spec("Researcher");
    s.workdir = t.dir.path().join("work").to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.making.on = true;
    s.spend_cap.daily_usd = Some(1.0);
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    // The maker's own run spent its cap, which it shares with the dots it made.
    t.rt.store()
        .add_usage(&run.id, 10, 10, 0, Some(1.0))
        .await
        .unwrap();
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let run = finish(&t, &run.id, RunStatus::Succeeded, Some("Gave it."), None).await;

    settle(&t).await;
    settle(&t).await;

    assert!(wakes(&t).await.is_empty());
    assert!(!t.rt.store().making_job_held_told(&job.id).await.unwrap());
    t.rt.makers().deliver_wakes().await.unwrap();
    let chat =
        t.rt.chat(&maker.id, "How is it going?", Some(&run.id))
            .await
            .unwrap();
    let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
    let ids: Vec<&str> = thread.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [run.id.as_str(), chat.id.as_str()]);
}

/// A job held for Claude's usage limit in a topic nobody watches is not told either: its wake
/// would be background, and held the same way.
#[tokio::test]
async fn a_job_held_for_the_usage_limit_in_an_unattended_topic_is_not_reported() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let scheduled = live_run_as(&t, &maker, TriggerKind::Schedule).await;
    make(&t, &scheduled, "Blog Watcher").await;
    t.rt.limits().set_shares(HoldShares {
        enabled: true,
        five_hour: 60,
        weekly: 70,
    });
    let now = Utc::now();
    let reading = LimitReading {
        window: "five_hour".into(),
        utilization: 0.95,
        resets_at: timestamp(now + TimeDelta::hours(3)),
        status: "allowed".into(),
        read_at: timestamp(now),
    };
    t.rt.limits()
        .record("allowed", vec![reading])
        .await
        .unwrap();
    let job = give(&t, &scheduled, "Blog Watcher", "Read the blogs now.").await;
    let held = t.rt.limits_get().await.unwrap().held_run_ids;
    assert!(held.contains(&run_of(&t, &job).await.id), "{held:?}");

    settle(&t).await;

    assert!(wakes(&t).await.is_empty());
    assert!(!t.rt.store().making_job_held_told(&job.id).await.unwrap());
}

/// A topic the person started with Run now, or one an app started, is watched though nobody
/// typed in it (dots-make-dots design section 8): the jobs given there and the wakes that go
/// there are not background.
#[tokio::test]
async fn a_job_and_a_wake_in_a_run_now_or_app_topic_are_not_background() {
    let t = quiet().await;
    let maker = maker(&t).await;
    for (trigger, name) in [
        (TriggerKind::Manual, "Blog Watcher"),
        (TriggerKind::App, "Feed Reader"),
    ] {
        let root = live_run_as(&t, &maker, trigger).await;
        make(&t, &root, name).await;
        let job = done_job(&t, &root, name, "Three new posts.").await;
        let its = run_of(&t, &job).await;
        assert!(!run_is_background(&its), "{trigger:?}: {its:#?}");
        let root = finish(&t, &root.id, RunStatus::Succeeded, Some("Gave it."), None).await;
        t.rt.makers().deliver_wakes().await.unwrap();
        let thread = t.rt.store().list_thread(&root.root_run_id).await.unwrap();
        assert_eq!(thread.len(), 2, "{trigger:?}: {thread:#?}");
        assert!(
            !run_is_background(&thread[1]),
            "{trigger:?}: {:#?}",
            thread[1]
        );
    }
}

/// A delete that comes after a job's run ended, before any pass settled the job, reports the
/// job as it ended, with its final text, and not as deleted (dots-make-dots design section
/// 8); on the person's delete as on the maker's.
#[tokio::test]
async fn a_delete_after_a_job_run_ended_reports_the_job_as_it_ended() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    for (name, by_maker) in [("Blog Watcher", true), ("Feed Reader", false)] {
        let made = make(&t, &run, name).await;
        let job = give(&t, &run, name, "Read the blogs now.").await;
        let its = run_of(&t, &job).await;
        start(&t, &its.id).await;
        finish(
            &t,
            &its.id,
            RunStatus::Succeeded,
            Some("Three new posts."),
            None,
        )
        .await;
        if by_maker {
            t.rt.makers().delete_dot(&run.id, name).await.unwrap();
        } else {
            t.rt.delete_dot(&made.id).await.unwrap();
        }
    }
    assert_eq!(
        wakes(&t).await,
        [
            "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts.",
            "\"Feed Reader\" finished the job.\n\nIts final message:\n> Three new posts."
        ]
    );
}

/// A delete that is refused (a run of the dot was queued) changes nothing: the job still
/// waits for the person's answer, and the maker is not woken.
#[tokio::test]
async fn a_refused_delete_leaves_the_job_and_wakes_nobody() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    let its = start(&t, &its.id).await;
    question(&t, &its).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Which first?"),
        None,
    )
    .await;
    settle(&t).await;
    assert_eq!(stored(&t, &job).await.status, JobStatus::WaitingAnswer);
    // A run of the dot is queued, as a schedule would queue it.
    t.rt.store()
        .create_run(&NewRun::new(&made.id, TriggerKind::Schedule))
        .await
        .unwrap();

    let refused = t.rt.delete_dot(&made.id).await;

    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    assert_eq!(stored(&t, &job).await.status, JobStatus::WaitingAnswer);
    assert!(wakes(&t).await.is_empty());
    assert!(t.rt.store().get_dot(&made.id).await.is_ok());
}

/// A delete that is refused after it settled a job whose run had ended still tells the maker
/// how the job ended: the wake its settle stored is delivered then, and does not wait for the
/// end of some later run.
#[tokio::test]
async fn a_refused_delete_delivers_the_wake_its_settle_stored() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Three new posts."),
        None,
    )
    .await;
    let run = finish(&t, &run.id, RunStatus::Succeeded, Some("Gave it."), None).await;
    // A run of the dot is queued, as a schedule would queue it.
    t.rt.store()
        .create_run(&NewRun::new(&made.id, TriggerKind::Schedule))
        .await
        .unwrap();

    let refused = t.rt.delete_dot(&made.id).await;

    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    assert_eq!(stored(&t, &job).await.status, JobStatus::Ended);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
        if thread.len() == 2 {
            assert_eq!(
                message(&thread[1]),
                "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts."
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the wake was not delivered: {:?}",
            wakes(&t).await
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `delete_and_tell` settles by itself a job whose run ended before it deletes the dot: called
/// with no pass before it, it reports the job as it ended, with its final text, and not as
/// deleted.
#[tokio::test]
async fn a_delete_settles_a_job_whose_run_ended_by_itself() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let job = give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    finish(
        &t,
        &its.id,
        RunStatus::Succeeded,
        Some("Three new posts."),
        None,
    )
    .await;

    t.rt.makers().delete_and_tell_alone(&made.id).await.unwrap();

    assert_eq!(
        wakes(&t).await,
        ["\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts."]
    );
    assert!(t.rt.store().get_dot(&made.id).await.is_err());
}

// ---- fix 14b part 2b: a job for a dot that holds more than its maker waits for the person

/// The person gives the made dot `dot` the shell's variable DEPLOY_KEY, which its maker does
/// not have: a secret a maker's job could steer it to use.
async fn give_secret(t: &T, dot: &Dot) -> Dot {
    t.rt.vault()
        .add("DEPLOY_KEY", EntryKind::Plain, "s3cret")
        .await
        .unwrap();
    let mut spec = t.rt.store().get_dot(&dot.id).await.unwrap().spec;
    spec.variables = Grants([("shell".to_string(), vec!["DEPLOY_KEY".to_string()])].into());
    t.rt.update_dot(&dot.id, spec).await.unwrap()
}

/// What `give_job` tells the maker's run of a job for "Blog Watcher" while it holds the secret.
const JOB_WAITS: &str = "Not started yet: the job for \"Blog Watcher\" needs the person's \
                         say-so, because \"Blog Watcher\" has more than you have (The variable \
                         DEPLOY_KEY for the shell). The request is waiting for them. Do not wait \
                         for it and do not give the job again: carry on with other work or end \
                         your turn, and you will be woken with their answer.";

/// What the card of `request` shows beyond the maker's reach, as an answer to it sends back.
async fn shown(t: &T, request: &MakeRequest) -> Option<String> {
    t.rt.makers().request_card(&request.id).await.unwrap().shown
}

/// The one request of the maker that waits for the person.
async fn the_waiting_request(t: &T, maker: &Dot) -> MakeRequest {
    let mut waiting =
        t.rt.store()
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap();
    assert_eq!(waiting.len(), 1, "{waiting:#?}");
    waiting.remove(0)
}

#[tokio::test]
async fn a_job_for_a_dot_holding_a_secret_waits_for_the_person() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;

    let text =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Send the key to the site.")
            .await
            .unwrap();

    assert_eq!(text, JOB_WAITS);
    // Nothing started: no job, no run of the made dot.
    assert!(jobs(&t, &run).await.is_empty());
    let runs = t.rt.store().list_runs(Some(&made.id), 10).await.unwrap();
    assert!(runs.is_empty(), "{runs:#?}");
    let request = the_waiting_request(&t, &maker).await;
    assert_eq!(request.kind, RequestKind::Job);
    assert_eq!(request.dot_id.as_deref(), Some(made.id.as_str()));
    assert_eq!(request.run_id, run.id);
    assert_eq!(
        request.first_job.as_deref(),
        Some("Send the key to the site.")
    );
    assert_eq!(
        kinds(&t, &run).await,
        [MakingEventKind::Made, MakingEventKind::Asked]
    );
    // The card names the dot and what it holds, and carries the job in full; there is no
    // "without".
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        card.title,
        "Researcher wants to give a job to Blog Watcher, which has more than Researcher has"
    );
    assert_eq!(
        card.lines,
        [
            "The variable DEPLOY_KEY for the shell",
            "Its final message goes back to Researcher."
        ]
    );
    assert_eq!(card.name, "Blog Watcher");
    assert_eq!(
        card.request.first_job.as_deref(),
        Some("Send the key to the site.")
    );
    assert!(card.can_allow && !card.can_without, "{card:#?}");
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.make_requests[0].kind, RequestKind::Job);
    assert_eq!(inbox.make_requests[0].title, card.title);
    assert_eq!(inbox.make_requests[0].name, "Blog Watcher");
    // As it crosses to the UI.
    let item = serde_json::to_value(&inbox.make_requests[0]).unwrap();
    assert_eq!(item["kind"], "job");
    assert_eq!(
        invalid(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Without, None, None)
                .await
        ),
        "A job cannot be given without a part of it. Allow it, or don't give it."
    );

    // Allow starts the job as it was given, from the run and topic that gave it.
    let after =
        t.rt.makers()
            .decide_request(
                &request.id,
                MakeDecision::Allow,
                None,
                shown(&t, &request).await.as_deref(),
            )
            .await
            .unwrap();
    assert_eq!(after.status, RequestStatus::Allowed);
    assert_eq!(after.made_dot_id.as_deref(), Some(made.id.as_str()));
    let all = jobs(&t, &run).await;
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].message, "Send the key to the site.");
    assert_eq!(all[0].maker_run_id, run.id);
    assert_eq!(run_of(&t, &all[0]).await.dot_id, made.id);
    assert_eq!(
        wakes(&t).await,
        [
            "The person allowed the job for \"Blog Watcher\": it has started. Do not wait for \
             it; you will be woken with its result."
        ]
    );
}

#[tokio::test]
async fn a_refused_job_starts_nothing_and_wakes_the_maker_once() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;
    let makers = t.rt.makers();
    makers
        .give_job(&run.id, "Blog Watcher", "Send the key.")
        .await
        .unwrap();
    // One job for the dot waits at a time.
    assert_eq!(
        invalid(makers.give_job(&run.id, "Blog Watcher", "Again.").await),
        "A job for \"Blog Watcher\" is already waiting for the person. You will be woken with \
         their answer."
    );
    // It counts toward the requests that may wait.
    let mut browser = ask("Feed Reader");
    browser.wanted.browser = Some(true);
    makers.make_dot(&run.id, browser.clone()).await.unwrap();
    browser.wanted.name = Some("Mail Reader".into());
    makers.make_dot(&run.id, browser.clone()).await.unwrap();
    browser.wanted.name = Some("News Reader".into());
    assert_eq!(
        invalid(makers.make_dot(&run.id, browser).await),
        "Researcher already has 3 requests waiting for the person. Wait to be woken with an \
         answer before asking for more."
    );
    let waiting = t.rt.store().list_make_requests(Some(&maker.id), true);
    let request = waiting.await.unwrap().remove(0);
    assert_eq!(request.kind, RequestKind::Job);

    let after = makers
        .decide_request(&request.id, MakeDecision::Refuse, None, None)
        .await
        .unwrap();

    assert_eq!(after.status, RequestStatus::Refused);
    assert!(jobs(&t, &run).await.is_empty());
    assert_eq!(
        wakes(&t).await,
        [
            "The person chose not to give \"Blog Watcher\" the job. Do not give it again; carry \
             on without it, or tell the person what you cannot do."
        ]
    );
}

#[tokio::test]
async fn a_job_the_person_allows_passes_the_loop_guard_and_counts_toward_it() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    let keeper = make(&t, &run, "Key Keeper").await;
    give_secret(&t, &keeper).await;
    let text = t.rt.makers().give_job(&run.id, "Key Keeper", "Rotate it.");
    assert!(text.await.unwrap().starts_with("Not started yet"));
    // While it waits it does not count: six more jobs are taken.
    six_jobs(&t, &run).await;
    let request = the_waiting_request(&t, &maker).await;

    // The person allows it past the guard: the person gave it.
    t.rt.makers()
        .decide_request(
            &request.id,
            MakeDecision::Allow,
            None,
            shown(&t, &request).await.as_deref(),
        )
        .await
        .unwrap();

    let all = jobs(&t, &run).await;
    assert_eq!(all.len(), 7);
    assert_eq!(all[6].dot_id, keeper.id);
    // Once started it counts: the maker gives no job more.
    let next =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Once more.");
    assert_eq!(invalid(next.await), GUARD);
}

#[tokio::test]
async fn a_made_dot_the_person_lets_make_dots_takes_jobs_only_with_their_say_so() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let mut spec = made.spec.clone();
    spec.making.on = true;
    t.rt.update_dot(&made.id, spec).await.unwrap();

    let text =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Make two dots.");

    assert!(text.await.unwrap().starts_with("Not started yet"));
    let request = the_waiting_request(&t, &maker).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        card.lines,
        [
            "Can make dots itself (3 in one run, 10 kept)",
            "Its final message goes back to Researcher."
        ]
    );
    assert!(jobs(&t, &run).await.is_empty());
}

#[tokio::test]
async fn deleting_a_dot_whose_job_waits_wakes_its_maker_once() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;
    t.rt.makers()
        .give_job(&run.id, "Blog Watcher", "Send the key.")
        .await
        .unwrap();

    t.rt.makers()
        .delete_dot(&run.id, "Blog Watcher")
        .await
        .unwrap();

    assert_eq!(wakes(&t).await, [job_went_message("Blog Watcher")]);
    assert_eq!(
        job_went_message("Blog Watcher"),
        "\"Blog Watcher\" was deleted, so the job you gave it went with it: the person will not \
         answer it."
    );
    assert!(t
        .rt
        .store()
        .list_make_requests(Some(&maker.id), true)
        .await
        .unwrap()
        .is_empty());
}

// ---- fix 14b part 3b: after the re-review of part 2b

/// The shell's variable DEPLOY_KEY as a dot's only grant.
fn deploy_key() -> Grants {
    Grants([("shell".to_string(), vec!["DEPLOY_KEY".to_string()])].into())
}

/// A job is judged on its maker as it is once the lock the makes hold is taken (dots-make-dots
/// design section 8): the person takes a secret from the maker while its `give_job` waits for
/// that lock, so the made dot, which keeps the secret, now has more than its maker, and the job
/// asks the person.
#[tokio::test]
async fn a_job_given_while_the_person_narrows_the_maker_is_judged_on_the_maker_now() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;
    // The maker has the secret too: the dot holds nothing beyond its reach.
    let mut s = t.rt.store().get_dot(&maker.id).await.unwrap().spec;
    s.variables = deploy_key();
    t.rt.update_dot(&maker.id, s.clone()).await.unwrap();
    let makers = t.rt.makers();
    let held = makers.hold_makes().await;

    let (given, ()) = tokio::join!(
        makers.give_job(&run.id, "Blog Watcher", "Send the key to the site."),
        async {
            // The call has read its maker and waits for the lock.
            tokio::time::sleep(Duration::from_millis(300)).await;
            s.variables = Grants::default();
            t.rt.update_dot(&maker.id, s).await.unwrap();
            drop(held);
        }
    );

    assert_eq!(given.unwrap(), JOB_WAITS);
    assert!(jobs(&t, &run).await.is_empty());
    assert_eq!(the_waiting_request(&t, &maker).await.kind, RequestKind::Job);
}

/// A job for a dot whose last job from the conversation is still open is refused before any
/// request is stored, also when the dot holds more than its maker: no card is shown for a job
/// that could not start.
#[tokio::test]
async fn a_job_while_the_last_one_is_open_stores_no_request() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give(&t, &run, "Blog Watcher", "Read the blogs now.").await;
    give_secret(&t, &made).await;

    let refused =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Send the key.");

    assert_eq!(
        invalid(refused.await),
        "\"Blog Watcher\" is still working on the last job from this conversation. You will be \
         woken when it ends."
    );
    let waiting =
        t.rt.store()
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap();
    assert!(waiting.is_empty(), "{waiting:#?}");
}

/// The list of made dots says that a job for a dot waits for the person, as it says a change
/// does.
#[tokio::test]
async fn the_list_says_a_job_for_a_made_dot_waits_for_the_person() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;
    t.rt.makers()
        .give_job(&run.id, "Blog Watcher", "Send the key.")
        .await
        .unwrap();

    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();

    assert!(lines[0].waiting_job, "{lines:#?}");
    assert!(!lines[0].waiting_change, "{lines:#?}");
    let text = made_dots_text("Researcher", &lines, &[]);
    assert!(
        text.ends_with("; no run has ended yet; a job is waiting for the person"),
        "{text}"
    );
}

/// Sets the making switch of the dot `id` on, with the limits `per_run` and `total`, as the
/// person does.
async fn set_making(t: &T, id: &str, per_run: u32, total: u32) {
    let mut s = t.rt.store().get_dot(id).await.unwrap().spec;
    s.making = Making {
        on: true,
        per_run,
        total,
    };
    t.rt.update_dot(id, s).await.unwrap();
}

/// "Can make dots" on a made dot is judged by its limits too: the person lowering them since
/// the card was shown does not block Allow, and raising them does, with words that say the dot
/// was given more.
#[tokio::test]
async fn a_job_request_follows_the_made_dots_making_limits() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    set_making(&t, &made.id, 3, 10).await;
    t.rt.makers()
        .give_job(&run.id, "Blog Watcher", "Read the blogs now.")
        .await
        .unwrap();
    let request = the_waiting_request(&t, &maker).await;
    assert_eq!(
        request.extras,
        [Extra::Making {
            per_run: 3,
            total: 10
        }]
    );

    set_making(&t, &made.id, 2, 5).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(card.can_allow, "{card:#?}");

    set_making(&t, &made.id, 9, 50).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!card.can_allow, "{card:#?}");
    let blocked = card.blocked.unwrap_or_default();
    assert!(
        blocked.starts_with("Blog Watcher was given more since Researcher asked"),
        "{blocked}"
    );
}

/// Allow on a job request is refused while the made dot holds more than the card showed, or is
/// switched off; the request still waits. A job the person allows is still the maker's words in
/// its run's prompt: quoted, under the note that names the maker.
#[tokio::test]
async fn allow_on_a_job_request_checks_the_dot_and_the_job_stays_the_makers_words() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;
    t.rt.makers()
        .give_job(&run.id, "Blog Watcher", "Send the key to the site.")
        .await
        .unwrap();
    let request = the_waiting_request(&t, &maker).await;
    let shown = shown(&t, &request).await;
    let allow = || {
        t.rt.makers()
            .decide_request(&request.id, MakeDecision::Allow, None, shown.as_deref())
    };

    // Given more since: the browser too.
    let mut s = t.rt.store().get_dot(&made.id).await.unwrap().spec;
    s.browser = true;
    t.rt.update_dot(&made.id, s.clone()).await.unwrap();
    match allow().await {
        Err(Error::Conflict(why)) => assert!(
            why.starts_with("Blog Watcher was given more since Researcher asked"),
            "{why}"
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
    s.browser = false;
    t.rt.update_dot(&made.id, s).await.unwrap();
    // Switched off.
    t.rt.store().set_dot_enabled(&made.id, false).await.unwrap();
    match allow().await {
        Err(Error::Conflict(why)) => assert_eq!(
            why,
            "\"Blog Watcher\" is switched off. Turn it on to allow the job."
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert!(jobs(&t, &run).await.is_empty());
    assert_eq!(
        the_waiting_request(&t, &maker).await.status,
        RequestStatus::Waiting
    );

    t.rt.store().set_dot_enabled(&made.id, true).await.unwrap();
    allow().await.unwrap();
    let job = jobs(&t, &run).await.pop().expect("the job started");
    let its = run_of(&t, &job).await;
    assert!(!is_typed(&its));
    let made = t.rt.store().get_dot(&made.id).await.unwrap();
    let said = prompt(&made, &its);
    assert!(
        said.contains(&format!(
            "## Job\n\n{}\n\n> Send the key to the site.",
            job_note("Researcher")
        )),
        "{said}"
    );
    assert!(!said.contains("## Message from the user"), "{said}");
}

/// The job card says where the made dot's answer goes (dots-make-dots design section 8), and
/// the values of a variable that dot holds beyond its maker's reach are hidden from the maker's
/// wake: a dot that echoes its token does not put it in its maker's prompt.
#[tokio::test]
async fn a_job_card_says_the_answer_goes_back_and_the_wake_hides_the_dots_own_values() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    t.rt.vault()
        .add("SITE_TOKEN", EntryKind::Plain, "tok-4f9a2c71e0")
        .await
        .unwrap();
    let mut spec = t.rt.store().get_dot(&made.id).await.unwrap().spec;
    spec.variables = Grants([("shell".to_string(), vec!["SITE_TOKEN".to_string()])].into());
    t.rt.update_dot(&made.id, spec).await.unwrap();
    let text =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Post the list.")
            .await
            .unwrap();
    assert!(text.starts_with("Not started yet"), "{text}");
    let request = the_waiting_request(&t, &maker).await;

    let card = t.rt.makers().request_card(&request.id).await.unwrap();

    assert_eq!(
        card.lines,
        [
            "The variable SITE_TOKEN for the shell",
            "Its final message goes back to Researcher."
        ]
    );
    t.rt.makers()
        .decide_request(
            &request.id,
            MakeDecision::Allow,
            None,
            card.shown.as_deref(),
        )
        .await
        .unwrap();
    let job = jobs(&t, &run).await.pop().expect("the job started");
    let its = run_of(&t, &job).await;
    start(&t, &its.id).await;
    let posted = "Posted with tok-4f9a2c71e0, as asked.";
    finish(&t, &its.id, RunStatus::Succeeded, Some(posted), None).await;
    settle(&t).await;
    let said = wakes(&t).await;
    assert_eq!(
        said.last().unwrap(),
        "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Posted with [hidden], as \
         asked."
    );
}

/// A job held by its dot's own cap is not told to a maker whose wake would be held itself
/// (dots-make-dots design section 8): here the maker works in a topic nobody watches, on Claude,
/// while Claude's usage hold is on, and the made dot runs on the local engine, so its own run
/// waits for its cap and not for the usage limit. The wake would be background and usage-held,
/// and would keep the topic busy until the hold ends.
#[tokio::test]
async fn a_job_held_by_its_dots_cap_is_not_told_while_the_makers_wake_would_be_held() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let scheduled = live_run_as(&t, &maker, TriggerKind::Schedule).await;
    let made = make(&t, &scheduled, "Blog Watcher").await;
    // The person moves the made dot to a local model and gives it a daily cap, which a run of
    // it spends.
    let mut s = t.rt.store().get_dot(&made.id).await.unwrap().spec;
    s.engine = dots_core::model::EngineKind::Local;
    s.endpoint_url = Some("http://127.0.0.1:9/v1".into());
    s.model = "test-model".into();
    s.spend_cap.daily_usd = Some(0.5);
    t.rt.update_dot(&made.id, s).await.unwrap();
    let spent =
        t.rt.store()
            .create_run(&NewRun::new(&made.id, TriggerKind::Schedule))
            .await
            .unwrap();
    start(&t, &spent.id).await;
    t.rt.store()
        .add_usage(&spent.id, 10, 10, 0, Some(0.6))
        .await
        .unwrap();
    finish(&t, &spent.id, RunStatus::Succeeded, Some("Done."), None).await;
    // Claude's usage hold is on.
    t.rt.limits().set_shares(HoldShares {
        enabled: true,
        five_hour: 60,
        weekly: 70,
    });
    let now = Utc::now();
    let reading = LimitReading {
        window: "five_hour".into(),
        utilization: 0.95,
        resets_at: timestamp(now + TimeDelta::hours(3)),
        status: "allowed".into(),
        read_at: timestamp(now),
    };
    t.rt.limits()
        .record("allowed", vec![reading])
        .await
        .unwrap();
    let job = give(&t, &scheduled, "Blog Watcher", "Read the blogs now.").await;

    settle(&t).await;

    assert!(wakes(&t).await.is_empty());
    assert!(!t.rt.store().making_job_held_told(&job.id).await.unwrap());
}

/// A job request whose dot took a later job from the same topic in the meantime cannot be
/// allowed while that job is open (dots-make-dots design section 8): its card says so in words
/// for the person, and Allow is refused with the same words.
#[tokio::test]
async fn a_job_request_cannot_be_allowed_while_a_later_job_to_its_dot_is_open() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    give_secret(&t, &made).await;
    let text =
        t.rt.makers()
            .give_job(&run.id, "Blog Watcher", "Send the key.");
    assert_eq!(text.await.unwrap(), JOB_WAITS);
    let request = the_waiting_request(&t, &maker).await;
    // The person gives the maker the variable, and the maker gives the dot another job.
    let mut s = t.rt.store().get_dot(&maker.id).await.unwrap().spec;
    s.variables = Grants([("shell".to_string(), vec!["DEPLOY_KEY".to_string()])].into());
    t.rt.update_dot(&maker.id, s).await.unwrap();
    give(&t, &run, "Blog Watcher", "Rotate it.").await;

    let card = t.rt.makers().request_card(&request.id).await.unwrap();

    let why = "\"Blog Watcher\" is still working on another job from this chat. Allow this one \
               when that job ends.";
    assert!(!card.can_allow, "{card:#?}");
    assert_eq!(card.blocked.as_deref(), Some(why));
    let allow = t.rt.makers().decide_request(
        &request.id,
        MakeDecision::Allow,
        None,
        card.shown.as_deref(),
    );
    match allow.await {
        Err(Error::Conflict(text)) => assert_eq!(text, why),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert_eq!(jobs(&t, &run).await.len(), 1);
}

/// A first job that could not be stored tells the model a plain sentence, not the store's
/// error (dots-make-dots design section 12): the detail goes to the log.
#[tokio::test]
async fn a_first_job_the_store_could_not_start_tells_the_model_a_plain_sentence() {
    let t = quiet().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut conn = SqliteConnectOptions::new()
        .filename(t.dir.path().join("data").join("dots.db"))
        .foreign_keys(false)
        .connect()
        .await
        .unwrap();
    sqlx::query("ALTER TABLE making_jobs RENAME TO making_jobs_away")
        .execute(&mut conn)
        .await
        .unwrap();
    let mut asked = ask("Blog Watcher");
    asked.first_job = Some("Read them now.".into());

    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();

    assert_eq!(
        outcome.tool_text(),
        "Made the dot \"Blog Watcher\". It is on and ready; give it a job with give_job. Its \
         first job was not started: Dots could not do that just now. Give it the job with \
         give_job."
    );
}

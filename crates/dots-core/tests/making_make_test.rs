//! A maker's run makes a dot (dots-make-dots design sections 5, 7.1 and 10): a dot within the
//! maker's reach and within its limits is created at once, with who made it recorded and a
//! "made" entry in the makers' log; one beyond reach or over a limit creates nothing and waits
//! for the person (`making_request_test.rs`); and what could never be allowed is a plain error.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::library::{LibraryConfig, McpServerConfig, Uses};
use dots_core::making::log::{MakingEvent, MakingEventKind};
use dots_core::making::reach::{real_path, Extra, Wanted};
use dots_core::making::service::{MakeDot, MakeOutcome, OverLimit};
use dots_core::model::{Dot, DotSpec, Making, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::vault::{EntryKind, Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::ConnectOptions;
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

struct T {
    dir: TempDir,
    /// Where a dot made without a folder gets one (`Config::new_dot_root`).
    root: PathBuf,
    rt: Runtime,
}

impl T {
    /// The maker's folder.
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }
}

async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.new_dot_root = dir.path().join("Dots");
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    // No queued run starts: the tests put a run in the running state by hand.
    cfg.max_concurrent_runs = 0;
    let root = cfg.new_dot_root.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    T { dir, root, rt }
}

/// The spec of a dot named Researcher that works in the test's `work` folder itself and may
/// make dots.
fn maker_spec(t: &T) -> DotSpec {
    let mut s = spec("Researcher");
    s.workdir = t.work().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.making.on = true;
    s
}

/// A maker named Researcher, created as the person creates a dot.
async fn maker(t: &T) -> Dot {
    t.rt.create_dot(maker_spec(t), None, None).await.unwrap()
}

/// A maker whose settings allow `per_run` dots in one run and `total` kept.
async fn maker_with(t: &T, per_run: u32, total: u32) -> Dot {
    let mut s = maker_spec(t);
    s.making = Making {
        on: true,
        per_run,
        total,
    };
    t.rt.create_dot(s, None, None).await.unwrap()
}

/// A run of `dot` that is running: a chat the person started with a typed message.
async fn live_run(t: &T, dot: &Dot) -> Run {
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "watch three blogs", "typed": true}));
    let run = t.rt.store().create_run(&new).await.unwrap();
    assert!(t.rt.store().claim_run(&run.id).await.unwrap());
    t.rt.store().get_run(&run.id).await.unwrap()
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

/// The text of the refusal of `ask`.
async fn refused(t: &T, run: &Run, ask: MakeDot) -> String {
    match t.rt.makers().make_dot(&run.id, ask).await {
        Err(Error::Invalid(text) | Error::Conflict(text)) => text,
        other => panic!("expected a plain refusal, got {other:?}"),
    }
}

/// Every `minutes`, as a tool call carries it. The first slot is hours away, so nothing fires
/// during a test.
fn every(minutes: u32) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": minutes},
        "catch_up": true,
        "until": null
    }))
    .unwrap()
}

/// The topics named by the `MakingUpdated` events published since the last call, each with
/// the maker it names.
fn updated(rx: &mut Receiver<RuntimeEvent>) -> Vec<(String, String)> {
    let mut seen = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::MakingUpdated {
            dot_id,
            root_run_id,
        } = ev
        {
            seen.push((dot_id, root_run_id));
        }
    }
    seen
}

/// The makers' log of the topic `run` belongs to.
async fn log(t: &T, run: &Run) -> Vec<MakingEvent> {
    t.rt.store()
        .list_making_events(&run.root_run_id)
        .await
        .unwrap()
}

/// The names of the folders in `root`.
fn folders(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The names of all dots, sorted.
async fn dot_names(t: &T) -> Vec<String> {
    let mut names: Vec<String> =
        t.rt.store()
            .list_dots()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.spec.name)
            .collect();
    names.sort();
    names
}

/// Puts an MCP server named `github` in the Library whose config reads `GITHUB_TOKEN`, and the
/// variable in the Personal vault.
async fn add_github(t: &T) {
    let config = LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: [("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    });
    t.rt.store()
        .add_library_item("github", &config, None)
        .await
        .unwrap();
    t.rt.vault()
        .add("GITHUB_TOKEN", EntryKind::Plain, "t0ken")
        .await
        .unwrap();
}

fn uses_github() -> Uses {
    Uses {
        mcp_servers: vec!["github".into()],
        ..Uses::default()
    }
}

fn grants(consumer: &str, name: &str) -> Grants {
    Grants([(consumer.to_string(), vec![name.to_string()])].into())
}

#[tokio::test]
async fn a_dot_within_reach_is_made_at_once() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut rx = t.rt.subscribe();
    let outcome =
        t.rt.makers()
            .make_dot(&run.id, ask("Blog Watcher"))
            .await
            .unwrap();
    let MakeOutcome::Made { dot, .. } = outcome else {
        panic!("not made: {outcome:?}");
    };
    // It exists, on, and may not itself make dots.
    assert_eq!(t.rt.store().get_dot(&dot.id).await.unwrap(), dot);
    assert_eq!(dot.spec.name, "Blog Watcher");
    assert!(dot.spec.enabled);
    assert_eq!(dot.spec.making, Making::default());
    assert!(!dot.spec.making.on);
    // Who made it, in which run, and what for.
    let made = dot.made.clone().expect("a made dot");
    assert_eq!(made.by_dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(made.by_name, "Researcher");
    assert_eq!(made.run_id, run.id);
    assert_eq!(made.purpose, "Reads the feeds");
    // The maker's account and workspace, and a new folder of its own.
    assert_eq!(dot.org_id, maker.org_id);
    assert_eq!(dot.workspace_id, maker.workspace_id);
    assert_eq!(folders(&t.root), ["blog-watcher"]);
    assert!(Path::new(&dot.spec.workdir).is_dir());
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
    // The topic's log has the card's entry.
    let events = log(&t, &run).await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.kind, MakingEventKind::Made);
    assert_eq!(event.maker_dot_id, maker.id);
    assert_eq!(event.run_id, run.id);
    assert_eq!(event.root_run_id, run.root_run_id);
    assert_eq!(event.dot_id.as_deref(), Some(dot.id.as_str()));
    assert_eq!(event.dot_name, "Blog Watcher");
    // It keeps the folder made for the dot, so that folder is known as its own.
    assert_eq!(
        event.detail,
        json!({"purpose": "Reads the feeds", "folder": dot.spec.workdir})
    );
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    assert_eq!(t.rt.store().count_made_in_run(&run.id).await.unwrap(), 1);
}

#[tokio::test]
async fn a_run_that_continues_a_topic_logs_under_the_topic() {
    let t = start().await;
    let maker = maker(&t).await;
    let first = live_run(&t, &maker).await;
    make(&t, &first, "One").await;
    t.rt.store()
        .finish_run(&first.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    let mut new = NewRun::new(&maker.id, TriggerKind::Chat);
    new.parent_run_id = Some(first.id.clone());
    new.root_run_id = Some(first.id.clone());
    let next = t.rt.store().create_run(&new).await.unwrap();
    assert!(t.rt.store().claim_run(&next.id).await.unwrap());
    make(&t, &next, "Two").await;
    // Oldest first, both under the topic's first run.
    let events = log(&t, &first).await;
    let names: Vec<&str> = events.iter().map(|e| e.dot_name.as_str()).collect();
    assert_eq!(names, ["One", "Two"]);
    assert_eq!(events[1].run_id, next.id);
    assert_eq!(events[1].root_run_id, first.id);
}

#[tokio::test]
async fn a_schedule_given_is_kept_and_its_state_row_made() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut asked = ask("Blog Watcher");
    asked.wanted.schedule = Some(Some(every(60)));
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Made { dot, .. } = outcome else {
        panic!("not made: {outcome:?}");
    };
    assert!(dot.spec.schedule.is_some());
    assert!(t
        .rt
        .store()
        .get_schedule_state(&dot.id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn a_subfolder_that_does_not_exist_is_created() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut asked = ask("Blog Watcher");
    asked.wanted.folder = Some("notes/blogs".into());
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Made { dot, .. } = outcome else {
        panic!("not made: {outcome:?}");
    };
    let folder = real_path(&t.work()).join("notes").join("blogs");
    assert!(folder.is_dir());
    assert_eq!(PathBuf::from(&dot.spec.workdir), folder);
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
    // No default folder was made beside it.
    assert!(folders(&t.root).is_empty());
}

#[tokio::test]
async fn a_subfolder_made_for_a_dot_that_is_not_created_is_removed() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    // The path the person's answer takes creates from a checked spec. Here the name was taken
    // in between, so the store refuses it after the folder was made.
    make(&t, &run, "Taken").await;
    let reach = dots_core::making::reach::reach_new(
        &maker,
        &Wanted {
            folder: Some("notes/blogs".into()),
            ..ask("taken").wanted
        },
        Path::new(""),
    )
    .unwrap();
    let err =
        t.rt.makers()
            .create_made(
                &maker,
                &run.id,
                &run.root_run_id,
                reach.spec,
                "Reads the feeds",
                reach.create_folder,
            )
            .await
            .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert!(!t.work().join("notes").exists());
    assert_eq!(log(&t, &run).await.len(), 1);
}

#[tokio::test]
async fn a_maker_with_the_switch_off_is_refused() {
    let t = start().await;
    let mut s = maker_spec(&t);
    s.making.on = false;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &dot).await;
    assert_eq!(
        refused(&t, &run, ask("Blog Watcher")).await,
        "Researcher may not make dots. The person turns that on in the dot's settings."
    );
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(log(&t, &run).await.is_empty());
}

#[tokio::test]
async fn a_workflow_step_run_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    // Named a step without building a workflow: the step row itself does not matter here.
    let mut conn = SqliteConnectOptions::new()
        .filename(t.dir.path().join("data").join("dots.db"))
        .foreign_keys(false)
        .connect()
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET workflow_step_run_id = 'STEP' WHERE id = ?")
        .bind(&run.id)
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    assert_eq!(
        refused(&t, &run, ask("Blog Watcher")).await,
        "A run that is a workflow step cannot use the tools for making dots."
    );
    assert_eq!(dot_names(&t).await, ["Researcher"]);
}

#[tokio::test]
async fn a_finished_run_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    t.rt.store()
        .finish_run(&run.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    assert_eq!(
        refused(&t, &run, ask("Blog Watcher")).await,
        "This run has ended, so it cannot use the tools for making dots."
    );
    // A run that does not exist is no run of a maker either.
    let err =
        t.rt.makers()
            .make_dot("no-such-run", ask("Blog Watcher"))
            .await
            .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert_eq!(dot_names(&t).await, ["Researcher"]);
}

#[tokio::test]
async fn a_dot_that_is_not_valid_is_a_plain_error() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut nameless = ask("x");
    nameless.wanted.name = None;
    assert_eq!(refused(&t, &run, nameless).await, "A new dot needs a name.");
    for purpose in [
        "".to_string(),
        "  ".to_string(),
        "x".repeat(201),
        "one\ntwo".to_string(),
    ] {
        let mut asked = ask("Blog Watcher");
        asked.purpose = purpose;
        assert_eq!(
            refused(&t, &run, asked).await,
            "The purpose is one line of 1 to 200 characters."
        );
    }
    for job in ["".to_string(), " ".to_string(), "x".repeat(4001)] {
        let mut asked = ask("Blog Watcher");
        asked.first_job = Some(job);
        assert_eq!(
            refused(&t, &run, asked).await,
            "The first job is 1 to 4000 characters."
        );
    }
    // The longest of each is taken, and the purpose is kept without the spaces around it.
    let mut asked = ask("Blog Watcher");
    asked.purpose = format!(" {} ", "p".repeat(200));
    asked.first_job = Some("j".repeat(4000));
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Made { dot, .. } = outcome else {
        panic!("not made: {outcome:?}");
    };
    assert_eq!(dot.made.unwrap().purpose, "p".repeat(200));
}

#[tokio::test]
async fn a_name_in_use_is_a_plain_error() {
    // One dot in a run, so the name is seen to be checked before the limits.
    let t = start().await;
    let maker = maker_with(&t, 1, 10).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "Blog Watcher").await;
    for name in ["Blog Watcher", "blog watcher", "Researcher"] {
        let err =
            t.rt.makers()
                .make_dot(&run.id, ask(name))
                .await
                .unwrap_err();
        match err {
            Error::Conflict(text) => assert_eq!(
                text,
                format!("A dot named {name:?} already exists. Choose another name.")
            ),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "Researcher"]);
    assert_eq!(folders(&t.root), ["blog-watcher"]);
}

#[tokio::test]
async fn a_library_item_that_does_not_exist_is_a_plain_error() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut asked = ask("Blog Watcher");
    asked.wanted.uses = Some(uses_github());
    // The maker does not use it either, so it would be an extra; but no card could allow it.
    assert_eq!(
        refused(&t, &run, asked).await,
        "the Library has no MCP server named \"github\""
    );
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(folders(&t.root).is_empty());
}

#[tokio::test]
async fn a_variable_or_consumer_that_does_not_exist_is_a_plain_error() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    // No such variable in the maker's account.
    let mut asked = ask("Blog Watcher");
    asked.wanted.variables = Some(grants("shell", "NO_SUCH_KEY"));
    assert_eq!(
        refused(&t, &run, asked).await,
        "There is no variable named NO_SUCH_KEY. Name one that exists, or leave it out and \
         tell the person it is missing."
    );
    // A server the Library does not have, and one the dot would not use.
    for server in ["ghost", "github"] {
        let mut asked = ask("Blog Watcher");
        asked.wanted.variables = Some(grants(&format!("mcp:{server}"), "GITHUB_TOKEN"));
        assert_eq!(
            refused(&t, &run, asked).await,
            format!(
                "The dot would not use an MCP server named '{server}', so it cannot be given \
                 variables for it. Name the server in uses, or leave those variables out."
            )
        );
    }
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    // A variable that exists, beyond the maker's reach, is for the person to decide.
    let mut asked = ask("Blog Watcher");
    asked.wanted.variables = Some(grants("shell", "GITHUB_TOKEN"));
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.over, None);
    assert_eq!(
        request.extras,
        [Extra::Variable {
            consumer: "shell".into(),
            name: "GITHUB_TOKEN".into()
        }]
    );
}

#[tokio::test]
async fn an_extra_makes_nothing_and_says_what_it_is() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut rx = t.rt.subscribe();
    let mut asked = ask("Blog Watcher");
    asked.wanted.browser = Some(true);
    asked.wanted.folder = Some("notes/blogs".into());
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.extras, [Extra::Browser]);
    assert_eq!(request.over, None);
    // The dot as asked and the dot without the extra, for the person's answer.
    assert!(request.spec.browser);
    assert!(!request.without.browser);
    assert_eq!(request.spec.name, "Blog Watcher");
    // No dot and no folder: only the ask is in the log.
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(!t.work().join("notes").exists());
    assert!(folders(&t.root).is_empty());
    let events = log(&t, &run).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, MakingEventKind::Asked);
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
}

#[tokio::test]
async fn the_fourth_dot_of_a_run_needs_the_person() {
    let t = start().await;
    let maker = maker_with(&t, 3, 10).await;
    let run = live_run(&t, &maker).await;
    for name in ["One", "Two", "Three"] {
        make(&t, &run, name).await;
    }
    let outcome = t.rt.makers().make_dot(&run.id, ask("Four")).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.over, Some(OverLimit::PerRun { nth: 4, limit: 3 }));
    // Nothing of it is beyond reach: only the limit is passed.
    assert!(request.extras.is_empty());
    assert_eq!(request.spec.name, "Four");
    assert_eq!(dot_names(&t).await, ["One", "Researcher", "Three", "Two"]);
    assert_eq!(folders(&t.root), ["one", "three", "two"]);
    // Three dots made, and the ask.
    assert_eq!(log(&t, &run).await.len(), 4);
}

#[tokio::test]
async fn deleting_a_made_dot_does_not_give_the_run_a_place_back() {
    let t = start().await;
    let maker = maker_with(&t, 3, 3).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    make(&t, &run, "Two").await;
    let three = make(&t, &run, "Three").await;
    t.rt.delete_dot(&three.id).await.unwrap();
    // Two exist now, of three it may keep; but this run made three.
    let outcome = t.rt.makers().make_dot(&run.id, ask("Four")).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.over, Some(OverLimit::PerRun { nth: 4, limit: 3 }));
    // The entry of the deleted dot stays in the log, without the dot; the ask follows it.
    let events = log(&t, &run).await;
    assert_eq!(events.len(), 4);
    assert_eq!(events[2].dot_name, "Three");
    assert_eq!(events[2].dot_id, None);
}

#[tokio::test]
async fn a_new_run_starts_the_run_count_again() {
    let t = start().await;
    let maker = maker_with(&t, 1, 10).await;
    let first = live_run(&t, &maker).await;
    make(&t, &first, "One").await;
    let outcome = t.rt.makers().make_dot(&first.id, ask("Two")).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.over, Some(OverLimit::PerRun { nth: 2, limit: 1 }));
    // A wake is a new run (design section 10).
    let second = live_run(&t, &maker).await;
    make(&t, &second, "Two").await;
    assert_eq!(dot_names(&t).await, ["One", "Researcher", "Two"]);
}

#[tokio::test]
async fn the_total_counts_made_dots_that_exist_now() {
    let t = start().await;
    let maker = maker_with(&t, 3, 2).await;
    let first = live_run(&t, &maker).await;
    let one = make(&t, &first, "One").await;
    let second = live_run(&t, &maker).await;
    make(&t, &second, "Two").await;
    let outcome =
        t.rt.makers()
            .make_dot(&second.id, ask("Three"))
            .await
            .unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.over, Some(OverLimit::Total { nth: 3, limit: 2 }));
    // Deleting one frees a place (design section 10). The name is free too: the request that
    // waits holds no name.
    t.rt.delete_dot(&one.id).await.unwrap();
    make(&t, &second, "Three").await;
    assert_eq!(dot_names(&t).await, ["Researcher", "Three", "Two"]);
}

#[tokio::test]
async fn the_run_limit_is_said_first_when_both_are_passed() {
    let t = start().await;
    let maker = maker_with(&t, 1, 1).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let outcome = t.rt.makers().make_dot(&run.id, ask("Two")).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    assert_eq!(request.over, Some(OverLimit::PerRun { nth: 2, limit: 1 }));
}

#[tokio::test]
async fn a_made_dot_never_can_make_dots() {
    // The ask cannot even name the switch: a field `Wanted` does not have is refused.
    for field in ["making", "can_make_dots", "enabled", "org_id", "engine"] {
        let mut call = json!({
            "name": "Blog Watcher",
            "instructions": "Read the blogs.",
            "purpose": "Reads the feeds",
        });
        call[field] = json!(true);
        let err = serde_json::from_value::<MakeDot>(call).unwrap_err();
        assert!(err.to_string().contains(field), "{field}: {err}");
    }
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let dot = make(&t, &run, "Blog Watcher").await;
    assert!(!dot.spec.making.on);
    // Nor does a spec that says otherwise get through the path the person's answer takes.
    let mut spec = dot.spec.clone();
    spec.name = "Second".into();
    spec.workdir = String::new();
    spec.making.on = true;
    spec.enabled = false;
    let second =
        t.rt.makers()
            .create_made(&maker, &run.id, &run.root_run_id, spec, "Another", None)
            .await
            .unwrap();
    assert_eq!(second.spec.making, Making::default());
    assert!(second.spec.enabled);
}

#[tokio::test]
async fn a_made_dots_grants_are_exactly_what_was_asked() {
    let t = start().await;
    add_github(&t).await;
    // The person's dot is granted the variable its server reads.
    let mut s = maker_spec(&t);
    s.uses = uses_github();
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    assert_eq!(maker.spec.variables, grants("mcp:github", "GITHUB_TOKEN"));
    let run = live_run(&t, &maker).await;
    // The made dot uses the server and names no variables: it holds none.
    let mut asked = ask("Plain");
    asked.wanted.uses = Some(uses_github());
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Made { dot, .. } = outcome else {
        panic!("not made: {outcome:?}");
    };
    assert_eq!(dot.spec.uses, uses_github());
    assert_eq!(dot.spec.variables, Grants::default());
    // Named, it holds exactly that.
    let mut asked = ask("Granted");
    asked.wanted.uses = Some(uses_github());
    asked.wanted.variables = Some(grants("mcp:github", "GITHUB_TOKEN"));
    let outcome = t.rt.makers().make_dot(&run.id, asked).await.unwrap();
    let MakeOutcome::Made { dot, .. } = outcome else {
        panic!("not made: {outcome:?}");
    };
    assert_eq!(dot.spec.variables, grants("mcp:github", "GITHUB_TOKEN"));
}

#[tokio::test]
async fn a_made_dot_is_never_moved_from_worktree_copies_to_the_folder_itself() {
    let t = start().await;
    // A maker that works in copies of a folder that is not ready for them (no repository, as
    // when the repository went after the maker was saved), stored as it is.
    let mut s = maker_spec(&t);
    s.workspace_mode = WorkspaceMode::Worktree;
    let maker = t.rt.store().create_dot(&s, None, "default").await.unwrap();
    let run = live_run(&t, &maker).await;
    let mut asked = ask("Blog Watcher");
    asked.wanted.folder = Some(".".into());
    let said = refused(&t, &run, asked).await;
    assert_eq!(
        said,
        "The new dot would work in copies of that folder, as its maker does, but the folder is \
         not the top of a git repository with a commit. Leave the folder blank for a new one."
    );
    assert_eq!(dot_names(&t).await, ["Researcher"]);

    // A dot the person makes there still works in the folder itself.
    let mut own = maker_spec(&t);
    own.name = "Notes".into();
    own.making.on = false;
    own.workspace_mode = WorkspaceMode::Worktree;
    let dot = t.rt.create_dot(own, None, None).await.unwrap();
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
}

#[test]
fn a_tool_calls_json_reads_as_an_ask() {
    let call = json!({
        "name": "Blog Watcher",
        "instructions": "Read the three blogs and list what is new.",
        "purpose": "Watches three blogs for new posts",
        "folder": "blogs",
        "permissions": "read-only",
        "uses": {"mcp_servers": ["fetch"]},
        "variables": {"mcp:fetch": ["FETCH_KEY"]},
        "max_turns": 20,
        "schedule": {"rule": {"kind": "every", "minutes": 60}, "catch_up": true, "until": null},
        "first_job": "Read them now and tell me what is new."
    });
    let asked: MakeDot = serde_json::from_value(call.clone()).unwrap();
    assert_eq!(asked.purpose, "Watches three blogs for new posts");
    assert_eq!(
        asked.first_job.as_deref(),
        Some("Read them now and tell me what is new.")
    );
    assert_eq!(asked.wanted.name.as_deref(), Some("Blog Watcher"));
    assert_eq!(asked.wanted.folder.as_deref(), Some("blogs"));
    assert_eq!(asked.wanted.max_turns, Some(20));
    assert_eq!(
        asked.wanted.variables,
        Some(grants("mcp:fetch", "FETCH_KEY"))
    );
    assert!(matches!(asked.wanted.schedule, Some(Some(_))));
    // It is written as it is read: one flat object.
    let written = serde_json::to_value(&asked).unwrap();
    assert_eq!(written["name"], "Blog Watcher");
    assert_eq!(written["purpose"], "Watches three blogs for new posts");
    assert_eq!(written["permissions"], "read-only");
    assert_eq!(serde_json::from_value::<MakeDot>(written).unwrap(), asked);
    // The least a call carries; a model writes `null` for what it leaves out.
    let least: MakeDot = serde_json::from_value(json!({
        "name": "Blog Watcher",
        "instructions": "Read the blogs.",
        "purpose": "Reads the feeds",
        "first_job": null,
        "folder": null
    }))
    .unwrap();
    assert_eq!(least.first_job, None);
    assert_eq!(least.wanted.folder, None);
    // The purpose is needed, and is text.
    for purpose in [json!(null), json!(7)] {
        let mut call = call.clone();
        call["purpose"] = purpose;
        let err = serde_json::from_value::<MakeDot>(call).unwrap_err();
        assert!(err.to_string().contains("purpose"), "{err}");
    }
    let mut call = call.clone();
    call.as_object_mut().unwrap().remove("purpose");
    let err = serde_json::from_value::<MakeDot>(call).unwrap_err();
    assert!(err.to_string().contains("purpose"), "{err}");
    assert!(serde_json::from_value::<MakeDot>(json!(["Blog Watcher"])).is_err());
}

#[test]
fn a_passed_limit_says_itself_in_one_line() {
    assert_eq!(
        OverLimit::PerRun { nth: 4, limit: 3 }.line(),
        "4th dot in this run; the limit is 3"
    );
    assert_eq!(
        OverLimit::Total { nth: 11, limit: 10 }.line(),
        "11th made dot kept; the limit is 10"
    );
    let nth = |nth| OverLimit::PerRun { nth, limit: 1 }.line();
    assert!(nth(2).starts_with("2nd dot"));
    assert!(nth(3).starts_with("3rd dot"));
    assert!(nth(12).starts_with("12th dot"));
    assert!(nth(21).starts_with("21st dot"));
    // As it is stored with a make request.
    assert_eq!(
        serde_json::to_value(OverLimit::PerRun { nth: 4, limit: 3 }).unwrap(),
        json!({"limit": "per_run", "nth": 4, "max": 3})
    );
    assert_eq!(
        serde_json::from_value::<OverLimit>(json!({"limit": "total", "nth": 11, "max": 10}))
            .unwrap(),
        OverLimit::Total { nth: 11, limit: 10 }
    );
}

// ---- fix 14b part 2

/// A connection to the runtime's database beside its own pool, for what no API does.
async fn db(t: &T) -> sqlx::SqliteConnection {
    SqliteConnectOptions::new()
        .filename(t.dir.path().join("data").join("dots.db"))
        .foreign_keys(false)
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_dot_whose_log_entry_was_lost_still_counts_toward_the_runs_limit() {
    let t = start().await;
    let maker = maker_with(&t, 1, 10).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    // As when the dot was stored and writing its entry failed.
    let mut conn = db(&t).await;
    sqlx::query("DELETE FROM making_events WHERE run_id = ? AND kind = 'made'")
        .bind(&run.id)
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    assert_eq!(t.rt.store().count_made_in_run(&run.id).await.unwrap(), 1);
    match t.rt.makers().make_dot(&run.id, ask("Two")).await.unwrap() {
        MakeOutcome::Waiting { request, .. } => {
            assert_eq!(request.over, Some(OverLimit::PerRun { nth: 2, limit: 1 }))
        }
        other => panic!("made past the limit: {other:?}"),
    }
    assert_eq!(dot_names(&t).await, ["One", "Researcher"]);
}

#[tokio::test]
async fn a_purpose_or_folder_that_is_not_plain_text_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one_line = "The purpose is one line of 1 to 200 characters.";
    let plain = "The purpose is plain text: no tabs, control characters or marks that change the \
                 direction of text.";
    for (purpose, said) in [
        ("Reads\u{2028}The browser: not needed", one_line),
        ("Reads\u{2029}feeds", one_line),
        ("Reads\u{0085}feeds", one_line),
        ("Reads\tfeeds", plain),
        ("Reads \u{202E}sdeef", plain),
        ("Reads \u{2066}feeds\u{2069}", plain),
        ("Reads \u{200F}feeds", plain),
        ("Reads \u{0007}feeds", plain),
    ] {
        let mut asked = ask("Blog Watcher");
        asked.purpose = purpose.into();
        assert_eq!(refused(&t, &run, asked).await, said, "{purpose:?}");
    }
    for folder in ["notes\nThe browser", "notes/\u{202E}sgol", "notes\u{2028}x"] {
        let mut asked = ask("Blog Watcher");
        asked.wanted.folder = Some(folder.into());
        assert_eq!(
            refused(&t, &run, asked).await,
            "The folder is plain text: no line breaks, control characters or marks that change \
             the direction of text.",
            "{folder:?}"
        );
    }
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(!t.work().join("notes").exists());
}

#[tokio::test]
async fn a_made_dots_name_is_stored_without_the_spaces_around_it() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let dot = make(&t, &run, "  Blog Watcher ").await;
    assert_eq!(dot.spec.name, "Blog Watcher");
    assert_eq!(
        t.rt.store().get_dot(&dot.id).await.unwrap().spec.name,
        "Blog Watcher"
    );
    // So " Blog Watcher" is the same name, not a second dot beside it.
    assert_eq!(
        refused(&t, &run, ask(" Blog Watcher")).await,
        "A dot named \"Blog Watcher\" already exists. Choose another name."
    );
}

/// Makes `link` lead to the folder `to`: a symlink, or on Windows a junction where this PC
/// may not make symlinks. `false` when it can make neither.
fn link_dir(to: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(to, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(to)
            .output()
            .is_ok_and(|out| out.status.success())
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(to, link).is_ok()
    }
}

#[tokio::test]
async fn a_folder_that_leads_outside_once_made_is_removed_and_nothing_is_made() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    // Judged within reach while `notes` was a plain folder that did not exist yet.
    let reach = dots_core::making::reach::reach_new(
        &maker,
        &Wanted {
            folder: Some("notes/blogs".into()),
            ..ask("Blog Watcher").wanted
        },
        Path::new(""),
    )
    .unwrap();
    assert!(reach.extras.is_empty());
    // Then a junction to a folder outside is planted where `notes` was to be.
    let outside = t.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    if !link_dir(&outside, &t.work().join("notes")) {
        eprintln!("skipped: this PC can make neither a symlink nor a junction");
        return;
    }
    let err =
        t.rt.makers()
            .create_made(
                &maker,
                &run.id,
                &run.root_run_id,
                reach.spec,
                "Reads the feeds",
                reach.create_folder,
            )
            .await
            .unwrap_err();
    match err {
        Error::Invalid(text) => assert!(
            text.contains("leads somewhere else now that it exists, so the dot was not made"),
            "{text}"
        ),
        other => panic!("{other:?}"),
    }
    // The folder made outside is gone again, and no dot or entry was made.
    assert!(!outside.join("blogs").exists());
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(log(&t, &run).await.is_empty());
}

#[tokio::test]
async fn two_makes_at_once_with_one_place_left_make_one_dot() {
    let t = start().await;
    let maker = maker_with(&t, 3, 1).await;
    let run = live_run(&t, &maker).await;
    let makers = t.rt.makers();
    let (a, b) = tokio::join!(
        makers.make_dot(&run.id, ask("One")),
        makers.make_dot(&run.id, ask("Two"))
    );
    let outcomes = [a.unwrap(), b.unwrap()];
    let made = outcomes
        .iter()
        .filter(|o| matches!(o, MakeOutcome::Made { .. }))
        .count();
    assert_eq!(made, 1, "{outcomes:?}");
    let waiting: Vec<_> = outcomes
        .iter()
        .filter_map(|o| match o {
            MakeOutcome::Waiting { request, .. } => Some(request.over.clone()),
            MakeOutcome::Made { .. } => None,
        })
        .collect();
    assert_eq!(waiting, [Some(OverLimit::Total { nth: 2, limit: 1 })]);
    assert_eq!(t.rt.store().count_made_dots(&maker.id).await.unwrap(), 1);
}

#[tokio::test]
async fn the_switch_turned_off_between_two_calls_of_one_run_refuses_the_second() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let mut off = maker.spec.clone();
    off.making.on = false;
    t.rt.update_dot(&maker.id, off).await.unwrap();
    assert_eq!(
        refused(&t, &run, ask("Two")).await,
        "Researcher may not make dots. The person turns that on in the dot's settings."
    );
    assert_eq!(dot_names(&t).await, ["One", "Researcher"]);
}

// ---- fix 14b part 2b: a settings folder the person allowed; the model as plain text

#[tokio::test]
async fn a_claude_folder_the_person_allowed_is_made_and_stays() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask("Blog Watcher");
    wanted.wanted.folder = Some(".claude/x".into());
    let outcome = t.rt.makers().make_dot(&run.id, wanted).await.unwrap();
    let MakeOutcome::Waiting { request, .. } = outcome else {
        panic!("made: {outcome:?}");
    };
    let folder = real_path(&t.work()).join(".claude").join("x");
    let path = folder.to_string_lossy().into_owned();
    assert_eq!(
        request.extras,
        [Extra::SettingsFolder { path: path.clone() }]
    );

    let after =
        t.rt.makers()
            .decide_request(
                &request.id,
                dots_core::making::requests::MakeDecision::Allow,
                None,
                t.rt.makers()
                    .request_card(&request.id)
                    .await
                    .unwrap()
                    .shown
                    .as_deref(),
            )
            .await
            .unwrap();

    let made = after.made_dot_id.as_deref().expect("the dot was made");
    let dot = t.rt.store().get_dot(made).await.unwrap();
    assert_eq!(dot.spec.workdir, path);
    assert!(folder.is_dir());
}

#[tokio::test]
async fn a_model_that_is_not_plain_text_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let rule = dots_core::making::service::MODEL_CHARS;
    assert_eq!(
        rule,
        "The model is plain text: no line breaks, control characters or marks that change the \
         direction of text."
    );
    for model in ["opus\u{202E}tset", "opus\nModel: trusted", "a\u{2028}b"] {
        let mut wanted = ask("Blog Watcher");
        wanted.wanted.model = Some(model.into());
        assert_eq!(refused(&t, &run, wanted).await, rule, "{model:?}");
    }
    make(&t, &run, "One").await;
    let change = dots_core::making::manage::ChangeDot {
        dot: "One".into(),
        purpose: None,
        wanted: Wanted {
            model: Some("opus\u{2066}x".into()),
            ..Wanted::default()
        },
    };
    match t.rt.makers().change_dot(&run.id, change).await {
        Err(Error::Invalid(text)) => assert_eq!(text, rule),
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert_eq!(dot_names(&t).await, ["One", "Researcher"]);
}

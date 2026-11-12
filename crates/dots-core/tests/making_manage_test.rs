//! A maker lists, changes and deletes the dots it made (dots-make-dots design section 9): only
//! its own made dots, by name; a change within its reach is saved at once and changes only what
//! it names; a change that gives the dot more than the maker has waits for the person, whose
//! answer is put on the dot as it is then; and a delete takes the dot as the person's does.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::library::{LibraryConfig, McpServerConfig, Uses};
use dots_core::making::log::{MakingEvent, MakingEventKind};
use dots_core::making::manage::{
    change_went_message, made_dots_text, ChangeDot, ChangeOutcome, MadeDotLine,
};
use dots_core::making::reach::{Extra, Wanted};
use dots_core::making::requests::{MakeDecision, MakeRequest, RequestKind, RequestStatus};
use dots_core::making::service::{MakeDot, MakeOutcome, OverLimit};
use dots_core::model::{Dot, DotSpec, Making, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::vault::{EntryKind, Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::ConnectOptions;
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

struct T {
    dir: TempDir,
    rt: Runtime,
}

impl T {
    /// The maker's folder.
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }
}

/// As `making_request_test.rs` has it: no queued run starts, so a run is put in the running
/// state by hand, and a wake of the maker's topic waits where a test can read it.
fn config(dir: &TempDir) -> Config {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.new_dot_root = dir.path().join("Dots");
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    cfg.max_concurrent_runs = 0;
    cfg
}

async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::start(config(&dir)).await.unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    T { dir, rt }
}

/// The spec of a dot named `name` that works in the test's `work` folder itself and may make
/// dots.
fn maker_spec(t: &T, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.work().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.making.on = true;
    s
}

/// A maker named Researcher, created as the person creates a dot.
async fn maker(t: &T) -> Dot {
    t.rt.create_dot(maker_spec(t, "Researcher"), None, None)
        .await
        .unwrap()
}

/// A dot of `dot` that waits in the queue.
async fn queued_run(t: &T, dot: &Dot) -> Run {
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "watch three blogs", "typed": true}));
    t.rt.store().create_run(&new).await.unwrap()
}

/// A run of `dot` that is running: a chat the person started with a typed message.
async fn live_run(t: &T, dot: &Dot) -> Run {
    let run = queued_run(t, dot).await;
    assert!(t.rt.store().claim_run(&run.id).await.unwrap());
    t.rt.store().get_run(&run.id).await.unwrap()
}

/// Ends `run` as `status`.
async fn end(t: &T, run: &Run, status: RunStatus) -> Run {
    t.rt.store()
        .finish_run(&run.id, status, None, None)
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

/// A change to the made dot named `dot` that names what `wanted` names.
fn change(dot: &str, wanted: Wanted) -> ChangeDot {
    ChangeDot {
        dot: dot.into(),
        purpose: None,
        wanted,
    }
}

/// A change of the instructions alone.
fn new_instructions(dot: &str) -> ChangeDot {
    change(
        dot,
        Wanted {
            instructions: Some("List only the posts on Rust.".into()),
            ..Wanted::default()
        },
    )
}

/// A change that asks for the browser, which no maker of these tests has, and new
/// instructions: one extra, and one thing within reach.
fn browser_and_instructions(dot: &str) -> ChangeDot {
    let mut asked = new_instructions(dot);
    asked.wanted.browser = Some(true);
    asked
}

/// Changes the dot at once: the dot after, and what changed.
async fn changed(t: &T, run: &Run, ask: ChangeDot) -> (Dot, Vec<String>) {
    match t.rt.makers().change_dot(&run.id, ask).await.unwrap() {
        ChangeOutcome::Changed { dot, what } => (dot, what),
        other => panic!("not changed: {other:?}"),
    }
}

/// Asks for a change that must come back waiting for the person: its request.
async fn change_request(t: &T, run: &Run, ask: ChangeDot) -> MakeRequest {
    match t.rt.makers().change_dot(&run.id, ask).await.unwrap() {
        ChangeOutcome::Waiting { request, .. } => request,
        other => panic!("changed: {other:?}"),
    }
}

/// The person's answer to the card they see, which must be taken: it sends back the card's
/// limit line, as the UI does.
async fn decide(t: &T, request: &MakeRequest, decision: MakeDecision) -> MakeRequest {
    let makers = t.rt.makers();
    let card = makers.request_card(&request.id).await.unwrap();
    makers
        .decide_request(&request.id, decision, card.limit.as_deref())
        .await
        .unwrap()
}

/// The dot as it is stored now.
async fn stored(t: &T, dot: &Dot) -> Dot {
    t.rt.store().get_dot(&dot.id).await.unwrap()
}

/// The text of an `Invalid`; anything else fails the test.
fn invalid<V: std::fmt::Debug>(result: Result<V, Error>) -> String {
    match result {
        Err(Error::Invalid(text)) => text,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// The text of a `Conflict`; anything else fails the test.
fn conflict<V: std::fmt::Debug>(result: Result<V, Error>) -> String {
    match result {
        Err(Error::Conflict(text)) => text,
        other => panic!("expected Conflict, got {other:?}"),
    }
}

/// The messages of the wakes that wait: the maker's run is still running, so none is
/// delivered.
async fn wakes(t: &T) -> Vec<String> {
    let wakes = t.rt.store().waiting_making_wakes().await.unwrap();
    wakes.into_iter().map(|w| w.message).collect()
}

/// The topics named by the `MakingUpdated` events published since the last call.
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

/// The kinds of the log's entries, oldest first.
async fn kinds(t: &T, run: &Run) -> Vec<MakingEventKind> {
    log(t, run).await.iter().map(|e| e.kind).collect()
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

fn grants(pairs: &[(&str, &str)]) -> Grants {
    Grants(
        pairs
            .iter()
            .map(|(consumer, name)| (consumer.to_string(), vec![name.to_string()]))
            .collect(),
    )
}

/// The person saves `dot` with `edit` put on it.
async fn person_edits(t: &T, dot: &Dot, edit: impl FnOnce(&mut DotSpec)) -> Dot {
    let mut spec = stored(t, dot).await.spec;
    edit(&mut spec);
    t.rt.update_dot(&dot.id, spec).await.unwrap()
}

/// The sentence for a dot named `name` that Researcher did not make.
fn not_made(name: &str) -> String {
    format!(
        "Researcher did not make a dot named \"{name}\". list_made_dots shows the dots it made."
    )
}

const SWITCH_OFF: &str =
    "Researcher may not make dots. The person turns that on in the dot's settings.";
const BUSY: &str = "\"One\" has a run going or queued. Wait for it to end, then delete it.";

// ---- list_made_dots

#[tokio::test]
async fn the_list_shows_only_this_makers_dots_with_their_state() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    // A running one, one whose last run failed, one that is switched off, one with a schedule.
    let alpha = make(&t, &run, "Alpha").await;
    live_run(&t, &alpha).await;
    let beta = make(&t, &run, "Beta").await;
    let failed = live_run(&t, &beta).await;
    let failed = end(&t, &failed, RunStatus::Failed).await;
    let delta = make(&t, &run, "Delta").await;
    t.rt.set_dot_enabled(&delta.id, false).await.unwrap();
    // Past the run's limit of 3 it would ask, so the fourth is made by another run.
    let second = live_run(&t, &maker).await;
    let mut gamma = ask("Gamma");
    gamma.wanted = serde_json::from_value(json!({
        "name": "Gamma",
        "instructions": "Read the feeds.",
        "schedule": {"rule": {"kind": "daily", "time": "09:00"}}
    }))
    .unwrap();
    t.rt.makers().make_dot(&second.id, gamma).await.unwrap();
    // Another maker's dot and a dot of the person's are not this maker's.
    let writer =
        t.rt.create_dot(maker_spec(&t, "Writer"), None, None)
            .await
            .unwrap();
    let writers_run = live_run(&t, &writer).await;
    make(&t, &writers_run, "Other").await;
    t.rt.create_dot(maker_spec(&t, "Mine"), None, None)
        .await
        .unwrap();

    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();

    let line = |name: &str| MadeDotLine {
        name: name.into(),
        purpose: "Reads the feeds".into(),
        schedule: None,
        enabled: true,
        running: false,
        last_run: None,
        waiting_change: false,
        job: None,
    };
    let ended = failed.ended_at.clone().expect("the run ended");
    assert_eq!(
        lines,
        [
            MadeDotLine {
                running: true,
                ..line("Alpha")
            },
            MadeDotLine {
                last_run: Some(format!("failed at {ended}")),
                ..line("Beta")
            },
            MadeDotLine {
                enabled: false,
                ..line("Delta")
            },
            MadeDotLine {
                schedule: Some("daily at 09:00".into()),
                ..line("Gamma")
            },
        ]
    );
    assert_eq!(
        made_dots_text("Researcher", &lines, &[]),
        format!(
            "\"Alpha\": Reads the feeds; no schedule; running; no run has ended yet\n\
             \"Beta\": Reads the feeds; no schedule; last run: failed at {ended}\n\
             \"Delta\": Reads the feeds; no schedule; off; no run has ended yet\n\
             \"Gamma\": Reads the feeds; schedule: daily at 09:00; no run has ended yet"
        )
    );
    // The other maker sees its own.
    let theirs = t.rt.makers().list_made_dots(&writers_run.id).await.unwrap();
    assert_eq!(theirs, [line("Other")]);
}

#[tokio::test]
async fn the_list_text_names_waiting_requests() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    // A new dot that waits for the person, and a change to One that does.
    let mut two = ask("Two");
    two.wanted.browser = Some(true);
    t.rt.makers().make_dot(&run.id, two).await.unwrap();
    change_request(&t, &run, browser_and_instructions("One")).await;

    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].waiting_change);
    let waiting =
        t.rt.store()
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap();
    assert_eq!(waiting.len(), 2);
    assert_eq!(
        made_dots_text("Researcher", &lines, &waiting),
        "\"One\": Reads the feeds; no schedule; no run has ended yet; a change is waiting for \
         the person\n\
         Waiting for the person: \"Two\""
    );
}

#[tokio::test]
async fn a_maker_with_no_dots_is_told_so() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();
    assert!(lines.is_empty());
    assert_eq!(
        made_dots_text("Researcher", &lines, &[]),
        "Researcher has made no dots yet."
    );
    // A request that waits is still named.
    let mut two = ask("Two");
    two.wanted.browser = Some(true);
    t.rt.makers().make_dot(&run.id, two).await.unwrap();
    let waiting =
        t.rt.store()
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap();
    assert_eq!(
        made_dots_text("Researcher", &lines, &waiting),
        "Researcher has made no dots yet.\nWaiting for the person: \"Two\""
    );
}

// ---- change_dot, within reach

#[tokio::test]
async fn only_the_named_settings_change() {
    let t = start().await;
    add_github(&t).await;
    // The maker uses the server, so the dot holds nothing beyond its reach after the edit: a
    // dot that does asks the person for such a change (`a_dot_holding_more_than_its_maker...`).
    let mut s = maker_spec(&t, "Researcher");
    s.uses = uses_github();
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // The person edits it: another turn limit, and a Library item it did not use, whose
    // variable a person's save grants.
    let edited = person_edits(&t, &one, |spec| {
        spec.max_turns = 25;
        spec.uses = uses_github();
    })
    .await;
    assert_eq!(
        edited.spec.variables,
        grants(&[("mcp:github", "GITHUB_TOKEN")])
    );
    let mut rx = t.rt.subscribe();

    let outcome =
        t.rt.makers()
            .change_dot(&run.id, new_instructions("One"))
            .await
            .unwrap();

    assert_eq!(outcome.tool_text(), "Changed \"One\": instructions.");
    let ChangeOutcome::Changed { dot, what } = outcome else {
        unreachable!()
    };
    assert_eq!(what, ["instructions"]);
    assert_eq!(dot, stored(&t, &one).await);
    // The named setting changed, and nothing else: both edits of the person stay.
    let mut expected = edited.spec.clone();
    expected.instructions = "List only the posts on Rust.".into();
    assert_eq!(dot.spec, expected);
    assert_eq!(dot.made, one.made);
    // The "changed" card's entry is in the calling run's topic.
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [MakingEventKind::Made, MakingEventKind::Changed]
    );
    let event = &events[1];
    assert_eq!(event.maker_dot_id, maker.id);
    assert_eq!(event.run_id, run.id);
    assert_eq!(event.root_run_id, run.root_run_id);
    assert_eq!(event.dot_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(event.dot_name, "One");
    assert_eq!(event.detail, json!({"what": ["instructions"]}));
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    // No request, no wake.
    assert!(t
        .rt
        .store()
        .list_make_requests(None, false)
        .await
        .unwrap()
        .is_empty());
    assert!(wakes(&t).await.is_empty());
}

#[tokio::test]
async fn a_rename_shows_in_the_list_and_keeps_who_made_it() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let rename = |from: &str, to: &str| {
        change(
            from,
            Wanted {
                name: Some(to.into()),
                ..Wanted::default()
            },
        )
    };

    // The dot is named in any letter case.
    let (dot, what) = changed(&t, &run, rename("one", "Uno")).await;

    assert_eq!(what, ["name (was One)"]);
    assert_eq!(dot.id, one.id);
    assert_eq!(dot.spec.name, "Uno");
    assert_eq!(dot.made, one.made);
    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].name, "Uno");
    assert_eq!(log(&t, &run).await[1].dot_name, "Uno");
    // Its old name is no dot of the maker's now.
    assert_eq!(
        invalid(
            t.rt.makers()
                .change_dot(&run.id, new_instructions("One"))
                .await
        ),
        not_made("One")
    );
    // Its own name in another case is free for it, as it is for the person's rename.
    let (dot, what) = changed(&t, &run, rename("Uno", "UNO")).await;
    assert_eq!(dot.spec.name, "UNO");
    assert_eq!(what, ["name (was Uno)"]);
    // Another dot's name is not, in any case.
    assert_eq!(
        conflict(
            t.rt.makers()
                .change_dot(&run.id, rename("UNO", "researcher"))
                .await
        ),
        "A dot named \"researcher\" already exists. Choose another name."
    );
    assert_eq!(dot_names(&t).await, ["Researcher", "UNO"]);
}

#[tokio::test]
async fn the_purpose_changes() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let mut asked = change("One", Wanted::default());
    asked.purpose = Some("  Watches the three blogs ".into());

    let (dot, what) = changed(&t, &run, asked.clone()).await;

    assert_eq!(what, ["purpose"]);
    let made = dot.made.clone().expect("a made dot");
    assert_eq!(made.purpose, "Watches the three blogs");
    // Who made it, and in which run, stays.
    let was = one.made.clone().unwrap();
    assert_eq!(made.by_dot_id, was.by_dot_id);
    assert_eq!(made.by_name, was.by_name);
    assert_eq!(made.run_id, was.run_id);
    assert_eq!(dot.spec, one.spec);
    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();
    assert_eq!(lines[0].purpose, "Watches the three blogs");
    // The same again changes nothing, and says so.
    let outcome = t.rt.makers().change_dot(&run.id, asked).await.unwrap();
    assert_eq!(
        outcome.tool_text(),
        "\"One\" is already as you asked. Nothing was changed."
    );
    assert_eq!(log(&t, &run).await.len(), 2);
    // A purpose is one line.
    let mut two_lines = change("One", Wanted::default());
    two_lines.purpose = Some("Watches\nblogs".into());
    assert_eq!(
        invalid(t.rt.makers().change_dot(&run.id, two_lines).await),
        "The purpose is one line of 1 to 200 characters."
    );
}

#[tokio::test]
async fn a_schedule_is_set_and_cleared() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    assert_eq!(one.spec.schedule, None);
    // As a tool call carries it: one flat object.
    let call = json!({
        "dot": "One",
        "purpose": "Reads the feeds each morning",
        "schedule": {"rule": {"kind": "daily", "time": "09:00"}}
    });
    let asked: ChangeDot = serde_json::from_value(call).unwrap();
    assert_eq!(asked.dot, "One");
    assert_eq!(
        asked.purpose.as_deref(),
        Some("Reads the feeds each morning")
    );

    let (dot, what) = changed(&t, &run, asked).await;

    assert_eq!(what, ["purpose", "schedule"]);
    let schedule = dot.spec.schedule.clone().expect("a schedule");
    assert_eq!(
        serde_json::to_value(&schedule).unwrap()["rule"],
        json!({"kind": "daily", "time": "09:00"})
    );
    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();
    assert_eq!(lines[0].schedule.as_deref(), Some("daily at 09:00"));

    // The word "none" clears it; `null` names nothing.
    let clear: ChangeDot =
        serde_json::from_value(json!({"dot": "One", "schedule": "none", "model": null})).unwrap();
    assert_eq!(
        serde_json::to_value(&clear).unwrap(),
        json!({"dot": "One", "schedule": "none"})
    );
    let (dot, what) = changed(&t, &run, clear).await;
    assert_eq!(what, ["schedule"]);
    assert_eq!(dot.spec.schedule, None);
    assert_eq!(
        t.rt.makers().list_made_dots(&run.id).await.unwrap()[0].schedule,
        None
    );

    // A call must name the dot, and can not name what a maker may not set.
    assert!(serde_json::from_value::<ChangeDot>(json!({"instructions": "x"})).is_err());
    for forbidden in ["making", "engine", "enabled", "first_job"] {
        let call = json!({"dot": "One", forbidden: true});
        assert!(
            serde_json::from_value::<ChangeDot>(call).is_err(),
            "{forbidden}"
        );
    }
}

#[tokio::test]
async fn dropping_a_server_drops_its_grants_and_nothing_else() {
    let t = start().await;
    add_github(&t).await;
    // The maker uses the server and is granted the variable for it and for the shell.
    let mut s = maker_spec(&t, "Researcher");
    s.uses = uses_github();
    s.variables = grants(&[("shell", "GITHUB_TOKEN")]);
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let both = grants(&[("mcp:github", "GITHUB_TOKEN"), ("shell", "GITHUB_TOKEN")]);
    assert_eq!(maker.spec.variables, both);
    let run = live_run(&t, &maker).await;
    let mut asked = ask("One");
    asked.wanted.uses = Some(uses_github());
    asked.wanted.variables = Some(both.clone());
    let MakeOutcome::Made { dot: one, .. } = t.rt.makers().make_dot(&run.id, asked).await.unwrap()
    else {
        panic!("not made")
    };
    assert_eq!(one.spec.variables, both);

    // The change names the Library items and not the variables.
    let no_items = change(
        "One",
        Wanted {
            uses: Some(Uses::default()),
            ..Wanted::default()
        },
    );
    let (dot, what) = changed(&t, &run, no_items).await;

    assert_eq!(what, ["Library items", "variables"]);
    assert_eq!(dot.spec.uses, Uses::default());
    // The server's grants went with it; the shell's stay.
    assert_eq!(dot.spec.variables, grants(&[("shell", "GITHUB_TOKEN")]));
    let mut expected = one.spec.clone();
    expected.uses = Uses::default();
    expected.variables = grants(&[("shell", "GITHUB_TOKEN")]);
    assert_eq!(dot.spec, expected);

    // Naming the server again grants nothing by itself: a maker's change never adds a grant.
    let items_again = change(
        "One",
        Wanted {
            uses: Some(uses_github()),
            ..Wanted::default()
        },
    );
    let (dot, what) = changed(&t, &run, items_again).await;
    assert_eq!(what, ["Library items"]);
    assert_eq!(dot.spec.uses, uses_github());
    assert_eq!(dot.spec.variables, grants(&[("shell", "GITHUB_TOKEN")]));
}

#[tokio::test]
async fn removing_things_never_asks() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // The person gives it more than the maker has.
    let edited = person_edits(&t, &one, |spec| {
        spec.browser = true;
        spec.max_turns = 80;
        spec.uses = uses_github();
    })
    .await;
    // Naming what the dot holds already is no extra, and changes nothing.
    let same = change(
        "One",
        Wanted {
            browser: Some(true),
            max_turns: Some(80),
            uses: Some(uses_github()),
            ..Wanted::default()
        },
    );
    let outcome = t.rt.makers().change_dot(&run.id, same).await.unwrap();
    assert_eq!(
        outcome.tool_text(),
        "\"One\" is already as you asked. Nothing was changed."
    );
    assert_eq!(stored(&t, &one).await, edited);

    // Taking each of them away is within reach.
    let less = change(
        "One",
        Wanted {
            browser: Some(false),
            max_turns: Some(10),
            uses: Some(Uses::default()),
            ..Wanted::default()
        },
    );
    let (dot, what) = changed(&t, &run, less).await;

    assert_eq!(
        what,
        ["Library items", "variables", "browser", "turn limit"]
    );
    assert!(!dot.spec.browser);
    assert_eq!(dot.spec.max_turns, 10);
    assert_eq!(dot.spec.uses, Uses::default());
    assert_eq!(dot.spec.variables, Grants::default());
    assert!(t
        .rt
        .store()
        .list_make_requests(None, false)
        .await
        .unwrap()
        .is_empty());
    // What was taken away is beyond the maker's reach again.
    let back = change(
        "One",
        Wanted {
            browser: Some(true),
            ..Wanted::default()
        },
    );
    let request = change_request(&t, &run, back).await;
    assert_eq!(request.extras, [Extra::Browser]);
}

#[tokio::test]
async fn naming_nothing_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let blank_folder = change(
        "One",
        Wanted {
            folder: Some("  ".into()),
            ..Wanted::default()
        },
    );
    for nothing in [change("One", Wanted::default()), blank_folder] {
        assert_eq!(
            invalid(t.rt.makers().change_dot(&run.id, nothing).await),
            "Name at least one thing to change."
        );
    }
    assert_eq!(stored(&t, &one).await, one);
    assert_eq!(kinds(&t, &run).await, [MakingEventKind::Made]);
}

#[tokio::test]
async fn a_dot_of_another_maker_or_of_the_person_is_not_found() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let writer =
        t.rt.create_dot(maker_spec(&t, "Writer"), None, None)
            .await
            .unwrap();
    let writers_run = live_run(&t, &writer).await;
    let other = make(&t, &writers_run, "Other").await;
    let mine =
        t.rt.create_dot(maker_spec(&t, "Mine"), None, None)
            .await
            .unwrap();

    // Another maker's dot, the person's, another maker, the maker itself, and no dot at all:
    // one sentence, which does not say which of them exist.
    for name in ["Other", "Mine", "Writer", "Researcher", "Nope"] {
        assert_eq!(
            invalid(
                t.rt.makers()
                    .change_dot(&run.id, new_instructions(name))
                    .await
            ),
            not_made(name)
        );
    }
    assert_eq!(stored(&t, &other).await, other);
    assert_eq!(stored(&t, &mine).await, mine);
    assert_eq!(kinds(&t, &run).await, [MakingEventKind::Made]);
    // The other maker cannot change this maker's dot either.
    assert_eq!(
        invalid(
            t.rt.makers()
                .change_dot(&writers_run.id, new_instructions("One"))
                .await
        ),
        "Writer did not make a dot named \"One\". list_made_dots shows the dots it made."
    );
}

#[tokio::test]
async fn a_folder_change_while_it_runs_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let its_run = live_run(&t, &one).await;
    let move_it = || {
        change(
            "One",
            Wanted {
                folder: Some("notes".into()),
                ..Wanted::default()
            },
        )
    };

    // The store's rule for any dot, passed on as it is.
    assert_eq!(
        conflict(t.rt.makers().change_dot(&run.id, move_it()).await),
        "workdir and workspace_mode cannot change while the dot has queued, running or awaiting \
         runs"
    );
    assert_eq!(stored(&t, &one).await, one);
    // The folder that was made for it is removed again.
    assert!(!t.work().join("notes").exists());
    // Its other settings can change while it runs.
    let (_, what) = changed(&t, &run, new_instructions("One")).await;
    assert_eq!(what, ["instructions"]);
    // Once the run has ended the folder changes, and is made.
    end(&t, &its_run, RunStatus::Succeeded).await;
    let (dot, what) = changed(&t, &run, move_it()).await;
    assert_eq!(what, ["folder"]);
    assert!(t.work().join("notes").is_dir());
    assert!(PathBuf::from(&dot.spec.workdir).ends_with("notes"));
}

// ---- change_dot, beyond reach

#[tokio::test]
async fn an_extra_stores_a_change_request_and_changes_nothing() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let mut rx = t.rt.subscribe();
    let asked = browser_and_instructions("One");

    let outcome =
        t.rt.makers()
            .change_dot(&run.id, asked.clone())
            .await
            .unwrap();

    assert_eq!(
        outcome.tool_text(),
        "Not changed yet: \"One\" needs the person's say-so, because the change gives it more \
         than you have (The browser). The request is waiting for them. Do not wait for it and \
         do not ask again: carry on, and you will be woken with their answer."
    );
    let ChangeOutcome::Waiting {
        request,
        maker_name,
        held,
    } = outcome
    else {
        panic!("changed")
    };
    assert_eq!(maker_name, "Researcher");
    // The dot itself holds nothing beyond the maker's reach.
    assert!(!held);
    assert_eq!(request.maker_dot_id, maker.id);
    assert_eq!(request.run_id, run.id);
    assert_eq!(request.root_run_id, run.root_run_id);
    assert_eq!(request.kind, RequestKind::Change);
    assert_eq!(request.dot_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(request.name, "One");
    assert_eq!(request.purpose, "Reads the feeds");
    // The ask itself is kept, for the answer; and the dot as asked and without the extra.
    assert_eq!(request.ask, Some(asked));
    assert!(request.spec.browser);
    assert!(!request.without.browser);
    assert_eq!(request.without.instructions, "List only the posts on Rust.");
    assert_eq!(request.extras, [Extra::Browser]);
    // The limits do not apply to a change.
    assert_eq!(request.over, None);
    assert_eq!(request.first_job, None);
    assert_eq!(request.status, RequestStatus::Waiting);
    let store = t.rt.store();
    assert_eq!(store.get_make_request(&request.id).await.unwrap(), request);
    assert_eq!(
        store.count_waiting_make_requests(&maker.id).await.unwrap(),
        1
    );
    // Nothing of it is on the dot: not the part within reach either.
    assert_eq!(stored(&t, &one).await, one);
    // The ask card's entry is in the topic's log.
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [MakingEventKind::Made, MakingEventKind::Asked]
    );
    assert_eq!(events[1].dot_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(events[1].dot_name, "One");
    assert_eq!(events[1].detail, json!({"request_id": request.id}));
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    assert!(wakes(&t).await.is_empty());
}

#[tokio::test]
async fn a_second_change_request_for_the_dot_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    make(&t, &run, "Two").await;
    make(&t, &run, "Three").await;
    change_request(&t, &run, browser_and_instructions("One")).await;

    assert_eq!(
        invalid(
            t.rt.makers()
                .change_dot(&run.id, browser_and_instructions("one"))
                .await
        ),
        "A change to \"One\" is already waiting for the person."
    );
    // A change within reach is still made at once.
    let (_, what) = changed(&t, &run, new_instructions("One")).await;
    assert_eq!(what, ["instructions"]);
    assert_eq!(stored(&t, &one).await.spec.browser, one.spec.browser);
    // Another dot's change waits beside it, and a new dot's request: three wait.
    change_request(&t, &run, browser_and_instructions("Two")).await;
    let mut four = ask("Four");
    four.wanted.browser = Some(true);
    t.rt.makers().make_dot(&run.id, four).await.unwrap();
    // A change counts toward the three requests a maker may have waiting.
    assert_eq!(
        invalid(
            t.rt.makers()
                .change_dot(&run.id, browser_and_instructions("Three"))
                .await
        ),
        "Researcher already has 3 requests waiting for the person. Wait to be woken with an \
         answer before asking for more."
    );
    assert_eq!(
        t.rt.store()
            .count_waiting_make_requests(&maker.id)
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn allow_applies_the_change_onto_the_dot_as_it_is_then() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let request = change_request(&t, &run, browser_and_instructions("One")).await;
    // The person edits another setting between the ask and the answer.
    let edited = person_edits(&t, &one, |spec| spec.max_turns = 25).await;
    let mut rx = t.rt.subscribe();

    let after = decide(&t, &request, MakeDecision::Allow).await;

    assert_eq!(after.status, RequestStatus::Allowed);
    assert!(after.decided_at.is_some());
    assert_eq!(after.made_dot_id.as_deref(), Some(one.id.as_str()));
    // Both hold: what the maker asked, and what the person edited since.
    let dot = stored(&t, &one).await;
    let mut expected = edited.spec.clone();
    expected.browser = true;
    expected.instructions = "List only the posts on Rust.".into();
    assert_eq!(dot.spec, expected);
    assert_eq!(dot.spec.max_turns, 25);
    assert_eq!(dot.made, one.made);
    // The maker's own settings do not change: its reach does not grow.
    assert_eq!(stored(&t, &maker).await.spec, maker.spec);
    // The log: the dot, the ask, the change, the answer.
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Made,
            MakingEventKind::Asked,
            MakingEventKind::Changed,
            MakingEventKind::Answered
        ]
    );
    let change = &events[2];
    assert_eq!(change.run_id, run.id);
    assert_eq!(change.dot_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(change.detail, json!({"what": ["instructions", "browser"]}));
    let answered = &events[3];
    assert_eq!(answered.dot_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(answered.dot_name, "One");
    assert_eq!(
        answered.detail,
        json!({"request_id": request.id, "decision": "allow", "dot_name": "One"})
    );
    assert!(updated(&mut rx).contains(&(maker.id.clone(), run.root_run_id.clone())));
    // The maker is woken once, in the topic that asked.
    assert_eq!(
        wakes(&t).await,
        ["The person allowed it: \"One\" was changed as you asked."]
    );
    let wake = &t.rt.store().waiting_making_wakes().await.unwrap()[0];
    assert_eq!(wake.maker_dot_id, maker.id);
    assert_eq!(wake.root_run_id, run.root_run_id);
    // It can not be answered again.
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Refuse, None)
                .await
        ),
        "That request was already answered."
    );
}

#[tokio::test]
async fn without_applies_what_was_within_reach() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let mut asked = browser_and_instructions("One");
    asked.purpose = Some("Lists the posts on Rust".into());
    let request = change_request(&t, &run, asked).await;
    assert_eq!(request.purpose, "Lists the posts on Rust");
    let edited = person_edits(&t, &one, |spec| spec.max_turns = 25).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(card.can_allow);
    assert!(card.can_without);

    let after = decide(&t, &request, MakeDecision::Without).await;

    assert_eq!(after.status, RequestStatus::MadeWithout);
    assert_eq!(after.made_dot_id.as_deref(), Some(one.id.as_str()));
    let dot = stored(&t, &one).await;
    let mut expected = edited.spec.clone();
    expected.instructions = "List only the posts on Rust.".into();
    assert_eq!(dot.spec, expected);
    assert!(!dot.spec.browser);
    assert_eq!(dot.made.unwrap().purpose, "Lists the posts on Rust");
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Made,
            MakingEventKind::Asked,
            MakingEventKind::Changed,
            MakingEventKind::Answered
        ]
    );
    assert_eq!(
        events[2].detail,
        json!({"what": ["purpose", "instructions"]})
    );
    assert_eq!(events[3].detail["decision"], "without");
    assert_eq!(
        wakes(&t).await,
        [
            "The person chose to change \"One\" without what was beyond your reach. Left out: The \
          browser."
        ]
    );
}

#[tokio::test]
async fn refuse_changes_nothing() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let request = change_request(&t, &run, browser_and_instructions("One")).await;

    let after = decide(&t, &request, MakeDecision::Refuse).await;

    assert_eq!(after.status, RequestStatus::Refused);
    assert!(after.decided_at.is_some());
    assert_eq!(after.made_dot_id, None);
    assert_eq!(stored(&t, &one).await, one);
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Made,
            MakingEventKind::Asked,
            MakingEventKind::Answered
        ]
    );
    assert_eq!(events[2].dot_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(
        events[2].detail,
        json!({"request_id": request.id, "decision": "refuse", "dot_name": "One"})
    );
    assert_eq!(
        wakes(&t).await,
        ["The person chose not to change \"One\". Do not ask again for the same thing."]
    );
    // Its place is free: another change to the dot can wait.
    let lines = t.rt.makers().list_made_dots(&run.id).await.unwrap();
    assert!(!lines[0].waiting_change);
    change_request(&t, &run, browser_and_instructions("One")).await;
}

#[tokio::test]
async fn the_card_for_a_change_says_give_more() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    make(&t, &run, "Two").await;
    // Nothing of this change is within reach: the browser and a higher turn limit.
    let more = change(
        "One",
        Wanted {
            browser: Some(true),
            max_turns: Some(50),
            ..Wanted::default()
        },
    );
    let request = change_request(&t, &run, more).await;

    let card = t.rt.makers().request_card(&request.id).await.unwrap();

    assert_eq!(card.maker_name, "Researcher");
    assert_eq!(card.title, "Researcher wants to give One more than it has");
    assert_eq!(
        card.lines,
        ["The browser", "A turn limit of 50 (Researcher has 40)"]
    );
    assert!(card.can_allow);
    // There is nothing else to change, so the buttons are Allow and Don't change it.
    assert!(!card.can_without);
    assert_eq!(card.blocked, None);
    assert_eq!(card.request, request);
    assert_eq!(
        invalid(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Without, None)
                .await
        ),
        "Everything this request asks is beyond reach, so there is nothing to change without \
         it. Allow it, or don't change it."
    );
    assert_eq!(
        t.rt.store()
            .get_make_request(&request.id)
            .await
            .unwrap()
            .status,
        RequestStatus::Waiting
    );
    // As it crosses to the UI.
    let json = serde_json::to_value(&card).unwrap();
    assert_eq!(json["request"]["kind"], "change");
    assert_eq!(json["request"]["dot_id"], one.id.as_str());
    assert_eq!(
        json["request"]["ask"],
        json!({"dot": "One", "browser": true, "max_turns": 50})
    );
    // The title names the dot as it is called now.
    person_edits(&t, &one, |spec| spec.name = "Uno".into()).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.title, "Researcher wants to give Uno more than it has");
    assert!(card.can_allow);

    // A change with a part within reach can be made without the rest.
    let request = change_request(&t, &run, browser_and_instructions("Two")).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.title, "Researcher wants to give Two more than it has");
    assert_eq!(card.lines, ["The browser"]);
    assert!(card.can_allow);
    assert!(card.can_without);
}

#[tokio::test]
async fn a_change_request_dies_with_its_dot() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let request = change_request(&t, &run, browser_and_instructions("One")).await;

    // The person deletes the dot.
    t.rt.delete_dot(&one.id).await.unwrap();
    // The maker was told it would be woken with an answer: it is woken once, to say there is
    // none.
    assert_eq!(wakes(&t).await, [change_went_message("One")]);
    let wake = &t.rt.store().waiting_making_wakes().await.unwrap()[0];
    assert_eq!(
        (wake.maker_dot_id.as_str(), wake.root_run_id.as_str()),
        (maker.id.as_str(), run.root_run_id.as_str())
    );

    let store = t.rt.store();
    assert!(matches!(
        store.get_make_request(&request.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(store
        .list_make_requests(None, false)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store.count_waiting_make_requests(&maker.id).await.unwrap(),
        0
    );
    let makers = t.rt.makers();
    assert!(matches!(
        makers.request_card(&request.id).await,
        Err(Error::NotFound(_))
    ));
    for decision in [
        MakeDecision::Allow,
        MakeDecision::Without,
        MakeDecision::Refuse,
    ] {
        assert!(matches!(
            makers.decide_request(&request.id, decision, None).await,
            Err(Error::NotFound(_))
        ));
    }
    // The answers that found nothing woke no one.
    assert_eq!(wakes(&t).await.len(), 1);
    assert_eq!(dot_names(&t).await, ["Researcher"]);
}

#[tokio::test]
async fn the_makers_reach_shrank_since_it_asked() {
    let t = start().await;
    add_github(&t).await;
    let mut s = maker_spec(&t, "Researcher");
    s.uses = uses_github();
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // The server is within the maker's reach when it asks: only the browser is an extra.
    let mut asked = browser_and_instructions("One");
    asked.wanted.uses = Some(uses_github());
    let request = change_request(&t, &run, asked).await;
    assert_eq!(request.extras, [Extra::Browser]);
    // The person takes the server from the maker.
    person_edits(&t, &maker, |spec| spec.uses = Uses::default()).await;

    // Allow would now give the dot the server too, which the card did not show.
    let why = "Researcher's own settings changed since it asked. Don't change it, and let it \
               ask again.";
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!card.can_allow);
    assert_eq!(card.blocked.as_deref(), Some(why));
    // The card shows what Allow would give now.
    assert_eq!(card.lines, ["The MCP server 'github'", "The browser"]);
    assert_eq!(card.request.status, RequestStatus::Waiting);
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Allow, None)
                .await
        ),
        why
    );
    // Nothing was marked and nothing changed: the request still waits for the person.
    assert_eq!(
        t.rt.store().get_make_request(&request.id).await.unwrap(),
        request
    );
    assert_eq!(stored(&t, &one).await, one);
    assert!(wakes(&t).await.is_empty());
    assert_eq!(
        kinds(&t, &run).await,
        [MakingEventKind::Made, MakingEventKind::Asked]
    );
    // What is within the maker's reach as it is now can still be changed: the instructions,
    // and neither the server nor the browser.
    assert!(card.can_without);
    let after = decide(&t, &request, MakeDecision::Without).await;
    assert_eq!(after.status, RequestStatus::MadeWithout);
    let dot = stored(&t, &one).await;
    let mut expected = one.spec.clone();
    expected.instructions = "List only the posts on Rust.".into();
    assert_eq!(dot.spec, expected);
}

// ---- delete_dot

#[tokio::test]
async fn a_maker_deletes_a_dot_it_made() {
    let t = start().await;
    let mut s = maker_spec(&t, "Researcher");
    s.making = Making {
        on: true,
        per_run: 2,
        total: 2,
    };
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    make(&t, &run, "Two").await;
    // It has a history, and a change that waits for the person.
    let its_run = live_run(&t, &one).await;
    end(&t, &its_run, RunStatus::Succeeded).await;
    let request = change_request(&t, &run, browser_and_instructions("One")).await;
    let mut rx = t.rt.subscribe();

    // Named in any letter case.
    let text = t.rt.makers().delete_dot(&run.id, " one ").await.unwrap();

    assert_eq!(text, "Deleted the dot \"One\" and its history.");
    let store = t.rt.store();
    assert!(matches!(
        store.get_dot(&one.id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(dot_names(&t).await, ["Researcher", "Two"]);
    // Its history went with it, as with the person's delete, and so did its request.
    assert!(matches!(
        store.get_run(&its_run.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(store.list_runs(Some(&one.id), 10).await.unwrap().is_empty());
    assert!(matches!(
        store.get_make_request(&request.id).await,
        Err(Error::NotFound(_))
    ));
    // The "deleted" card's entry keeps the name.
    let events = log(&t, &run).await;
    let event = events.last().unwrap();
    assert_eq!(event.kind, MakingEventKind::Deleted);
    assert_eq!(event.maker_dot_id, maker.id);
    assert_eq!(event.run_id, run.id);
    assert_eq!(event.root_run_id, run.root_run_id);
    assert_eq!(event.dot_id, None);
    assert_eq!(event.dot_name, "One");
    assert_eq!(event.detail, json!({}));
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    // It is no dot of the maker's any more.
    assert_eq!(
        invalid(t.rt.makers().delete_dot(&run.id, "One").await),
        not_made("One")
    );
    // A place in the total is free again; the run's own count is not.
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 1);
    assert_eq!(store.count_made_in_run(&run.id).await.unwrap(), 2);
    match t.rt.makers().make_dot(&run.id, ask("Three")).await.unwrap() {
        MakeOutcome::Waiting { request, .. } => {
            assert_eq!(request.over, Some(OverLimit::PerRun { nth: 3, limit: 2 }))
        }
        other => panic!("made: {other:?}"),
    }
    let next = live_run(&t, &maker).await;
    make(&t, &next, "Four").await;
    assert_eq!(dot_names(&t).await, ["Four", "Researcher", "Two"]);
}

#[tokio::test]
async fn a_busy_dot_is_not_deleted() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;

    // A run of it that waits in the queue.
    let its_run = queued_run(&t, &one).await;
    assert_eq!(
        conflict(t.rt.makers().delete_dot(&run.id, "One").await),
        BUSY
    );
    // The same run, running.
    assert!(t.rt.store().claim_run(&its_run.id).await.unwrap());
    assert_eq!(
        conflict(t.rt.makers().delete_dot(&run.id, "One").await),
        BUSY
    );
    // And waiting for an approval.
    t.rt.store()
        .set_status(&its_run.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    assert_eq!(
        conflict(t.rt.makers().delete_dot(&run.id, "One").await),
        BUSY
    );
    assert_eq!(stored(&t, &one).await, one);
    assert_eq!(kinds(&t, &run).await, [MakingEventKind::Made]);

    // Once it has ended the dot goes.
    t.rt.store()
        .set_status(&its_run.id, RunStatus::Running)
        .await
        .unwrap();
    end(&t, &its_run, RunStatus::Cancelled).await;
    t.rt.makers().delete_dot(&run.id, "One").await.unwrap();
    assert_eq!(dot_names(&t).await, ["Researcher"]);
}

#[tokio::test]
async fn a_dot_it_did_not_make_is_not_deleted() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let writer =
        t.rt.create_dot(maker_spec(&t, "Writer"), None, None)
            .await
            .unwrap();
    let writers_run = live_run(&t, &writer).await;
    make(&t, &writers_run, "Other").await;
    t.rt.create_dot(maker_spec(&t, "Mine"), None, None)
        .await
        .unwrap();

    for name in ["Other", "Mine", "Writer", "Researcher", "Nope"] {
        assert_eq!(
            invalid(t.rt.makers().delete_dot(&run.id, name).await),
            not_made(name)
        );
    }
    assert_eq!(
        dot_names(&t).await,
        ["Mine", "Other", "Researcher", "Writer"]
    );
    assert!(log(&t, &run).await.is_empty());
}

#[tokio::test]
async fn a_makers_made_dots_survive_their_makers_delete_by_the_grand_maker() {
    let t = start().await;
    let a = maker(&t).await;
    let a_run = live_run(&t, &a).await;
    let b = make(&t, &a_run, "B").await;
    // Only the person lets a made dot make dots.
    let b = person_edits(&t, &b, |spec| spec.making.on = true).await;
    let b_run = live_run(&t, &b).await;
    let c = make(&t, &b_run, "C").await;
    assert_eq!(
        c.made.as_ref().unwrap().by_dot_id.as_deref(),
        Some(b.id.as_str())
    );
    // C is B's, not A's.
    assert_eq!(
        invalid(t.rt.makers().delete_dot(&a_run.id, "C").await),
        not_made("C")
    );
    end(&t, &b_run, RunStatus::Succeeded).await;

    t.rt.makers().delete_dot(&a_run.id, "B").await.unwrap();

    // C stays, and still says who made it, by the name B had.
    assert_eq!(dot_names(&t).await, ["C", "Researcher"]);
    let made = stored(&t, &c).await.made.expect("a made dot");
    assert_eq!(made.by_dot_id, None);
    assert_eq!(made.by_name, "B");
    assert_eq!(made.run_id, b_run.id);
    assert_eq!(stored(&t, &c).await.spec, c.spec);
}

// ---- all three

/// What each of the three tools says to `run`: each must refuse it with an `Invalid`.
async fn refusals(t: &T, run: &Run) -> [String; 3] {
    let makers = t.rt.makers();
    [
        invalid(makers.list_made_dots(&run.id).await),
        invalid(makers.change_dot(&run.id, new_instructions("One")).await),
        invalid(makers.delete_dot(&run.id, "One").await),
    ]
}

#[tokio::test]
async fn a_maker_with_the_switch_off_can_do_none_of_the_three() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // The person turns the switch off while the run is going.
    person_edits(&t, &maker, |spec| spec.making.on = false).await;

    assert_eq!(refusals(&t, &run).await, [SWITCH_OFF; 3]);

    assert_eq!(stored(&t, &one).await, one);
    assert_eq!(kinds(&t, &run).await, [MakingEventKind::Made]);
    // A run that has ended can do none of them either, switch on or off.
    person_edits(&t, &maker, |spec| spec.making.on = true).await;
    end(&t, &run, RunStatus::Succeeded).await;
    assert_eq!(
        refusals(&t, &run).await,
        ["This run has ended, so it cannot use the tools for making dots."; 3]
    );
    assert_eq!(stored(&t, &one).await, one);
}

#[tokio::test]
async fn a_workflow_step_run_can_do_none_of_the_three() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
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
        refusals(&t, &run).await,
        ["A run that is a workflow step cannot use the tools for making dots."; 3]
    );

    assert_eq!(stored(&t, &one).await, one);
    assert_eq!(kinds(&t, &run).await, [MakingEventKind::Made]);
}

#[tokio::test]
async fn a_failed_or_cancelled_run_can_do_none_of_the_three() {
    let t = start().await;
    let maker = maker(&t).await;
    for status in [RunStatus::Failed, RunStatus::Cancelled] {
        let run = live_run(&t, &maker).await;
        make(&t, &run, &format!("{status:?}")).await;
        end(&t, &run, status).await;
        let makers = t.rt.makers();
        let ended = "This run has ended, so it cannot use the tools for making dots.";
        let name = format!("{status:?}");
        assert_eq!(invalid(makers.list_made_dots(&run.id).await), ended);
        assert_eq!(
            invalid(makers.change_dot(&run.id, new_instructions(&name)).await),
            ended
        );
        assert_eq!(invalid(makers.delete_dot(&run.id, &name).await), ended);
    }
    assert_eq!(dot_names(&t).await, ["Cancelled", "Failed", "Researcher"]);
}

// ---- fix 14b part 2: a dot that holds more than its maker, the card's lines, the delete

#[tokio::test]
async fn a_dot_holding_more_than_its_maker_asks_for_any_change_that_is_not_narrowing() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // The person gives the dot the browser, which its maker does not have.
    let edited = person_edits(&t, &one, |spec| spec.browser = true).await;

    // New instructions would steer what the person allowed: the person decides.
    let outcome =
        t.rt.makers()
            .change_dot(&run.id, new_instructions("One"))
            .await
            .unwrap();
    assert_eq!(
        outcome.tool_text(),
        "Not changed yet: \"One\" needs the person's say-so, because it has, or the change gives \
         it, more than you have (The browser). The request is waiting for them. Do not wait for \
         it and do not ask again: carry on, and you will be woken with their answer."
    );
    let ChangeOutcome::Waiting { request, held, .. } = outcome else {
        panic!("changed")
    };
    assert!(held);
    assert_eq!(request.extras, [Extra::Browser]);
    assert_eq!(stored(&t, &one).await, edited);
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        card.title,
        "Researcher wants to change One, which has more than Researcher has"
    );
    assert_eq!(card.lines, ["The browser"]);
    assert_eq!(card.asked, ["New instructions"]);
    assert_eq!(
        card.instructions.as_deref(),
        Some("List only the posts on Rust.")
    );
    assert!(card.can_allow);
    // There is no part of it to save without what the dot holds.
    assert!(!card.can_without);
    assert_eq!(
        invalid(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Without, None)
                .await
        ),
        "This dot has more than its maker has, so the change cannot be saved without that. \
         Allow it, or don't change it."
    );
    let after = decide(&t, &request, MakeDecision::Allow).await;
    assert_eq!(after.status, RequestStatus::Allowed);
    let dot = stored(&t, &one).await;
    assert_eq!(dot.spec.instructions, "List only the posts on Rust.");
    assert!(dot.spec.browser);

    // Taking things away is saved at once, the browser too; then nothing is held.
    let (_, what) = changed(
        &t,
        &run,
        change(
            "One",
            Wanted {
                max_turns: Some(10),
                schedule: Some(None),
                ..Wanted::default()
            },
        ),
    )
    .await;
    assert_eq!(what, ["turn limit"]);
    let (_, what) = changed(
        &t,
        &run,
        change(
            "One",
            Wanted {
                browser: Some(false),
                ..Wanted::default()
            },
        ),
    )
    .await;
    assert_eq!(what, ["browser"]);
    let mut asked = new_instructions("One");
    asked.wanted.instructions = Some("List only the posts on Go.".into());
    let (_, what) = changed(&t, &run, asked).await;
    assert_eq!(what, ["instructions"]);
}

#[tokio::test]
async fn a_new_purpose_or_folder_of_a_dot_holding_more_asks_too() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let one = make(&t, &run, "Two").await;
    person_edits(&t, &one, |spec| spec.max_turns = 80).await;
    let mut purpose = change("Two", Wanted::default());
    purpose.purpose = Some("Sends the posts out".into());
    let request = change_request(&t, &run, purpose).await;
    assert_eq!(
        request.extras,
        [Extra::MaxTurns {
            wanted: 80,
            makers: 40
        }]
    );
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.asked, ["Purpose: Sends the posts out"]);
    decide(&t, &request, MakeDecision::Refuse).await;
    let folder = change(
        "Two",
        Wanted {
            folder: Some("notes".into()),
            ..Wanted::default()
        },
    );
    let request = change_request(&t, &run, folder).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    let notes = dots_core::making::reach::real_path(&t.work()).join("notes");
    assert_eq!(card.asked, [format!("Folder: {}", notes.display())]);
}

#[tokio::test]
async fn a_change_card_lists_every_setting_the_change_names() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let mut asked = browser_and_instructions("One");
    asked.purpose = Some("Lists the posts on Rust".into());
    asked.wanted.name = Some("Feeds".into());
    asked.wanted.uses = Some(uses_github());
    asked.wanted.max_turns = Some(50);
    asked.wanted.schedule = Some(Some(
        serde_json::from_value(json!({"rule": {"kind": "every", "minutes": 120}})).unwrap(),
    ));
    let request = change_request(&t, &run, asked).await;

    let card = t.rt.makers().request_card(&request.id).await.unwrap();

    assert_eq!(
        card.lines,
        [
            "The MCP server 'github'",
            "The browser",
            "A turn limit of 50 (Researcher has 40)"
        ]
    );
    assert_eq!(
        card.asked,
        [
            "Name: Feeds",
            "Purpose: Lists the posts on Rust",
            "New instructions",
            "Adds the MCP server 'github'",
            "Browser: on",
            "Turn limit: 50",
            "Schedule: every 2 hours"
        ]
    );
    assert_eq!(
        card.instructions.as_deref(),
        Some("List only the posts on Rust.")
    );
    assert_eq!(card.schedule.as_deref(), Some("every 2 hours"));
    // As it crosses to the UI.
    let json = serde_json::to_value(&card).unwrap();
    assert_eq!(json["asked"][2], "New instructions");
    assert_eq!(json["instructions"], "List only the posts on Rust.");
    assert!(json["request"].get("make_ask").is_none());
}

#[tokio::test]
async fn deleting_a_dot_with_a_waiting_change_wakes_its_maker_once() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    change_request(&t, &run, browser_and_instructions("One")).await;

    t.rt.makers().delete_dot(&run.id, "One").await.unwrap();

    assert_eq!(wakes(&t).await, [change_went_message("One")]);
    assert_eq!(
        change_went_message("One"),
        "\"One\" was deleted, so the change you asked for it went with it: the person will not \
         answer it. Do not ask for it again."
    );
    // A dot with no change waiting wakes no one.
    make(&t, &run, "Two").await;
    t.rt.makers().delete_dot(&run.id, "Two").await.unwrap();
    assert_eq!(wakes(&t).await.len(), 1);
}

// ---- fix 14b part 2b: what a dot holds, and what the card says when it was given more

#[tokio::test]
async fn a_folder_in_the_new_dots_folder_that_was_not_made_for_the_dot_is_held() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // An old dot's folder, directly in the folder for new dots, that no dot works in now.
    let old = t.dir.path().join("Dots").join("old-dot");
    std::fs::create_dir_all(&old).unwrap();
    person_edits(&t, &one, |spec| {
        spec.workdir = old.to_string_lossy().into_owned()
    })
    .await;

    let request = change_request(&t, &run, new_instructions("One")).await;

    let path = dots_core::making::reach::real_path(&old)
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        request.extras,
        [Extra::Folder {
            path,
            below_maker: false
        }]
    );
    // The folder a dot was made with is its own, within reach: it changes at once.
    make(&t, &run, "Two").await;
    changed(&t, &run, new_instructions("Two")).await;
}

#[tokio::test]
async fn a_dot_given_more_since_the_change_was_asked_says_so() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let request = change_request(&t, &run, browser_and_instructions("One")).await;
    // The person gives the dot a turn limit above its maker's while the change waits.
    person_edits(&t, &one, |spec| spec.max_turns = 80).await;

    let why = "One was given more since Researcher asked, so it has more than its maker can \
               give. Don't change it, and let it ask again.";
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!card.can_allow);
    assert_eq!(card.blocked.as_deref(), Some(why));
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Allow, None)
                .await
        ),
        why
    );
    assert_eq!(
        stored(&t, &one).await.spec.instructions,
        one.spec.instructions
    );
}

#[tokio::test]
async fn without_tells_the_maker_what_was_left_out_of_the_change_as_it_stood_then() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let mut asked = browser_and_instructions("One");
    asked.wanted.uses = Some(uses_github());
    let request = change_request(&t, &run, asked).await;
    assert_eq!(request.extras.len(), 2, "{:?}", request.extras);
    // The person gives the maker the server: only the browser is beyond its reach now.
    let mut spec = maker.spec.clone();
    spec.uses = uses_github();
    t.rt.update_dot(&maker.id, spec).await.unwrap();

    decide(&t, &request, MakeDecision::Without).await;

    assert_eq!(
        wakes(&t).await,
        [
            "The person chose to change \"One\" without what was beyond your reach. Left out: \
             The browser."
        ]
    );
}

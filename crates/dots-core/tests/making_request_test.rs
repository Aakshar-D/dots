//! Make requests (dots-make-dots design sections 7.1, 7.2 and 10): a dot beyond its maker's
//! reach or over a limit is stored for the person and nothing is made; the person allows it,
//! makes it without the extras or refuses it; each answer wakes the maker; and a request is
//! checked again when it is shown and when it is answered.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, roster, sign, sign_roster, token, ACME, ACME_NAME,
    ISSUED, START, TOKEN,
};
use common::spec;
use dots_core::accounts::identity;
use dots_core::events::RuntimeEvent;
use dots_core::library::{LibraryConfig, McpServerConfig, Uses};
use dots_core::making::log::{MakingEvent, MakingEventKind};
use dots_core::making::manage::{ChangeDot, ChangeOutcome};
use dots_core::making::reach::{real_path, Extra, Wanted, WantedPermissions};
use dots_core::making::requests::{
    MakeDecision, MakeRequest, RequestKind, RequestStatus, MAX_WAITING_REQUESTS,
};
use dots_core::making::service::{MakeDot, MakeOutcome, OverLimit};
use dots_core::model::{Dot, DotSpec, Making, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::Preset;
use dots_core::schedule::Schedule;
use dots_core::vault::{EntryKind, Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
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

/// As `making_make_test.rs` has it: no queued run starts, so a maker's run is put in the
/// running state by hand, and a wake of its topic waits where a test can read it.
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
    let cfg = config(&dir);
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

/// The same with the browser, which no maker of these tests has: one extra.
fn ask_browser(name: &str) -> MakeDot {
    let mut asked = ask(name);
    asked.wanted.browser = Some(true);
    asked
}

/// Makes the dot, which must be within reach and within the limits.
async fn make(t: &T, run: &Run, name: &str) -> Dot {
    match t.rt.makers().make_dot(&run.id, ask(name)).await.unwrap() {
        MakeOutcome::Made { dot, .. } => dot,
        other => panic!("{name} was not made: {other:?}"),
    }
}

/// Asks for a dot that must come back waiting for the person: the outcome.
async fn asked(t: &T, run: &Run, ask: MakeDot) -> MakeOutcome {
    let outcome = t.rt.makers().make_dot(&run.id, ask).await.unwrap();
    assert!(
        matches!(outcome, MakeOutcome::Waiting { .. }),
        "made: {outcome:?}"
    );
    outcome
}

/// Asks for a dot that must come back waiting for the person: its request.
async fn request(t: &T, run: &Run, ask: MakeDot) -> MakeRequest {
    match asked(t, run, ask).await {
        MakeOutcome::Waiting { request, .. } => request,
        other => panic!("made: {other:?}"),
    }
}

/// The person's answer to the card they see, which must be taken: it sends back the card's
/// limit line and what it showed beyond reach, as the UI does.
async fn decide(t: &T, request: &MakeRequest, decision: MakeDecision) -> MakeRequest {
    let makers = t.rt.makers();
    let card = makers.request_card(&request.id).await.unwrap();
    makers
        .decide_request(
            &request.id,
            decision,
            card.limit.as_deref(),
            card.shown.as_deref(),
        )
        .await
        .unwrap()
}

/// The request as it is stored now.
async fn stored(t: &T, request: &MakeRequest) -> MakeRequest {
    t.rt.store().get_make_request(&request.id).await.unwrap()
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
/// variable in the Personal vault. Returns the item's id.
async fn add_github(t: &T) -> String {
    let config = LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: [("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    });
    let item =
        t.rt.store()
            .add_library_item("github", &config, None)
            .await
            .unwrap();
    t.rt.vault()
        .add("GITHUB_TOKEN", EntryKind::Plain, "t0ken")
        .await
        .unwrap();
    item.id
}

fn uses_github() -> Uses {
    Uses {
        mcp_servers: vec!["github".into()],
        ..Uses::default()
    }
}

fn github_token() -> Grants {
    Grants([("mcp:github".to_string(), vec!["GITHUB_TOKEN".to_string()])].into())
}

/// Every `minutes`, as a tool call carries it.
fn every(minutes: u32) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": minutes},
        "catch_up": true,
        "until": null
    }))
    .unwrap()
}

const WAITING_TAIL: &str = "The request is waiting for them. Do not wait for it and do not ask \
                            again: carry on with other work or end your turn, and you will be \
                            woken with their answer.";
const REFUSED: &str = "The person chose not to make \"Blog Watcher\". Do not ask again for the \
                       same thing; carry on without it, or tell the person what you cannot do.";

#[tokio::test]
async fn an_extra_stores_a_request_and_makes_nothing() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut rx = t.rt.subscribe();
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.folder = Some("notes/blogs".into());
    wanted.first_job = Some(" Read them now. ".into());
    let outcome = asked(&t, &run, wanted).await;
    assert_eq!(
        outcome.tool_text(),
        format!(
            "Not made yet: \"Blog Watcher\" needs the person's say-so, because it asks for more \
             than you have (The browser). {WAITING_TAIL}"
        )
    );
    let MakeOutcome::Waiting {
        request,
        maker_name,
    } = outcome
    else {
        unreachable!()
    };
    assert_eq!(maker_name, "Researcher");
    assert_eq!(request.maker_dot_id, maker.id);
    assert_eq!(request.run_id, run.id);
    assert_eq!(request.root_run_id, run.root_run_id);
    assert_eq!(request.kind, RequestKind::Make);
    assert_eq!(request.dot_id, None);
    assert_eq!(request.name, "Blog Watcher");
    assert_eq!(request.purpose, "Reads the feeds");
    // The dot as asked and the dot without the extra, for the person's answer.
    assert_eq!(request.spec.name, "Blog Watcher");
    assert!(request.spec.browser);
    assert!(!request.without.browser);
    assert_eq!(request.extras, [Extra::Browser]);
    assert_eq!(request.over, None);
    // The folder is within reach and still to be made: both dots would work in it.
    let folder = real_path(&t.work()).join("notes").join("blogs");
    assert_eq!(
        request.create_folder.as_deref().map(PathBuf::from),
        Some(folder.clone())
    );
    assert_eq!(PathBuf::from(&request.spec.workdir), folder);
    assert_eq!(request.without.workdir, request.spec.workdir);
    assert_eq!(request.first_job.as_deref(), Some("Read them now."));
    assert_eq!(request.status, RequestStatus::Waiting);
    assert_eq!(request.gone_reason, None);
    assert_eq!(request.decided_at, None);
    assert_eq!(request.made_dot_id, None);
    // It is stored as it was returned.
    assert_eq!(stored(&t, &request).await, request);
    let store = t.rt.store();
    assert_eq!(
        store
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap(),
        std::slice::from_ref(&request)
    );
    assert_eq!(
        store.list_make_requests(None, false).await.unwrap(),
        std::slice::from_ref(&request)
    );
    // The ask card's entry is in the topic's log.
    let events = log(&t, &run).await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.kind, MakingEventKind::Asked);
    assert_eq!(event.maker_dot_id, maker.id);
    assert_eq!(event.run_id, run.id);
    assert_eq!(event.root_run_id, run.root_run_id);
    assert_eq!(event.dot_id, None);
    assert_eq!(event.dot_name, "Blog Watcher");
    assert_eq!(event.detail, json!({"request_id": request.id}));
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    // No dot, no folder, no wake, and nothing counted toward the limits.
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(!t.work().join("notes").exists());
    assert!(folders(&t.root).is_empty());
    assert!(wakes(&t).await.is_empty());
    assert_eq!(store.count_made_in_run(&run.id).await.unwrap(), 0);
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 0);
}

#[tokio::test]
async fn a_limit_alone_stores_a_request_that_only_allow_or_refuse_can_answer() {
    let t = start().await;
    let maker = maker_with(&t, 3, 10).await;
    let run = live_run(&t, &maker).await;
    for name in ["One", "Two", "Three"] {
        make(&t, &run, name).await;
    }
    let outcome = asked(&t, &run, ask("Four")).await;
    assert_eq!(
        outcome.tool_text(),
        format!(
            "Not made yet: \"Four\" needs the person's say-so, because it would be the 4th dot \
             in this run; the limit is 3. {WAITING_TAIL}"
        )
    );
    let MakeOutcome::Waiting { request, .. } = outcome else {
        unreachable!()
    };
    assert!(request.extras.is_empty());
    assert_eq!(request.over, Some(OverLimit::PerRun { nth: 4, limit: 3 }));
    assert_eq!(request.spec, request.without);

    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.title, "Researcher wants to make one more dot");
    assert_eq!(card.lines, ["4th dot in this run; the limit is 3"]);
    assert!(card.can_allow);
    assert!(!card.can_without);
    assert_eq!(card.blocked, None);
    assert_eq!(card.request, request);

    // There is nothing to make it without.
    let err =
        t.rt.makers()
            .decide_request(&request.id, MakeDecision::Without, None, None)
            .await
            .unwrap_err();
    match err {
        Error::Invalid(text) => assert_eq!(
            text,
            "Nothing of this request is beyond reach, so it cannot be made without anything. \
             Allow it, or don't make it."
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(stored(&t, &request).await.status, RequestStatus::Waiting);
    assert!(wakes(&t).await.is_empty());

    // Allow covers this one dot, past the limit (design section 10).
    let after = decide(&t, &request, MakeDecision::Allow).await;
    assert_eq!(after.status, RequestStatus::Allowed);
    assert_eq!(
        dot_names(&t).await,
        ["Four", "One", "Researcher", "Three", "Two"]
    );
    assert_eq!(
        wakes(&t).await,
        ["The person allowed it: the dot \"Four\" was made as you asked."]
    );
}

#[tokio::test]
async fn allow_makes_the_dot_as_asked_and_the_extras_are_that_dots_alone() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.uses = Some(uses_github());
    wanted.wanted.variables = Some(github_token());
    wanted.first_job = Some("Read them now.".into());
    let request = request(&t, &run, wanted).await;
    assert_eq!(request.extras.len(), 3, "{:?}", request.extras);
    // The maker's run may have ended by the time the person answers.
    t.rt.store()
        .set_session(&run.id, "session-1")
        .await
        .unwrap();
    t.rt.store()
        .finish_run(&run.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    let mut rx = t.rt.subscribe();

    let after = decide(&t, &request, MakeDecision::Allow).await;

    assert_eq!(after.status, RequestStatus::Allowed);
    assert!(after.decided_at.is_some());
    let dot_id = after.made_dot_id.clone().expect("the made dot");
    assert_eq!(stored(&t, &request).await, after);
    // The dot holds what was asked, extras included.
    let dot = t.rt.store().get_dot(&dot_id).await.unwrap();
    assert_eq!(dot.spec.name, "Blog Watcher");
    assert_eq!(dot.spec.uses, uses_github());
    assert_eq!(dot.spec.variables, github_token());
    assert!(dot.spec.browser);
    assert!(dot.spec.enabled);
    assert!(!dot.spec.making.on);
    // Made by the maker, in the run that asked.
    let made = dot.made.clone().expect("a made dot");
    assert_eq!(made.by_dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(made.run_id, run.id);
    assert_eq!(made.purpose, "Reads the feeds");
    // The maker's own settings do not change: its reach does not grow.
    assert_eq!(
        t.rt.store().get_dot(&maker.id).await.unwrap().spec,
        maker.spec
    );
    // The log: the ask, the dot, the answer.
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Asked,
            MakingEventKind::Made,
            MakingEventKind::JobStarted,
            MakingEventKind::Answered
        ]
    );
    assert_eq!(events[1].dot_id.as_deref(), Some(dot_id.as_str()));
    assert_eq!(events[1].run_id, run.id);
    let answered = &events[3];
    assert_eq!(answered.maker_dot_id, maker.id);
    assert_eq!(answered.run_id, run.id);
    assert_eq!(answered.root_run_id, run.root_run_id);
    assert_eq!(answered.dot_id.as_deref(), Some(dot_id.as_str()));
    assert_eq!(answered.dot_name, "Blog Watcher");
    assert_eq!(
        answered.detail,
        json!({"request_id": request.id, "decision": "allow", "dot_name": "Blog Watcher"})
    );
    assert!(updated(&mut rx).contains(&(maker.id.clone(), run.root_run_id.clone())));
    // The maker is woken once, in its topic, which was free: the wake's run is queued.
    assert!(wakes(&t).await.is_empty());
    let thread = t.rt.store().list_thread(&run.root_run_id).await.unwrap();
    assert_eq!(thread.len(), 2, "{thread:#?}");
    assert_eq!(
        thread[1].payload.as_ref().unwrap()["message"],
        "The person allowed it: the dot \"Blog Watcher\" was made as you asked. Its first job \
         has started; you will be woken with the result."
    );
}

#[tokio::test]
async fn without_makes_the_dot_minus_the_extras() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let outside = t.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let outside = real_path(&outside).to_string_lossy().into_owned();
    // One of every kind of extra.
    let mut wanted = ask("Blog Watcher");
    wanted.wanted.uses = Some(uses_github());
    wanted.wanted.variables = Some(github_token());
    wanted.wanted.permissions = Some(WantedPermissions::Preset(Preset::Trusted));
    wanted.wanted.folder = Some(outside.clone());
    wanted.wanted.browser = Some(true);
    wanted.wanted.use_user_settings = Some(true);
    wanted.wanted.max_turns = Some(50);
    wanted.wanted.timeout_secs = Some(3600);
    let request = request(&t, &run, wanted).await;
    let lines = [
        "The MCP server 'github'".to_string(),
        "The variable GITHUB_TOKEN for the server 'github'".to_string(),
        "The preset Trusted (Researcher has Sandboxed)".to_string(),
        format!("The folder {outside}, outside Researcher's folder"),
        "The browser".to_string(),
        "Your Claude settings".to_string(),
        "A turn limit of 50 (Researcher has 40)".to_string(),
        "A time limit of 1 h (Researcher has 30 min)".to_string(),
    ];
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.lines, lines);
    assert!(card.can_allow);
    assert!(card.can_without);

    let after = decide(&t, &request, MakeDecision::Without).await;

    assert_eq!(after.status, RequestStatus::MadeWithout);
    assert!(after.decided_at.is_some());
    let dot_id = after.made_dot_id.clone().expect("the made dot");
    let dot = t.rt.store().get_dot(&dot_id).await.unwrap();
    // Items and variables dropped, the preset and the limits the maker's, a new folder of its
    // own, no browser and no user settings.
    assert_eq!(dot.spec.name, "Blog Watcher");
    assert_eq!(dot.spec.uses, Uses::default());
    assert_eq!(dot.spec.variables, Grants::default());
    assert_eq!(dot.spec.policy, maker.spec.policy);
    assert_eq!(folders(&t.root), ["blog-watcher"]);
    assert!(Path::new(&dot.spec.workdir).starts_with(&t.root));
    assert!(!dot.spec.browser);
    assert!(!dot.spec.use_user_settings);
    assert_eq!(dot.spec.max_turns, 40);
    assert_eq!(dot.spec.timeout_secs, 1800);
    assert_eq!(dot.made.as_ref().unwrap().run_id, run.id);
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [
            MakingEventKind::Asked,
            MakingEventKind::Made,
            MakingEventKind::Answered
        ]
    );
    assert_eq!(events[2].detail["decision"], "without");
    assert_eq!(
        wakes(&t).await,
        [format!(
            "The person chose to make \"Blog Watcher\" without what was beyond your reach. It \
             was made without: {}.",
            lines.join("; ")
        )]
    );
}

#[tokio::test]
async fn without_keeps_a_folder_that_was_within_reach() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.folder = Some("notes/blogs".into());
    let request = request(&t, &run, wanted).await;
    let after = decide(&t, &request, MakeDecision::Without).await;
    let dot =
        t.rt.store()
            .get_dot(after.made_dot_id.as_deref().unwrap())
            .await
            .unwrap();
    // The subfolder was no extra, so it is made and the dot works in it.
    let folder = real_path(&t.work()).join("notes").join("blogs");
    assert!(folder.is_dir());
    assert_eq!(PathBuf::from(&dot.spec.workdir), folder);
    assert!(folders(&t.root).is_empty());
    assert!(!dot.spec.browser);
}

#[tokio::test]
async fn refuse_makes_nothing_and_wakes_the_maker() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask_browser("Blog Watcher");
    wanted.first_job = Some("Read them now.".into());
    let request = request(&t, &run, wanted).await;

    let after = decide(&t, &request, MakeDecision::Refuse).await;

    assert_eq!(after.status, RequestStatus::Refused);
    assert!(after.decided_at.is_some());
    assert_eq!(after.made_dot_id, None);
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(folders(&t.root).is_empty());
    let events = log(&t, &run).await;
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        [MakingEventKind::Asked, MakingEventKind::Answered]
    );
    assert_eq!(events[1].dot_id, None);
    assert_eq!(
        events[1].detail,
        json!({"request_id": request.id, "decision": "refuse", "dot_name": "Blog Watcher"})
    );
    // No dot was made, so nothing is said of the first job.
    assert_eq!(wakes(&t).await, [REFUSED]);
    let wake = &t.rt.store().waiting_making_wakes().await.unwrap()[0];
    assert_eq!(wake.maker_dot_id, maker.id);
    assert_eq!(wake.root_run_id, run.root_run_id);
}

#[tokio::test]
async fn a_fourth_waiting_request_is_refused() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    assert_eq!(MAX_WAITING_REQUESTS, 3);
    for name in ["One", "Two", "Three"] {
        request(&t, &run, ask_browser(name)).await;
    }
    let err =
        t.rt.makers()
            .make_dot(&run.id, ask_browser("Four"))
            .await
            .unwrap_err();
    match err {
        Error::Invalid(text) => assert_eq!(
            text,
            "Researcher already has 3 requests waiting for the person. Wait to be woken with an \
             answer before asking for more."
        ),
        other => panic!("{other:?}"),
    }
    let store = t.rt.store();
    assert_eq!(
        store.count_waiting_make_requests(&maker.id).await.unwrap(),
        3
    );
    let names: Vec<String> = store
        .list_make_requests(Some(&maker.id), true)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(names, ["One", "Two", "Three"]);
    assert_eq!(kinds(&t, &run).await.len(), 3);
    // A dot within reach is still made: only asking is held back.
    make(&t, &run, "Plain").await;
    // Another maker's requests are its own.
    let mut other = maker_spec(&t);
    other.name = "Writer".into();
    let other = t.rt.create_dot(other, None, None).await.unwrap();
    let other_run = live_run(&t, &other).await;
    request(&t, &other_run, ask_browser("Four")).await;
    assert_eq!(store.list_make_requests(None, true).await.unwrap().len(), 4);
}

#[tokio::test]
async fn an_answered_request_frees_a_place() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = request(&t, &run, ask_browser("One")).await;
    request(&t, &run, ask_browser("Two")).await;
    request(&t, &run, ask_browser("Three")).await;
    decide(&t, &one, MakeDecision::Refuse).await;
    request(&t, &run, ask_browser("Four")).await;
    let store = t.rt.store();
    assert_eq!(
        store.count_waiting_make_requests(&maker.id).await.unwrap(),
        3
    );
    // The answered one is kept, and is no longer among those that wait.
    assert_eq!(
        store
            .list_make_requests(Some(&maker.id), false)
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        store
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn a_request_cannot_be_answered_twice() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let allowed = request(&t, &run, ask_browser("Blog Watcher")).await;
    decide(&t, &allowed, MakeDecision::Allow).await;
    let refused = request(&t, &run, ask_browser("Feed Reader")).await;
    decide(&t, &refused, MakeDecision::Refuse).await;
    for request in [&allowed, &refused] {
        for decision in [
            MakeDecision::Allow,
            MakeDecision::Without,
            MakeDecision::Refuse,
        ] {
            assert_eq!(
                conflict(
                    t.rt.makers()
                        .decide_request(&request.id, decision, None, None)
                        .await
                ),
                "That request was already answered."
            );
        }
    }
    assert_eq!(stored(&t, &allowed).await.status, RequestStatus::Allowed);
    assert_eq!(stored(&t, &refused).await.status, RequestStatus::Refused);
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "Researcher"]);
    assert_eq!(wakes(&t).await.len(), 2);
    // An answered request's card offers nothing.
    let card = t.rt.makers().request_card(&allowed.id).await.unwrap();
    assert!(!card.can_allow);
    assert!(!card.can_without);
    assert_eq!(card.blocked, None);
    assert_eq!(card.request.status, RequestStatus::Allowed);
    // A request that does not exist.
    let err =
        t.rt.makers()
            .decide_request("no-such-request", MakeDecision::Allow, None, None)
            .await
            .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn two_answers_at_once_make_one_dot() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    let makers = t.rt.makers();
    let shown = makers.request_card(&request.id).await.unwrap().shown;
    let (a, b) = tokio::join!(
        makers.decide_request(&request.id, MakeDecision::Allow, None, shown.as_deref()),
        makers.decide_request(&request.id, MakeDecision::Allow, None, shown.as_deref())
    );
    let (taken, late) = match (a, b) {
        (Ok(taken), late @ Err(_)) | (late @ Err(_), Ok(taken)) => (taken, late),
        other => panic!("expected one answer to be taken: {other:?}"),
    };
    assert_eq!(taken.status, RequestStatus::Allowed);
    assert_eq!(conflict(late), "That request was already answered.");
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "Researcher"]);
    assert_eq!(
        kinds(&t, &run).await,
        [
            MakingEventKind::Asked,
            MakingEventKind::Made,
            MakingEventKind::Answered
        ]
    );
    assert_eq!(wakes(&t).await.len(), 1);
}

#[tokio::test]
async fn a_library_item_gone_since_leaves_only_dont_make_it() {
    let t = start().await;
    let github = add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask("Blog Watcher");
    wanted.wanted.uses = Some(uses_github());
    let request = request(&t, &run, wanted).await;
    // While it is there the card offers every answer.
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(card.can_allow && card.can_without);
    assert_eq!(card.request.status, RequestStatus::Waiting);

    t.rt.store().remove_library_item(&github).await.unwrap();
    let mut rx = t.rt.subscribe();
    let gone = "The MCP server 'github' is no longer in the Library.";
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!card.can_allow);
    assert!(!card.can_without);
    assert_eq!(card.blocked.as_deref(), Some(gone));
    assert_eq!(card.request.status, RequestStatus::Gone);
    assert_eq!(card.request.gone_reason.as_deref(), Some(gone));
    assert_eq!(stored(&t, &request).await.status, RequestStatus::Gone);
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    // It still stands before the person, so it still takes one of the maker's places.
    assert_eq!(
        t.rt.store()
            .count_waiting_make_requests(&maker.id)
            .await
            .unwrap(),
        1
    );
    for decision in [MakeDecision::Allow, MakeDecision::Without] {
        assert_eq!(
            conflict(
                t.rt.makers()
                    .decide_request(&request.id, decision, None, None)
                    .await
            ),
            format!("{gone} Only \"Don't make it\" is left.")
        );
    }
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(wakes(&t).await.is_empty());

    let after = decide(&t, &request, MakeDecision::Refuse).await;
    assert_eq!(after.status, RequestStatus::Refused);
    assert_eq!(wakes(&t).await, [REFUSED]);
    assert_eq!(
        t.rt.store()
            .count_waiting_make_requests(&maker.id)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_variable_gone_since_is_found_when_the_person_answers() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask("Blog Watcher");
    wanted.wanted.variables = Some(Grants(
        [("shell".to_string(), vec!["GITHUB_TOKEN".to_string()])].into(),
    ));
    let request = request(&t, &run, wanted).await;
    t.rt.vault().remove("GITHUB_TOKEN").await.unwrap();
    // No card was read in between: the answer itself finds it.
    let gone = "The variable GITHUB_TOKEN no longer exists.";
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Allow, None, None)
                .await
        ),
        format!("{gone} Only \"Don't make it\" is left.")
    );
    let now = stored(&t, &request).await;
    assert_eq!(now.status, RequestStatus::Gone);
    assert_eq!(now.gone_reason.as_deref(), Some(gone));
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(wakes(&t).await.is_empty());
}

#[tokio::test]
async fn a_name_taken_since_leaves_only_dont_make_it() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    // The person makes a dot of that name, in another letter case.
    let mut theirs = spec("blog watcher");
    theirs.workdir = t.work().to_string_lossy().to_string();
    theirs.workspace_mode = WorkspaceMode::Folder;
    t.rt.create_dot(theirs, None, None).await.unwrap();

    let gone = "A dot named \"Blog Watcher\" exists now.";
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!card.can_allow);
    assert!(!card.can_without);
    assert_eq!(card.blocked.as_deref(), Some(gone));
    assert_eq!(card.request.status, RequestStatus::Gone);
    for decision in [MakeDecision::Allow, MakeDecision::Without] {
        assert_eq!(
            conflict(
                t.rt.makers()
                    .decide_request(&request.id, decision, None, None)
                    .await
            ),
            format!("{gone} Only \"Don't make it\" is left.")
        );
    }
    assert_eq!(dot_names(&t).await, ["Researcher", "blog watcher"]);
    let after = decide(&t, &request, MakeDecision::Refuse).await;
    assert_eq!(after.status, RequestStatus::Refused);
    assert_eq!(wakes(&t).await, [REFUSED]);
}

#[tokio::test]
async fn the_card_says_what_is_asked_in_plain_words() {
    let t = start().await;
    let maker = maker_with(&t, 1, 10).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.max_turns = Some(50);
    let outcome = asked(&t, &run, wanted).await;
    assert_eq!(
        outcome.tool_text(),
        format!(
            "Not made yet: \"Blog Watcher\" needs the person's say-so, because it asks for more \
             than you have (The browser; A turn limit of 50 (Researcher has 40)) and it would \
             be the 2nd dot in this run; the limit is 1. {WAITING_TAIL}"
        )
    );
    let MakeOutcome::Waiting { request, .. } = outcome else {
        unreachable!()
    };
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.maker_name, "Researcher");
    assert_eq!(
        card.title,
        "Researcher wants to make a dot with more than it has"
    );
    assert_eq!(
        card.lines,
        [
            "The browser",
            "A turn limit of 50 (Researcher has 40)",
            "2nd dot in this run; the limit is 1"
        ]
    );
    assert!(card.can_allow);
    assert!(card.can_without);
    assert_eq!(card.blocked, None);
    assert_eq!(card.request.name, "Blog Watcher");
    assert_eq!(card.request.purpose, "Reads the feeds");
    // Reading the card changes nothing.
    assert_eq!(stored(&t, &request).await, request);
    // As it crosses to the UI.
    let json = serde_json::to_value(&card).unwrap();
    assert_eq!(json["request"]["status"], "waiting");
    assert_eq!(json["request"]["kind"], "make");
    assert_eq!(json["request"]["extras"][0], json!({"kind": "browser"}));
    assert_eq!(
        json["request"]["over"],
        json!({"limit": "per_run", "nth": 2, "max": 1})
    );
    assert_eq!(
        serde_json::to_value(MakeDecision::Without).unwrap(),
        "without"
    );
    assert_eq!(
        serde_json::to_value(RequestStatus::MadeWithout).unwrap(),
        "made_without"
    );
}

#[tokio::test]
async fn a_made_dot_says_so_to_the_tool() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let outcome =
        t.rt.makers()
            .make_dot(&run.id, ask("Blog Watcher"))
            .await
            .unwrap();
    assert_eq!(
        outcome.tool_text(),
        "Made the dot \"Blog Watcher\". It is on and ready; give it a job with give_job."
    );
    let mut wanted = ask("Feed Reader");
    wanted.wanted.schedule = Some(Some(every(60)));
    let outcome = t.rt.makers().make_dot(&run.id, wanted).await.unwrap();
    assert_eq!(
        outcome.tool_text(),
        "Made the dot \"Feed Reader\". It is on and ready; give it a job with give_job. It will \
         also run on its schedule."
    );
}

#[tokio::test]
async fn an_answer_is_honoured_after_the_switch_is_turned_off() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    let mut off = maker.spec.clone();
    off.making.on = false;
    t.rt.update_dot(&maker.id, off).await.unwrap();
    let after = decide(&t, &request, MakeDecision::Allow).await;
    assert_eq!(after.status, RequestStatus::Allowed);
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "Researcher"]);
}

#[tokio::test]
async fn deleting_the_maker_discards_its_requests() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    t.rt.store()
        .finish_run(&run.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    t.rt.delete_dot(&maker.id).await.unwrap();
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
    // The topic's log reads as empty, and the card and an answer find nothing.
    assert!(store
        .list_making_events(&run.root_run_id)
        .await
        .unwrap()
        .is_empty());
    let makers = t.rt.makers();
    assert!(matches!(
        makers.request_card(&request.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        makers
            .decide_request(&request.id, MakeDecision::Allow, None, None)
            .await,
        Err(Error::NotFound(_))
    ));
    assert!(dot_names(&t).await.is_empty());
}

#[tokio::test]
async fn an_allowed_dot_counts_toward_the_asking_runs_count_and_the_total() {
    let t = start().await;
    let maker = maker_with(&t, 1, 2).await;
    let first = live_run(&t, &maker).await;
    make(&t, &first, "One").await;
    let two = request(&t, &first, ask("Two")).await;
    assert_eq!(two.over, Some(OverLimit::PerRun { nth: 2, limit: 1 }));
    // A request that waits counts toward neither limit (design section 10).
    let store = t.rt.store();
    assert_eq!(store.count_made_in_run(&first.id).await.unwrap(), 1);
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 1);

    decide(&t, &two, MakeDecision::Allow).await;

    // Allowed, it counts toward the run that asked and toward the total.
    assert_eq!(store.count_made_in_run(&first.id).await.unwrap(), 2);
    assert_eq!(store.count_made_dots(&maker.id).await.unwrap(), 2);
    let three = request(&t, &first, ask("Three")).await;
    assert_eq!(three.over, Some(OverLimit::PerRun { nth: 3, limit: 1 }));
    // A new run has its own count, and meets the total.
    let second = live_run(&t, &maker).await;
    let four = request(&t, &second, ask("Four")).await;
    assert_eq!(four.over, Some(OverLimit::Total { nth: 3, limit: 2 }));
}

/// A runtime joined to Acme, whose signed member list the fake OpenBao serves, as
/// `dot_admin_test.rs` sets it up, with no run starting.
struct Acme {
    t: T,
    bao: FakeOpenBao,
    keys: minisign::KeyPair,
}

impl Acme {
    async fn start() -> Acme {
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config(&dir);
        cfg.clock = FakeClock::new(at(START));
        let root = cfg.new_dot_root.clone();
        let rt = Runtime::start(cfg).await.unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        Acme {
            t: T { dir, root, rt },
            bao,
            keys: keys(),
        }
    }

    /// Serves a member list at `seq` with an owner who is not this PC, and this PC as `role`.
    async fn serve_list(&self, seq: u64, role: &str) {
        self.serve_list_with(seq, role, None).await;
    }

    /// `serve_list`, with `guardrails` on the Default workspace (guardrails spec section 3).
    async fn serve_list_with(
        &self,
        seq: u64,
        role: &str,
        guardrails: Option<dots_core::accounts::Guardrails>,
    ) {
        let me = identity(self.t.rt.store()).await.unwrap().public_key;
        let member = |id: &str, email: &str, role: &str, key: &str| {
            json!({
                "id": id, "name": email, "email": email, "role": role, "keys": [key],
                "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
            })
        };
        let members = json!([
            member("01OWNER", "owner@acme.com", "owner", &public_key(&keys())),
            member("01ME", "me@acme.com", role, &me),
        ]);
        let extra = json!({"v": 2, "roster": "required", "admins": [], "offline_hours": 0});
        let signed = sign(&self.keys, &policy(ACME, ISSUED, extra));
        self.bao.put_env("secret", "dots", json!({}));
        self.bao.put_policy("secret", "dots", &signed);
        let mut list: serde_json::Value =
            serde_json::from_str(&roster(ACME, seq, members)).unwrap();
        if let Some(g) = guardrails {
            list["workspaces"][0]["guardrails"] = serde_json::to_value(g).unwrap();
        }
        let list = sign_roster(&self.keys, &list.to_string());
        self.bao.put_roster("secret", "dots", &list);
    }

    /// Joins Acme with this PC on its member list as `role`.
    async fn join_as(&self, role: &str) {
        self.serve_list(1, role).await;
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        let orgs = self.t.rt.vault().orgs().expect("the runtime's orgs");
        orgs.join(&code, Some(token())).await.unwrap();
    }

    /// Serves the list again at `seq` with this PC as `role`, and fetches it.
    async fn become_(&self, seq: u64, role: &str) {
        self.serve_list(seq, role).await;
        self.t.rt.refresh_org(ACME).await.unwrap();
    }

    /// Serves the list again at `seq` with this PC as a Member of the Default workspace, which
    /// now has the default guardrails (a Member's dot may use up to Sandboxed), and fetches it.
    async fn guard(&self, seq: u64) {
        let rails = dots_core::accounts::Guardrails::defaults();
        self.serve_list_with(seq, "member", Some(rails)).await;
        self.t.rt.refresh_org(ACME).await.unwrap();
    }
}

/// The answer carries what the card showed beyond the maker's reach (`MakeRequestCard::shown`),
/// as it carries the limit line: when the maker's settings moved twice between the card and the
/// answer, so that Allow would now give an extra the card the person looked at did not show,
/// Allow and Make it without these are refused, and the card read again shows it.
#[tokio::test]
async fn allow_covers_only_the_extras_the_card_showed() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut both = ask_browser("Blog Watcher");
    both.wanted.use_user_settings = Some(true);
    let request = request(&t, &run, both).await;
    assert_eq!(request.extras, [Extra::Browser, Extra::UserSettings]);
    // The person gives the maker the browser: the card shows only the Claude settings.
    let mut s = t.rt.store().get_dot(&maker.id).await.unwrap().spec;
    s.browser = true;
    t.rt.update_dot(&maker.id, s.clone()).await.unwrap();
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.lines, ["Your Claude settings"]);
    assert!(card.can_allow);
    // Then takes the browser away again, while that card is still open.
    s.browser = false;
    t.rt.update_dot(&maker.id, s).await.unwrap();

    let mut rx = t.rt.subscribe();
    for decision in [MakeDecision::Allow, MakeDecision::Without] {
        let answer = t.rt.makers().decide_request(
            &request.id,
            decision,
            card.limit.as_deref(),
            card.shown.as_deref(),
        );
        assert_eq!(
            conflict(answer.await),
            dots_core::making::requests::CARD_CHANGED
        );
    }
    assert!(!updated(&mut rx).is_empty());
    assert_eq!(stored(&t, &request).await.status, RequestStatus::Waiting);
    assert_eq!(dot_names(&t).await, ["Researcher"]);

    // The card read again shows both, and its Allow gives both.
    let again = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(again.lines, ["The browser", "Your Claude settings"]);
    assert_ne!(again.shown, card.shown);
    let after = decide(&t, &request, MakeDecision::Allow).await;
    assert_eq!(after.status, RequestStatus::Allowed);
    let made = t.rt.store().get_dot(after.made_dot_id.as_deref().unwrap());
    let made = made.await.unwrap();
    assert!(made.spec.browser && made.spec.use_user_settings);
}

/// The card of a request whose dot the workspace's guardrails ceiling would refuse shows Allow
/// off, with the ceiling's own words (dots-make-dots design section 7.2, guardrails spec section
/// 4.3): pressing it could only fail. Make it without these, which keeps the maker's own
/// permissions, stays on. The same for a change.
#[tokio::test]
async fn the_guardrails_ceiling_turns_allow_off_on_the_card() {
    let a = Acme::start().await;
    let t = &a.t;
    a.join_as("member").await;
    let maker =
        t.rt.create_dot(maker_spec(t), Some(ACME.into()), None)
            .await
            .unwrap();
    let run = live_run(t, &maker).await;
    let made = make(t, &run, "Feed Reader").await;
    let mut trusted = ask_browser("Blog Watcher");
    trusted.wanted.permissions = Some(WantedPermissions::Preset(Preset::Trusted));
    let request = request(t, &run, trusted).await;
    let change = ChangeDot {
        dot: "Feed Reader".into(),
        purpose: None,
        wanted: Wanted {
            permissions: Some(WantedPermissions::Preset(Preset::Trusted)),
            ..Wanted::default()
        },
    };
    let change = match t.rt.makers().change_dot(&run.id, change).await.unwrap() {
        ChangeOutcome::Waiting { request, .. } => request,
        other => panic!("changed: {other:?}"),
    };
    a.guard(2).await;

    let shown = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!shown.can_allow, "{shown:#?}");
    let why = shown.blocked.clone().unwrap_or_default();
    assert!(why.contains("may use up to Sandboxed"), "{why}");
    assert!(shown.can_without, "{shown:#?}");
    let card = t.rt.makers().request_card(&change.id).await.unwrap();
    assert!(!card.can_allow, "{card:#?}");
    let why = card.blocked.clone().unwrap_or_default();
    assert!(why.contains("may use up to Sandboxed"), "{why}");
    // Allow is refused as the card says.
    match t
        .rt
        .makers()
        .decide_request(
            &request.id,
            MakeDecision::Allow,
            None,
            shown.shown.as_deref(),
        )
        .await
    {
        Err(Error::NotAllowed(text)) => assert!(text.contains("may use up to Sandboxed")),
        other => panic!("expected NotAllowed, got {other:?}"),
    }
    assert_eq!(dot_names(t).await, ["Feed Reader", "Researcher"]);
    assert_eq!(made.spec.policy.preset, Preset::Sandboxed);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_person_without_the_rights_cannot_allow() {
    let a = Acme::start().await;
    let t = &a.t;
    a.join_as("member").await;
    let maker =
        t.rt.create_dot(maker_spec(t), Some(ACME.into()), None)
            .await
            .unwrap();
    let run = live_run(t, &maker).await;
    let request = request(t, &run, ask_browser("Blog Watcher")).await;
    // A Member may answer.
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(card.can_allow && card.can_without);
    assert_eq!(card.blocked, None);

    a.become_(2, "viewer").await;
    let why = "Acme Engineering: a Viewer can't create or change dots. Ask an Admin or Owner.";
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(!card.can_allow);
    assert!(!card.can_without);
    assert_eq!(card.blocked.as_deref(), Some(why));
    assert_eq!(card.request.status, RequestStatus::Waiting);
    for decision in [MakeDecision::Allow, MakeDecision::Without] {
        match t
            .rt
            .makers()
            .decide_request(&request.id, decision, None, card.shown.as_deref())
            .await
        {
            Err(Error::NotAllowed(text)) => assert_eq!(text, why),
            other => panic!("expected NotAllowed, got {other:?}"),
        }
    }
    // Nothing was made, and the request still waits for someone who may answer.
    assert_eq!(stored(t, &request).await, request);
    assert_eq!(dot_names(t).await, ["Researcher"]);
    assert!(folders(&t.root).is_empty());
    assert!(wakes(t).await.is_empty());
    assert_eq!(kinds(t, &run).await, [MakingEventKind::Asked]);

    // With the rights back, the answer is taken.
    a.become_(3, "member").await;
    let after = decide(t, &request, MakeDecision::Allow).await;
    assert_eq!(after.status, RequestStatus::Allowed);
    assert_eq!(dot_names(t).await, ["Blog Watcher", "Researcher"]);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_viewer_may_refuse() {
    let a = Acme::start().await;
    let t = &a.t;
    a.join_as("member").await;
    let maker =
        t.rt.create_dot(maker_spec(t), Some(ACME.into()), None)
            .await
            .unwrap();
    let run = live_run(t, &maker).await;
    let request = request(t, &run, ask_browser("Blog Watcher")).await;
    a.become_(2, "viewer").await;
    // Not making it needs no right: nothing is created or changed.
    let after = decide(t, &request, MakeDecision::Refuse).await;
    assert_eq!(after.status, RequestStatus::Refused);
    assert_eq!(dot_names(t).await, ["Researcher"]);
    assert_eq!(wakes(t).await, [REFUSED]);
    t.rt.shutdown();
}

// ---- fix 14b part 2: the card and the answer judge against the maker as it is then

/// A connection to the runtime's database beside its own pool, for what no API does.
async fn db(t: &T) -> sqlx::SqliteConnection {
    use sqlx::ConnectOptions;
    sqlx::sqlite::SqliteConnectOptions::new()
        .filename(t.dir.path().join("data").join("dots.db"))
        .foreign_keys(false)
        .connect()
        .await
        .unwrap()
}

const SHRANK: &str =
    "Researcher's own settings changed since it asked. Don't make it, and let it ask again.";

#[tokio::test]
async fn the_maker_narrowed_since_the_ask_blocks_allow() {
    let t = start().await;
    add_github(&t).await;
    let mut s = maker_spec(&t);
    s.uses = uses_github();
    s.variables = github_token();
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &maker).await;
    // The server and its variable are within the maker's reach when it asks: only the browser
    // is an extra.
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.uses = Some(uses_github());
    wanted.wanted.variables = Some(github_token());
    wanted.wanted.permissions = Some(WantedPermissions::Preset(Preset::Sandboxed));
    let request = request(&t, &run, wanted).await;
    assert_eq!(request.extras, [Extra::Browser]);
    // The person takes the server and its variable from the maker, and makes it Read-only.
    let mut narrowed = maker.spec.clone();
    narrowed.uses = Uses::default();
    narrowed.variables = Grants::default();
    narrowed.policy = dots_core::policy::Policy::preset(Preset::ReadOnly);
    t.rt.update_dot(&maker.id, narrowed).await.unwrap();

    // The card shows what Allow would give now, and Allow is off.
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        card.lines,
        [
            "The MCP server 'github'",
            "The variable GITHUB_TOKEN for the server 'github'",
            "The preset Sandboxed (Researcher has Read-only)",
            "The browser"
        ]
    );
    assert!(!card.can_allow);
    assert_eq!(card.blocked.as_deref(), Some(SHRANK));
    assert!(card.can_without);
    assert_eq!(card.request.status, RequestStatus::Waiting);
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Allow, None, None)
                .await
        ),
        SHRANK
    );
    assert_eq!(dot_names(&t).await, ["Researcher"]);
    assert!(wakes(&t).await.is_empty());
    // Without holds nothing beyond the maker's reach as it is now.
    let after = decide(&t, &request, MakeDecision::Without).await;
    let dot =
        t.rt.store()
            .get_dot(after.made_dot_id.as_deref().unwrap())
            .await
            .unwrap();
    assert_eq!(dot.spec.uses, Uses::default());
    assert_eq!(dot.spec.variables, Grants::default());
    assert_eq!(dot.spec.policy.preset, Preset::ReadOnly);
    assert!(!dot.spec.browser);
}

#[tokio::test]
async fn a_limit_reached_while_a_request_waits_is_shown_before_allow() {
    let t = start().await;
    let maker = maker_with(&t, 3, 1).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    assert_eq!(request.over, None);
    let seen = t.rt.makers().request_card(&request.id).await.unwrap().shown;
    // Another dot takes the last place while the request waits.
    make(&t, &run, "One").await;
    let mut rx = t.rt.subscribe();

    // The person answers the card they saw, which had no limit line.
    let reached = "A limit was reached since the card was shown: it would be the 2nd made dot \
                   kept; the limit is 1. Look at the card again, and answer again if you still \
                   want this dot.";
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Allow, None, seen.as_deref())
                .await
        ),
        reached
    );
    assert_eq!(dot_names(&t).await, ["One", "Researcher"]);
    assert!(updated(&mut rx).contains(&(maker.id.clone(), run.root_run_id.clone())));
    // The card shows the limit now, and the answer to that card is taken.
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        card.lines,
        ["The browser", "2nd made dot kept; the limit is 1"]
    );
    assert_eq!(
        card.limit.as_deref(),
        Some("2nd made dot kept; the limit is 1")
    );
    // Reading the card wrote nothing: the request keeps the limit it was stored with.
    assert_eq!(stored(&t, &request).await.over, None);
    assert!(card.can_allow);
    let after = decide(&t, &request, MakeDecision::Allow).await;
    assert_eq!(after.status, RequestStatus::Allowed);
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "One", "Researcher"]);
}

#[tokio::test]
async fn a_limit_raised_while_a_request_waits_leaves_the_card() {
    let t = start().await;
    let maker = maker_with(&t, 1, 10).await;
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    let request = request(&t, &run, ask("Two")).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.title, "Researcher wants to make one more dot");
    assert_eq!(card.lines, ["2nd dot in this run; the limit is 1"]);
    // The person raises the run's limit: the card no longer shows one.
    let mut raised = maker.spec.clone();
    raised.making.per_run = 5;
    t.rt.update_dot(&maker.id, raised).await.unwrap();
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert!(card.lines.is_empty(), "{:?}", card.lines);
    assert_eq!(card.limit, None);
    assert!(card.can_allow);
    decide(&t, &request, MakeDecision::Allow).await;
    assert_eq!(dot_names(&t).await, ["One", "Researcher", "Two"]);
}

#[tokio::test]
async fn an_outside_folder_gone_since_leaves_only_dont_make_it() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let outside = t.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let path = real_path(&outside).to_string_lossy().into_owned();
    let mut wanted = ask("Blog Watcher");
    wanted.wanted.folder = Some(path.clone());
    let request = request(&t, &run, wanted).await;
    std::fs::remove_dir(&outside).unwrap();

    let gone = format!("The folder {path} no longer exists.");
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(card.request.status, RequestStatus::Gone);
    assert_eq!(card.blocked.as_deref(), Some(gone.as_str()));
    assert!(!card.can_allow && !card.can_without);
    assert_eq!(
        conflict(
            t.rt.makers()
                .decide_request(&request.id, MakeDecision::Allow, None, None)
                .await
        ),
        format!("{gone} Only \"Don't make it\" is left.")
    );
    assert!(!outside.exists());
    assert_eq!(dot_names(&t).await, ["Researcher"]);
}

#[tokio::test]
async fn an_answer_that_races_the_makers_delete_says_what_it_did() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    // As when the maker is deleted while the answer makes the dot: the request's row goes
    // with the maker once the dot is stored.
    let mut conn = db(&t).await;
    sqlx::query(
        "CREATE TRIGGER gone_with_maker AFTER INSERT ON making_events WHEN NEW.kind = 'made' \
         BEGIN DELETE FROM make_requests; END",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    drop(conn);

    let after = decide(&t, &request, MakeDecision::Allow).await;

    assert_eq!(after.id, request.id);
    assert_eq!(after.status, RequestStatus::Allowed);
    assert!(after.decided_at.is_some());
    let made = after.made_dot_id.as_deref().expect("the made dot");
    assert_eq!(
        t.rt.store().get_dot(made).await.unwrap().spec.name,
        "Blog Watcher"
    );
}

#[tokio::test]
async fn a_request_row_that_cannot_be_read_does_not_hide_the_others() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let bad = request(&t, &run, ask_browser("One")).await;
    let good = request(&t, &run, ask_browser("Two")).await;
    let mut conn = db(&t).await;
    sqlx::query("UPDATE make_requests SET kind = 'bogus' WHERE id = ?")
        .bind(&bad.id)
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    let store = t.rt.store();
    for _ in 0..2 {
        let listed = store.list_make_requests(None, true).await.unwrap();
        assert_eq!(listed, std::slice::from_ref(&good));
    }
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.make_requests.len(), 1);
    assert_eq!(inbox.make_requests[0].id, good.id);
}

#[tokio::test]
async fn the_card_carries_the_new_dots_instructions_and_schedule() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.schedule = Some(Some(every(120)));
    let request = request(&t, &run, wanted).await;
    let card = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        card.instructions.as_deref(),
        Some("Read the three blogs and list what is new.")
    );
    assert_eq!(card.schedule.as_deref(), Some("every 2 hours"));
    assert!(card.asked.is_empty());
    // The Inbox item is the card's.
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.make_requests[0].title, card.title);
    assert_eq!(inbox.make_requests[0].lines, card.lines);
}

// ---- fix 14b part 2b: the answer carries the card's limit line; Without names what it left out

#[tokio::test]
async fn allow_on_an_open_card_after_another_reader_computed_a_new_limit_is_refused() {
    let t = start().await;
    let maker = maker_with(&t, 3, 1).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    // The person opens the card: it shows no limit.
    let open = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(open.limit, None);
    // The maker takes the last place, and the tray's count and the Inbox read the card again.
    make(&t, &run, "One").await;
    t.rt.inbox_count().await.unwrap();
    t.rt.inbox().await.unwrap();
    assert_eq!(stored(&t, &request).await.over, None);

    // Allow on the card still open is refused: it did not show the limit.
    let reached = "A limit was reached since the card was shown: it would be the 2nd made dot \
                   kept; the limit is 1. Look at the card again, and answer again if you still \
                   want this dot.";
    let answer = t.rt.makers().decide_request(
        &request.id,
        MakeDecision::Allow,
        open.limit.as_deref(),
        open.shown.as_deref(),
    );
    assert_eq!(conflict(answer.await), reached);
    assert_eq!(dot_names(&t).await, ["One", "Researcher"]);
    // The card read again shows it, and Allow on that card is taken.
    let again = t.rt.makers().request_card(&request.id).await.unwrap();
    assert_eq!(
        again.limit.as_deref(),
        Some("2nd made dot kept; the limit is 1")
    );
    let after =
        t.rt.makers()
            .decide_request(
                &request.id,
                MakeDecision::Allow,
                again.limit.as_deref(),
                again.shown.as_deref(),
            )
            .await
            .unwrap();
    assert_eq!(after.status, RequestStatus::Allowed);
    assert_eq!(dot_names(&t).await, ["Blog Watcher", "One", "Researcher"]);
}

#[tokio::test]
async fn without_tells_the_maker_what_was_left_out_as_it_stood_then() {
    let t = start().await;
    add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let mut wanted = ask_browser("Blog Watcher");
    wanted.wanted.uses = Some(uses_github());
    let request = request(&t, &run, wanted).await;
    assert_eq!(request.extras.len(), 2, "{:?}", request.extras);
    // The person gives the maker the server: only the browser is beyond its reach now.
    let mut spec = maker.spec.clone();
    spec.uses = uses_github();
    t.rt.update_dot(&maker.id, spec).await.unwrap();

    decide(&t, &request, MakeDecision::Without).await;

    assert_eq!(
        wakes(&t).await,
        [
            "The person chose to make \"Blog Watcher\" without what was beyond your reach. It \
             was made without: The browser."
        ]
    );
}

/// A waiting change whose Library item went since, on the first read that finds it gone, names
/// the made dot as it is called now: the person renamed it after the maker asked.
#[tokio::test]
async fn a_change_found_gone_names_the_dot_as_it_is_called_now() {
    let t = start().await;
    let github = add_github(&t).await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let ask = ChangeDot {
        dot: "Blog Watcher".into(),
        purpose: None,
        wanted: Wanted {
            uses: Some(uses_github()),
            ..Wanted::default()
        },
    };
    let outcome = t.rt.makers().change_dot(&run.id, ask).await.unwrap();
    assert!(
        matches!(outcome, ChangeOutcome::Waiting { .. }),
        "{outcome:?}"
    );
    let mut s = t.rt.store().get_dot(&made.id).await.unwrap().spec;
    s.name = "Feed Watcher".into();
    t.rt.update_dot(&made.id, s).await.unwrap();
    t.rt.store().remove_library_item(&github).await.unwrap();
    let waiting =
        t.rt.store()
            .list_make_requests(Some(&maker.id), true)
            .await
            .unwrap();

    let card = t.rt.makers().request_card(&waiting[0].id).await.unwrap();

    assert_eq!(card.request.status, RequestStatus::Gone);
    assert_eq!(card.name, "Feed Watcher");
    assert!(card.title.contains("Feed Watcher"), "{}", card.title);
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.make_requests[0].name, "Feed Watcher");
}

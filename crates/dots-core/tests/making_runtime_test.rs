//! What the app needs of making, on `Runtime` (dots-make-dots design section 11): the ask card
//! and the person's answer, the waiting requests in the Inbox and its count, the cards of a
//! maker's chat topic, the "uses" sentence of a made dot, a maker's count of kept dots, the
//! events the app notifies on, and what a person's delete of a made dot or of a maker leaves.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, roster, sign, sign_roster, token, ACME, ACME_NAME,
    ISSUED, START, TOKEN,
};
use common::{fake_dot, spec};
use dots_core::accounts::identity;
use dots_core::events::RuntimeEvent;
use dots_core::inbox::{InboxKey, InboxKind};
use dots_core::library::Uses;
use dots_core::making::cards::{uses_line, CardWhat, JobCard, MakingCard};
use dots_core::making::jobs::{deleted_message, JobEnd, JobStatus, MakingJob};
use dots_core::making::log::{MakingEvent, MakingEventKind};
use dots_core::making::manage::{change_went_message, ChangeDot, ChangeOutcome};
use dots_core::making::reach::Wanted;
use dots_core::making::requests::{
    MakeDecision, MakeRequest, PendingMakeRequest, RequestKind, RequestStatus,
};
use dots_core::making::service::{MakeDot, MakeOutcome};
use dots_core::model::{Dot, DotSpec, Making, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::vault::{Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
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

/// As `making_request_test.rs` has it: no queued run starts, so a run is put in the state a
/// test needs by hand, and a wake of a busy topic waits where a test can read it.
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

/// Puts the queued run `run_id` in the running state by hand, with a session, as a run the
/// runner started.
async fn started(t: &T, run_id: &str) -> Run {
    assert!(t.rt.store().claim_run(run_id).await.unwrap());
    t.rt.store()
        .set_session(run_id, &format!("session-{run_id}"))
        .await
        .unwrap();
    t.rt.store().get_run(run_id).await.unwrap()
}

/// A run of `dot` that is running and starts a topic, with `payload`.
async fn live_run_with(t: &T, dot: &Dot, trigger: TriggerKind, payload: serde_json::Value) -> Run {
    let mut new = NewRun::new(&dot.id, trigger);
    new.payload = Some(payload);
    let run = t.rt.store().create_run(&new).await.unwrap();
    started(t, &run.id).await
}

/// A run of `dot` that is running: a chat the person started with a typed message.
async fn live_run(t: &T, dot: &Dot) -> Run {
    let typed = json!({"message": "watch three blogs", "typed": true});
    live_run_with(t, dot, TriggerKind::Chat, typed).await
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

/// Makes the dot `asked`, which must be within reach and within the limits.
async fn make_as(t: &T, run: &Run, asked: MakeDot) -> Dot {
    match t.rt.makers().make_dot(&run.id, asked).await.unwrap() {
        MakeOutcome::Made { dot, .. } => dot,
        other => panic!("not made: {other:?}"),
    }
}

async fn make(t: &T, run: &Run, name: &str) -> Dot {
    make_as(t, run, ask(name)).await
}

/// Asks for a dot that must come back waiting for the person: its request.
async fn request(t: &T, run: &Run, ask: MakeDot) -> MakeRequest {
    match t.rt.makers().make_dot(&run.id, ask).await.unwrap() {
        MakeOutcome::Waiting { request, .. } => request,
        other => panic!("made: {other:?}"),
    }
}

/// Asks to give the made dot `name` the browser, which must come back waiting for the person:
/// the change request.
async fn change_request(t: &T, run: &Run, name: &str) -> MakeRequest {
    let change = ChangeDot {
        dot: name.into(),
        purpose: None,
        wanted: Wanted {
            browser: Some(true),
            ..Wanted::default()
        },
    };
    match t.rt.makers().change_dot(&run.id, change).await.unwrap() {
        ChangeOutcome::Waiting { request, .. } => request,
        other => panic!("changed: {other:?}"),
    }
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

/// The makers' log of the topic `run` belongs to.
async fn log(t: &T, run: &Run) -> Vec<MakingEvent> {
    t.rt.store()
        .list_making_events(&run.root_run_id)
        .await
        .unwrap()
}

/// The cards of the topic `run` belongs to.
async fn cards(t: &T, run: &Run) -> Vec<MakingCard> {
    t.rt.making_cards(&run.root_run_id).await.unwrap()
}

/// The jobs given in the maker's topic `run` belongs to, oldest first.
async fn jobs(t: &T, run: &Run) -> Vec<MakingJob> {
    t.rt.store()
        .making_jobs_of_topic(&run.root_run_id)
        .await
        .unwrap()
}

/// Gives the made dot `name` a job from the maker's run `run` and starts its run by hand: the
/// job and its run.
async fn working_job(t: &T, run: &Run, name: &str) -> (MakingJob, Run) {
    t.rt.makers()
        .give_job(&run.id, name, "Read the blogs now.")
        .await
        .unwrap();
    let job = jobs(t, run).await.pop().expect("a job was stored");
    let its = started(t, job.last_run_id.as_deref().expect("the job has a run")).await;
    (job, its)
}

/// The messages of the wakes that wait, oldest first.
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

/// The events the app notifies on that were published since the last call, as JSON.
fn notices(rx: &mut Receiver<RuntimeEvent>) -> Vec<serde_json::Value> {
    let mut seen = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if matches!(
            ev,
            RuntimeEvent::MakeRequested { .. } | RuntimeEvent::DotMade { .. }
        ) {
            seen.push(serde_json::to_value(&ev).unwrap());
        }
    }
    seen
}

/// The card's part every card has, without what its kind adds.
fn head(card: &MakingCard) -> (&str, &str, &str, &str, Option<&str>, &str, &str) {
    (
        card.id.as_str(),
        card.maker_dot_id.as_str(),
        card.maker_name.as_str(),
        card.run_id.as_str(),
        card.dot_id.as_deref(),
        card.dot_name.as_str(),
        card.created_at.as_str(),
    )
}

#[tokio::test]
async fn waiting_requests_are_in_the_inbox_and_its_count_until_each_is_answered() {
    let t = start().await;
    let mut s = maker_spec(&t);
    s.making = Making {
        on: true,
        per_run: 1,
        total: 10,
    };
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let run = live_run(&t, &maker).await;
    make(&t, &run, "One").await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);

    let two = request(&t, &run, ask("Two")).await;
    let watcher = request(&t, &run, ask_browser("Blog Watcher")).await;
    let change = change_request(&t, &run, "One").await;

    // Newest first, each with what the Inbox sorts, links and titles it by.
    let inbox = t.rt.inbox().await.unwrap();
    let item = |request: &MakeRequest, title: &str, lines: &[&str]| PendingMakeRequest {
        id: request.id.clone(),
        dot_id: maker.id.clone(),
        dot_name: "Researcher".into(),
        root_run_id: run.root_run_id.clone(),
        kind: request.kind,
        name: request.name.clone(),
        purpose: "Reads the feeds".into(),
        title: title.into(),
        lines: lines.iter().map(|l| l.to_string()).collect(),
        status: RequestStatus::Waiting,
        created_at: request.created_at.clone(),
    };
    let limit = "2nd dot in this run; the limit is 1";
    assert_eq!(
        inbox.make_requests,
        [
            item(
                &change,
                "Researcher wants to give One more than it has",
                &["The browser"]
            ),
            item(
                &watcher,
                "Researcher wants to make a dot with more than it has",
                &["The browser", limit]
            ),
            item(&two, "Researcher wants to make one more dot", &[limit]),
        ]
    );
    assert_eq!(change.kind, RequestKind::Change);
    assert_eq!(
        serde_json::to_value(&inbox.make_requests[2]).unwrap(),
        json!({
            "id": two.id,
            "dot_id": maker.id,
            "dot_name": "Researcher",
            "root_run_id": run.root_run_id,
            "kind": "make",
            "name": "Two",
            "purpose": "Reads the feeds",
            "title": "Researcher wants to make one more dot",
            "lines": [limit],
            "status": "waiting",
            "created_at": two.created_at,
        })
    );
    assert_eq!(t.rt.inbox_count().await.unwrap(), 3);
    // They count as what the maker has for the person, as a proposal's card does.
    let overview = t.rt.dot_overview().await.unwrap();
    let row = overview.iter().find(|row| row.dot.id == maker.id).unwrap();
    assert_eq!(row.needs_you, 3);

    // The same requests as whole cards, for the Inbox's section, newest first.
    let waiting = t.rt.waiting_make_requests().await.unwrap();
    let ids: Vec<&str> = waiting.iter().map(|c| c.request.id.as_str()).collect();
    assert_eq!(ids, [&change.id, &watcher.id, &two.id]);
    assert_eq!(
        waiting[1],
        t.rt.make_request_card(&watcher.id).await.unwrap()
    );
    assert_eq!(waiting[1].maker_name, "Researcher");
    assert!(waiting[1].can_allow && waiting[1].can_without);
    assert!(waiting[2].can_allow && !waiting[2].can_without);

    // Each of the three answers takes its request out of the Inbox and the count.
    let answers = [
        (&two, MakeDecision::Allow, RequestStatus::Allowed),
        (&watcher, MakeDecision::Without, RequestStatus::MadeWithout),
        (&change, MakeDecision::Refuse, RequestStatus::Refused),
    ];
    let mut left = 3;
    for (request, decision, status) in answers {
        // As the card shows it: the request for Two is past the run's limit.
        let card = t.rt.make_request_card(&request.id).await.unwrap();
        let after =
            t.rt.decide_make_request(&request.id, decision, card.limit.as_deref())
                .await;
        assert_eq!(after.unwrap().status, status);
        left -= 1;
        let inbox = t.rt.inbox().await.unwrap();
        assert_eq!(inbox.make_requests.len(), left);
        assert!(inbox.make_requests.iter().all(|r| r.id != request.id));
        assert_eq!(t.rt.inbox_count().await.unwrap(), left);
    }
    assert!(t.rt.waiting_make_requests().await.unwrap().is_empty());
    let overview = t.rt.dot_overview().await.unwrap();
    let row = overview.iter().find(|row| row.dot.id == maker.id).unwrap();
    assert_eq!(row.needs_you, 0);
    // An answer given twice is refused as the makers refuse it.
    assert!(matches!(
        t.rt.decide_make_request(&two.id, MakeDecision::Refuse, None)
            .await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn a_request_is_snoozed_and_marked_done_as_a_proposal_is() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let request = request(&t, &run, ask_browser("Blog Watcher")).await;
    let key = InboxKey::new(InboxKind::MakeRequest, request.id.clone());
    let title = "Researcher wants to make a dot with more than it has";

    t.rt.inbox_snooze(std::slice::from_ref(&key), Utc::now() + TimeDelta::hours(1))
        .await
        .unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.make_requests.is_empty());
    assert_eq!(inbox.snoozed.len(), 1);
    assert_eq!(inbox.snoozed[0].key(), key);
    assert_eq!(inbox.snoozed[0].title, title);
    assert_eq!(inbox.snoozed[0].dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    let overview = t.rt.dot_overview().await.unwrap();
    assert_eq!(overview[0].needs_you, 0);

    t.rt.inbox_unsnooze(&key).await.unwrap();
    assert_eq!(t.rt.inbox().await.unwrap().make_requests.len(), 1);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    // Done hides it too, as it hides a proposal; the request itself still waits.
    t.rt.inbox_done(&key).await.unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert!(inbox.make_requests.is_empty());
    assert_eq!(inbox.done.len(), 1);
    assert_eq!(inbox.done[0].title, title);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    let card = t.rt.make_request_card(&request.id).await.unwrap();
    assert_eq!(card.request.status, RequestStatus::Waiting);
    t.rt.inbox_undo_done(&key).await.unwrap();
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    // A key of a request that is answered is no longer in the Inbox.
    t.rt.decide_make_request(&request.id, MakeDecision::Refuse, None)
        .await
        .unwrap();
    assert!(matches!(
        t.rt.inbox_done(&key).await,
        Err(Error::NotFound(_))
    ));
}

/// A runtime joined to Acme, whose signed member list the fake OpenBao serves, as
/// `making_request_test.rs` sets it up, with no run starting.
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
        let rt = Runtime::start(cfg).await.unwrap();
        std::fs::create_dir_all(dir.path().join("work")).unwrap();
        Acme {
            t: T { dir, rt },
            bao,
            keys: keys(),
        }
    }

    /// Serves a member list at `seq` with an owner who is not this PC, and this PC as `role`.
    async fn serve_list(&self, seq: u64, role: &str) {
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
        let list = sign_roster(&self.keys, &roster(ACME, seq, members));
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
}

#[tokio::test]
async fn a_viewer_sees_the_card_and_why_it_cannot_allow() {
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

    // The request is in the Inbox for a Viewer too, and its card says why Allow is off.
    let why = "Acme Engineering: a Viewer can't create or change dots. Ask an Admin or Owner.";
    assert_eq!(t.rt.inbox().await.unwrap().make_requests.len(), 1);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    let card = t.rt.make_request_card(&request.id).await.unwrap();
    assert!(!card.can_allow && !card.can_without);
    assert_eq!(card.blocked.as_deref(), Some(why));
    assert_eq!(t.rt.waiting_make_requests().await.unwrap(), [card]);
    for decision in [MakeDecision::Allow, MakeDecision::Without] {
        match t.rt.decide_make_request(&request.id, decision, None).await {
            Err(Error::NotAllowed(text)) => assert_eq!(text, why),
            other => panic!("expected NotAllowed, got {other:?}"),
        }
    }
    // Nothing was made, and the request still waits for someone who may answer.
    assert_eq!(t.rt.store().list_dots().await.unwrap().len(), 1);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);

    a.become_(3, "member").await;
    let after =
        t.rt.decide_make_request(&request.id, MakeDecision::Allow, None);
    assert_eq!(after.await.unwrap().status, RequestStatus::Allowed);
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn the_topics_cards_tell_a_make_a_job_an_ask_a_change_and_a_delete() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    assert!(cards(&t, &run).await.is_empty());

    // Make: a dot in a folder below the maker's, with a schedule.
    let mut asked = ask("Blog Watcher");
    asked.wanted.folder = Some("research/blogs".into());
    asked.wanted.schedule = Some(Some(every(120)));
    let made = make_as(&t, &run, asked).await;
    // Job: given, and ended by its run.
    let (job, its) = working_job(&t, &run, "Blog Watcher").await;
    let working = cards(&t, &run).await;
    t.rt.store()
        .finish_run(
            &its.id,
            RunStatus::Succeeded,
            Some("Three new posts."),
            None,
        )
        .await
        .unwrap();
    t.rt.makers().settle_jobs().await.unwrap();
    // Ask: a change that gives the dot the browser, which the maker does not have.
    let change = change_request(&t, &run, "Blog Watcher").await;
    let asking = cards(&t, &run).await;
    let waiting_card = t.rt.make_request_card(&change.id).await.unwrap();
    // Allow, then a change within reach.
    t.rt.decide_make_request(&change.id, MakeDecision::Allow, None)
        .await
        .unwrap();
    // The person gives the maker the browser too, so the dot holds nothing beyond its reach
    // and a new purpose is the maker's to give (a dot that holds more asks the person).
    let mut with_browser = t.rt.store().get_dot(&maker.id).await.unwrap().spec;
    with_browser.browser = true;
    t.rt.update_dot(&maker.id, with_browser).await.unwrap();
    let purpose = ChangeDot {
        dot: "Blog Watcher".into(),
        purpose: Some("Reads four feeds".into()),
        wanted: Wanted::default(),
    };
    t.rt.makers().change_dot(&run.id, purpose).await.unwrap();
    let before = cards(&t, &run).await;

    // One card per thing that happened, in order. The person's answer has none of its own: it
    // shows on the ask card.
    let entries = log(&t, &run).await;
    let kinds: Vec<MakingEventKind> = entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            MakingEventKind::Made,
            MakingEventKind::JobStarted,
            MakingEventKind::JobEnded,
            MakingEventKind::Asked,
            MakingEventKind::Changed,
            MakingEventKind::Answered,
            MakingEventKind::Changed,
        ]
    );
    let shown: Vec<&MakingEvent> = entries
        .iter()
        .filter(|e| e.kind != MakingEventKind::Answered)
        .collect();
    assert_eq!(before.len(), shown.len());
    for (card, entry) in before.iter().zip(&shown) {
        assert_eq!(
            head(card),
            (
                entry.id.as_str(),
                maker.id.as_str(),
                "Researcher",
                run.id.as_str(),
                Some(made.id.as_str()),
                "Blog Watcher",
                entry.created_at.as_str(),
            )
        );
        assert_eq!(card.root_run_id, run.root_run_id);
    }

    // The made card: the purpose as it is now, the schedule in words, and what the dot uses.
    assert_eq!(
        before[0].what,
        CardWhat::Made {
            purpose: "Reads four feeds".into(),
            schedule: Some("every 2 hours".into()),
            uses: Some("Uses: the browser, folder research/blogs. No secrets.".into()),
        }
    );
    assert_eq!(
        serde_json::to_value(&before[0]).unwrap(),
        json!({
            "id": entries[0].id,
            "maker_dot_id": maker.id,
            "maker_name": "Researcher",
            "run_id": run.id,
            "root_run_id": run.root_run_id,
            "dot_id": made.id,
            "dot_name": "Blog Watcher",
            "created_at": entries[0].created_at,
            "kind": "made",
            "purpose": "Reads four feeds",
            "schedule": "every 2 hours",
            "uses": "Uses: the browser, folder research/blogs. No secrets.",
        })
    );
    // Before the browser was allowed, the sentence had none.
    assert_eq!(
        working[0].what,
        CardWhat::Made {
            purpose: "Reads the feeds".into(),
            schedule: Some("every 2 hours".into()),
            uses: Some("Uses: folder research/blogs. No secrets.".into()),
        }
    );

    // The job's cards: "is working" while it works, and how it ended once it has.
    let job_card = |status: Option<JobStatus>, how: Option<JobEnd>, run_exists: bool| JobCard {
        job_id: job.id.clone(),
        message: "Read the blogs now.".into(),
        job_run_id: Some(its.id.clone()),
        run_exists,
        status,
        how,
    };
    assert_eq!(working.len(), 2);
    assert_eq!(
        working[1].what,
        CardWhat::JobStarted(job_card(Some(JobStatus::Working), None, true))
    );
    let ended = job_card(Some(JobStatus::Ended), Some(JobEnd::Succeeded), true);
    assert_eq!(before[1].what, CardWhat::JobStarted(ended.clone()));
    assert_eq!(before[2].what, CardWhat::JobEnded(ended));
    assert_eq!(
        serde_json::to_value(&before[2].what).unwrap(),
        json!({
            "kind": "job_ended",
            "job_id": job.id,
            "message": "Read the blogs now.",
            "job_run_id": its.id,
            "run_exists": true,
            "status": "ended",
            "how": "succeeded",
        })
    );

    // The ask card carries the whole card: as it waited, and then how it ended.
    assert_eq!(
        waiting_card.title,
        "Researcher wants to give Blog Watcher more than it has"
    );
    assert_eq!(waiting_card.lines, ["The browser"]);
    assert!(waiting_card.can_allow && !waiting_card.can_without);
    assert_eq!(waiting_card.request.status, RequestStatus::Waiting);
    assert_eq!(
        asking[3].what,
        CardWhat::Asked {
            request_id: change.id.clone(),
            card: Some(waiting_card.clone()),
        }
    );
    let answered = t.rt.make_request_card(&change.id).await.unwrap();
    assert_eq!(answered.request.status, RequestStatus::Allowed);
    assert!(answered.request.decided_at.is_some());
    assert!(!answered.can_allow && !answered.can_without);
    assert_eq!(
        before[3].what,
        CardWhat::Asked {
            request_id: change.id.clone(),
            card: Some(answered),
        }
    );
    let as_json = serde_json::to_value(&asking[3].what).unwrap();
    assert_eq!(as_json["kind"], "asked");
    assert_eq!(as_json["request_id"], change.id.as_str());
    assert_eq!(
        as_json["card"],
        serde_json::to_value(&waiting_card).unwrap()
    );

    // The two changes, each with what changed.
    let changed = |what: &str| CardWhat::Changed {
        what: vec![what.to_string()],
    };
    assert_eq!(before[4].what, changed("browser"));
    assert_eq!(before[5].what, changed("purpose"));
    assert_eq!(
        serde_json::to_value(&before[5].what).unwrap(),
        json!({"kind": "changed", "what": ["purpose"]})
    );

    // Delete: the maker deletes the dot. Every card of it says the dot is gone.
    t.rt.makers()
        .delete_dot(&run.id, "Blog Watcher")
        .await
        .unwrap();
    let after = cards(&t, &run).await;
    assert_eq!(after.len(), 7);
    assert!(after.iter().all(|card| card.dot_id.is_none()));
    assert!(after.iter().all(|card| card.dot_name == "Blog Watcher"));
    // The made card keeps the purpose the dot was made with.
    assert_eq!(
        after[0].what,
        CardWhat::Made {
            purpose: "Reads the feeds".into(),
            schedule: None,
            uses: None,
        }
    );
    // The job's rows and runs went with the dot; how it ended stays on its last card.
    assert_eq!(
        after[1].what,
        CardWhat::JobStarted(job_card(None, None, false))
    );
    assert_eq!(
        after[2].what,
        CardWhat::JobEnded(job_card(None, Some(JobEnd::Succeeded), false))
    );
    // The change request went with the dot it was for: its card says so and does not fail.
    assert_eq!(
        after[3].what,
        CardWhat::Asked {
            request_id: change.id.clone(),
            card: None,
        }
    );
    assert_eq!(after[6].what, CardWhat::Deleted);
    assert_eq!(
        serde_json::to_value(&after[6].what).unwrap(),
        json!({"kind": "deleted"})
    );
    // A card reads back as it was written.
    let text = serde_json::to_string(&before).unwrap();
    let back: Vec<MakingCard> = serde_json::from_str(&text).unwrap();
    assert_eq!(back, before);
}

#[test]
fn the_uses_sentence_names_what_a_dot_uses_and_never_a_value() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("research").join("blogs")).unwrap();
    let maker = fake_dot("Researcher", &work, WorkspaceMode::Folder);
    let folder = |path: &std::path::Path| path.to_string_lossy().to_string();

    // Nothing: no Library item, no browser, no folder yet, no variable.
    let mut nothing = spec("Blog Watcher");
    nothing.workdir = String::new();
    assert_eq!(
        uses_line(&nothing, Some(&maker)),
        "Uses: nothing. No secrets."
    );

    // Items, by kind, then the browser and the person's Claude settings.
    let mut items = nothing.clone();
    items.uses = Uses {
        mcp_servers: vec!["github".into()],
        skills: vec!["summarize".into()],
        plugins: vec!["formatter".into()],
        context: vec!["handbook".into()],
        apps: vec!["projects".into()],
    };
    items.browser = true;
    items.use_user_settings = true;
    assert_eq!(
        uses_line(&items, Some(&maker)),
        "Uses: MCP server github, skill summarize, plugin formatter, context collection \
         handbook, app projects, the browser, your Claude settings. No secrets."
    );

    // Variables: each name with what it is for. The sentence is built from the dot's settings,
    // which hold names only.
    let mut variables = nothing.clone();
    variables.uses.mcp_servers = vec!["github".into()];
    variables.variables = Grants(
        [
            ("mcp:github".to_string(), vec!["GITHUB_TOKEN".to_string()]),
            (
                "shell".to_string(),
                vec!["API_KEY".to_string(), "REGION".to_string()],
            ),
        ]
        .into(),
    );
    assert_eq!(
        uses_line(&variables, Some(&maker)),
        "Uses: MCP server github. Variables: GITHUB_TOKEN for the server 'github', API_KEY for \
         the shell, REGION for the shell."
    );

    // A folder below the maker's is named from there, the maker's own by its owner, and any
    // other in full.
    let mut below = nothing.clone();
    below.workdir = folder(&work.join("research").join("blogs"));
    assert_eq!(
        uses_line(&below, Some(&maker)),
        "Uses: folder research/blogs. No secrets."
    );
    let mut same = nothing.clone();
    same.workdir = folder(&work);
    assert_eq!(
        uses_line(&same, Some(&maker)),
        "Uses: Researcher's folder. No secrets."
    );
    let elsewhere = dir.path().join("Dots").join("blog-watcher");
    let mut other = nothing.clone();
    other.workdir = folder(&elsewhere);
    let full = format!("Uses: folder {}. No secrets.", folder(&elsewhere));
    assert_eq!(uses_line(&other, Some(&maker)), full);
    // Without the maker (it was deleted), every folder is named in full.
    let alone = format!("Uses: folder {}. No secrets.", below.workdir);
    assert_eq!(uses_line(&below, None), alone);
}

#[tokio::test]
async fn a_made_dot_the_person_deletes_ends_its_job_and_takes_its_request_with_it() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    // Its job waits for the person's answer: no run of the dot is going.
    let (job, its) = working_job(&t, &run, "Blog Watcher").await;
    t.rt.store()
        .add_question(&made.id, &its.id, "Which of the three first?", None)
        .await
        .unwrap()
        .expect("the run's first question");
    t.rt.store()
        .finish_run(&its.id, RunStatus::Succeeded, Some("Which first?"), None)
        .await
        .unwrap();
    t.rt.makers().settle_jobs().await.unwrap();
    let waiting = t.rt.store().get_making_job(&job.id).await.unwrap();
    assert_eq!(waiting.status, JobStatus::WaitingAnswer);
    // A change the maker asked for it waits too.
    let change = change_request(&t, &run, "Blog Watcher").await;
    assert_eq!(t.rt.inbox().await.unwrap().make_requests.len(), 1);
    assert!(wakes(&t).await.is_empty());
    let mut rx = t.rt.subscribe();

    t.rt.delete_dot(&made.id).await.unwrap();

    // The topic's cards are told to read again.
    let topic = (maker.id.clone(), run.root_run_id.clone());
    let told = updated(&mut rx);
    assert!(!told.is_empty(), "no MakingUpdated was published");
    assert!(told.iter().all(|seen| *seen == topic), "{told:?}");
    // The change request went with the dot, out of the Inbox and its count.
    assert!(matches!(
        t.rt.store().get_make_request(&change.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(t.rt.inbox().await.unwrap().make_requests.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    assert!(matches!(
        t.rt.make_request_card(&change.id).await,
        Err(Error::NotFound(_))
    ));
    // The job ended with the dot, and so did the change it waited on: the maker is woken
    // once, for both.
    let both = format!(
        "{}\n\n{}",
        deleted_message("Blog Watcher"),
        change_went_message("Blog Watcher")
    );
    assert_eq!(wakes(&t).await, [both]);
    assert!(wakes(&t).await[0].starts_with("\"Blog Watcher\" was deleted before the job ended."));
    t.rt.makers().settle_jobs().await.unwrap();
    assert_eq!(wakes(&t).await.len(), 1);
    assert!(t
        .rt
        .store()
        .open_making_jobs(None)
        .await
        .unwrap()
        .is_empty());

    // The cards say the dot is gone, and none fails to read.
    let after = cards(&t, &run).await;
    let kinds: Vec<&str> = after
        .iter()
        .map(|card| match &card.what {
            CardWhat::Made { .. } => "made",
            CardWhat::Asked { .. } => "asked",
            CardWhat::Changed { .. } => "changed",
            CardWhat::Deleted => "deleted",
            CardWhat::JobStarted(_) => "job_started",
            CardWhat::JobWaiting(_) => "job_waiting",
            CardWhat::JobEnded(_) => "job_ended",
        })
        .collect();
    assert_eq!(
        kinds,
        ["made", "job_started", "job_waiting", "asked", "job_ended"]
    );
    assert!(after.iter().all(|card| card.dot_id.is_none()));
    assert_eq!(
        after[0].what,
        CardWhat::Made {
            purpose: "Reads the feeds".into(),
            schedule: None,
            uses: None,
        }
    );
    assert_eq!(
        after[3].what,
        CardWhat::Asked {
            request_id: change.id.clone(),
            card: None,
        }
    );
    let gone = |how: Option<JobEnd>| JobCard {
        job_id: job.id.clone(),
        message: "Read the blogs now.".into(),
        job_run_id: Some(its.id.clone()),
        run_exists: false,
        status: None,
        how,
    };
    assert_eq!(after[1].what, CardWhat::JobStarted(gone(None)));
    assert_eq!(after[2].what, CardWhat::JobWaiting(gone(None)));
    assert_eq!(
        after[4].what,
        CardWhat::JobEnded(gone(Some(JobEnd::Deleted)))
    );
    // A place in the maker's total is free again.
    assert_eq!(t.rt.store().count_made_dots(&maker.id).await.unwrap(), 0);
}

#[tokio::test]
async fn a_made_dot_without_a_job_is_deleted_as_any_dot_and_its_card_is_told() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let made = make(&t, &run, "Blog Watcher").await;
    let plain = t.rt.create_dot(spec_in(&t, "Plain"), None, None).await;
    let plain = plain.unwrap();
    let mut rx = t.rt.subscribe();

    // A dot nobody made is deleted with no word to the makers.
    t.rt.delete_dot(&plain.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    // A made dot tells the topic it was made in, once.
    t.rt.delete_dot(&made.id).await.unwrap();
    assert_eq!(
        updated(&mut rx),
        [(maker.id.clone(), run.root_run_id.clone())]
    );
    assert!(wakes(&t).await.is_empty());
    assert_eq!(log(&t, &run).await.len(), 1);
    // A dot that does not exist is refused as before.
    assert!(matches!(
        t.rt.delete_dot(&made.id).await,
        Err(Error::NotFound(_))
    ));
}

/// The spec of a dot named `name` that works in the test's `work` folder.
fn spec_in(t: &T, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.work().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

#[tokio::test]
async fn a_deleted_maker_leaves_nothing_dangling() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    // A job that ended, whose wake waits behind the maker's run; and a request that waits.
    let (_, its) = working_job(&t, &run, "One").await;
    t.rt.store()
        .finish_run(&its.id, RunStatus::Succeeded, Some("Done."), None)
        .await
        .unwrap();
    t.rt.makers().settle_jobs().await.unwrap();
    assert_eq!(wakes(&t).await.len(), 1);
    let request = request(&t, &run, ask_browser("Two")).await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    assert_eq!(cards(&t, &run).await.len(), 4);
    // The maker's run ends: a dot with a run going is not deleted. The wake still waits.
    t.rt.store()
        .finish_run(&run.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();
    assert_eq!(wakes(&t).await.len(), 1);
    let mut rx = t.rt.subscribe();

    t.rt.delete_dot(&maker.id).await.unwrap();

    // The Inbox and its count are told and no longer have the request.
    let told = updated(&mut rx);
    assert!(
        told.contains(&(maker.id.clone(), run.root_run_id.clone())),
        "{told:?}"
    );
    assert!(t.rt.inbox().await.unwrap().make_requests.is_empty());
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    assert!(t.rt.waiting_make_requests().await.unwrap().is_empty());
    // Its requests, cards, jobs and wakes went with it.
    let store = t.rt.store();
    assert!(store
        .list_make_requests(None, false)
        .await
        .unwrap()
        .is_empty());
    assert!(cards(&t, &run).await.is_empty());
    assert!(jobs(&t, &run).await.is_empty());
    assert!(wakes(&t).await.is_empty());
    assert!(matches!(
        t.rt.make_request_card(&request.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        t.rt.decide_make_request(&request.id, MakeDecision::Allow, None)
            .await,
        Err(Error::NotFound(_))
    ));
    // The dot it made stays, and still says who made it.
    let overview = t.rt.dot_overview().await.unwrap();
    assert_eq!(overview.len(), 1);
    assert_eq!(overview[0].dot.id, one.id);
    let made = overview[0].dot.made.as_ref().expect("who made it");
    assert_eq!(made.by_dot_id, None);
    assert_eq!(made.by_name, "Researcher");
    assert_eq!(overview[0].made_dots, 0);
    // Later passes find nothing to follow.
    t.rt.makers().settle_jobs().await.unwrap();
    t.rt.makers().deliver_wakes().await.unwrap();
}

#[tokio::test]
async fn an_ask_is_always_announced_and_a_made_dot_says_whether_the_person_typed() {
    let t = start().await;
    let maker = maker(&t).await;
    let mut rx = t.rt.subscribe();

    // A run the person started by typing: the made dot is announced as typed.
    let typed = live_run(&t, &maker).await;
    let one = make(&t, &typed, "One").await;
    assert_eq!(
        notices(&mut rx),
        [json!({
            "type": "dot_made",
            "dot_id": maker.id,
            "root_run_id": typed.root_run_id,
            "maker_name": "Researcher",
            "made_dot_id": one.id,
            "name": "One",
            "purpose": "Reads the feeds",
            "typed": true,
        })]
    );
    // An ask is announced whoever started the run, with what its notification says.
    let asked = request(&t, &typed, ask_browser("Blog Watcher")).await;
    assert_eq!(
        notices(&mut rx),
        [json!({
            "type": "make_requested",
            "request_id": asked.id,
            "dot_id": maker.id,
            "root_run_id": typed.root_run_id,
            "title": "Researcher wants to make a dot with more than it has",
            "name": "Blog Watcher",
            "purpose": "Reads the feeds",
        })]
    );
    let change = change_request(&t, &typed, "One").await;
    let said = notices(&mut rx);
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["type"], "make_requested");
    assert_eq!(said[0]["request_id"], change.id.as_str());
    assert_eq!(
        said[0]["title"],
        "Researcher wants to give One more than it has"
    );
    assert_eq!(said[0]["name"], "One");
    // The dot the person's own answer makes is not announced: they made it happen.
    t.rt.decide_make_request(&asked.id, MakeDecision::Allow, None)
        .await
        .unwrap();
    assert!(notices(&mut rx).is_empty());

    // A run that resumes the typed one after an approval is the same turn.
    let mut resume = NewRun::new(&maker.id, TriggerKind::Resume);
    resume.parent_run_id = Some(typed.id.clone());
    resume.root_run_id = Some(typed.root_run_id.clone());
    let resume = t.rt.store().create_run(&resume).await.unwrap();
    let resume = started(&t, &resume.id).await;
    make(&t, &resume, "Two").await;
    let said = notices(&mut rx);
    assert_eq!(said.len(), 1);
    assert_eq!(said[0]["typed"], true);

    // A wake, a scheduled run and an answer to a question are not typed.
    let wake = json!({"message": "\"One\" finished the job.", "wake_id": "w1"});
    let woken = live_run_with(&t, &maker, TriggerKind::Chat, wake).await;
    make(&t, &woken, "Three").await;
    let scheduled = live_run_with(&t, &maker, TriggerKind::Schedule, json!({})).await;
    make(&t, &scheduled, "Four").await;
    let said = notices(&mut rx);
    assert_eq!(said.len(), 2);
    for (event, name, run) in [(&said[0], "Three", &woken), (&said[1], "Four", &scheduled)] {
        assert_eq!(event["type"], "dot_made");
        assert_eq!(event["name"], name);
        assert_eq!(event["root_run_id"], run.root_run_id.as_str());
        assert_eq!(event["typed"], false);
    }
}

#[tokio::test]
async fn a_makers_row_counts_the_dots_it_keeps() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    make(&t, &run, "Two").await;
    // A request that waits is no dot yet.
    request(&t, &run, ask_browser("Three")).await;

    let rows = t.rt.dot_overview().await.unwrap();
    let count = |rows: &[dots_core::model::DotOverview], id: &str| {
        let row = rows.iter().find(|row| row.dot.id == id).unwrap();
        (row.made_dots, row.dot.spec.making.total)
    };
    // "2 of 10" for the maker; a made dot has made none, and says who made it.
    assert_eq!(count(&rows, &maker.id), (2, 10));
    assert_eq!(count(&rows, &one.id), (0, 10));
    let row = rows.iter().find(|row| row.dot.id == one.id).unwrap();
    let made = row.dot.made.as_ref().expect("who made it");
    assert_eq!(made.by_dot_id.as_deref(), Some(maker.id.as_str()));
    assert_eq!(serde_json::to_value(row).unwrap()["made_dots"], json!(0));

    t.rt.delete_dot(&one.id).await.unwrap();
    let rows = t.rt.dot_overview().await.unwrap();
    assert_eq!(count(&rows, &maker.id), (1, 10));
}

#[tokio::test]
async fn the_inbox_names_a_renamed_dot_as_its_card_does() {
    let t = start().await;
    let maker = maker(&t).await;
    let run = live_run(&t, &maker).await;
    let one = make(&t, &run, "One").await;
    let change = change_request(&t, &run, "One").await;
    // The person renames the dot while the change waits.
    let mut renamed = t.rt.store().get_dot(&one.id).await.unwrap().spec;
    renamed.name = "Feeds".into();
    t.rt.update_dot(&one.id, renamed).await.unwrap();

    let card = t.rt.make_request_card(&change.id).await.unwrap();
    assert_eq!(
        card.title,
        "Researcher wants to give Feeds more than it has"
    );
    let inbox = t.rt.inbox().await.unwrap();
    let item = &inbox.make_requests[0];
    assert_eq!(item.title, card.title);
    assert_eq!(item.lines, card.lines);
    assert_eq!(item.lines, ["The browser"]);
    // And by its name now.
    assert_eq!(item.name, "Feeds");
}

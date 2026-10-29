//! A dot proposes an app (app library spec section 6.2): `propose_app` on both engines checks
//! the folder and runs its tests, a problem goes back to the dot, a passing proposal waits for
//! the person, and the person's decision adds the app or continues the dot's thread.

mod common;
mod llm;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::apps::{expenses, expenses_with, write_app, MIGRATION};
use common::{spec, wait_status};
use dots_core::apps::propose::{
    AppProposal, Decision, ProposalStatus, DISCARDED, PROPOSED_ALREADY, SENT,
};
use dots_core::apps::source::read_folder;
use dots_core::apps::tests::NO_TESTS;
use dots_core::library::{AppOrigin, LibraryConfig};
use dots_core::model::{DotSpec, EngineKind, Run, RunStatus, WorkspaceMode};
use dots_core::policy::{Action, Rule};
use dots_core::{Config, Error, Runtime};
use llm::{call as llm_call, text as llm_text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start_with(edit: impl FnOnce(&mut Config, &Path)) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    edit(&mut cfg, dir.path());
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

async fn start() -> Rt {
    start_with(|_, _| {}).await
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
    s
}

/// Writes the app `files` to `<workspace>/<folder>`.
fn in_workspace(t: &Rt, folder: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = t.dir.path().join(folder);
    write_app(&dir, files);
    dir
}

/// Expenses with `tests/expenses.test.js` replaced by `code`.
fn expenses_tested(code: &str) -> Vec<(&'static str, String)> {
    let mut files = expenses_with("tests/expenses.test.js", None);
    files.push(("tests/check.test.js", code.to_string()));
    files
}

async fn proposals(t: &Rt, dot_id: &str) -> Vec<AppProposal> {
    t.rt.store().list_app_proposals(Some(dot_id)).await.unwrap()
}

fn message(run: &Run) -> String {
    run.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string()
}

/// A local dot whose model answers every turn with "Done.", and one finished run of it.
async fn finished_run(t: &Rt, llm: &Llm, name: &str) -> Run {
    let dot =
        t.rt.create_dot(local_spec(t, &llm.endpoint(), name), None, None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await
}

fn dones(n: usize) -> Vec<wiremock::ResponseTemplate> {
    (0..n).map(|_| llm_text("Done.")).collect()
}

#[tokio::test]
async fn a_local_dot_proposes_an_app_and_the_person_adds_it() {
    let t = start().await;
    in_workspace(&t, "expenses", &expenses());
    let llm = Llm::start(vec![
        llm_call(
            "c1",
            "propose_app",
            json!({"path": "expenses", "note": "Tracks expenses by month."}),
        ),
        llm_call(
            "c2",
            "propose_app",
            json!({"path": "expenses", "note": "Again."}),
        ),
        llm_text("Sent for review."),
        llm_text("Thanks."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "builder"), None, None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    // The run ends: the card waits for the person, not the run.
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    let names: Vec<&str> = reqs[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"propose_app"), "{names:?}");
    assert_eq!(tool_result(&reqs[1], "c1"), SENT);
    assert_eq!(tool_result(&reqs[2], "c2"), PROPOSED_ALREADY);

    let list = proposals(&t, &dot.id).await;
    assert_eq!(list.len(), 1);
    let p = &list[0];
    assert_eq!(p.status, ProposalStatus::Pending);
    assert_eq!((p.app_name.as_str(), p.app_id.as_ref()), ("expenses", None));
    assert_eq!(p.note, "Tracks expenses by month.");
    assert_eq!(p.run_id, run.id);
    let s = &p.summary;
    assert_eq!(s.title, "Expenses");
    let tools: Vec<(&str, bool)> = s
        .tools
        .iter()
        .map(|t| (t.name.as_str(), t.writes))
        .collect();
    assert_eq!(
        tools,
        [
            ("add_expense", true),
            ("list_expenses", false),
            ("month_total", false)
        ]
    );
    assert!(s.permissions.is_empty());
    assert!(s.tests.passed());
    assert_eq!(s.tests.results.len(), 2);
    assert_eq!(s.files, 7);
    assert_eq!(s.migrations, ["0001_expenses.sql"]);
    assert_eq!(s.update_from, None);
    let kept = t.rt.apps().proposed_dir(&p.id);
    assert!(read_folder(&kept).is_ok());
    // The transcript names the call by its policy name.
    let shown: Vec<Value> =
        t.rt.store()
            .list_events(&run.id, 0)
            .await
            .unwrap()
            .into_iter()
            .filter(|ev| ev.kind == "tool_call")
            .map(|ev| ev.data["tool"].clone())
            .collect();
    assert_eq!(shown, vec![json!("ProposeApp"); 2]);

    let (decided, next) =
        t.rt.decide_app_proposal(&p.id, Decision::Add)
            .await
            .unwrap();
    assert_eq!(decided.status, ProposalStatus::Added);
    assert_eq!(decided.answer_run_id.as_deref(), Some(next.id.as_str()));
    assert!(decided.decided_at.is_some());
    assert!(
        message(&next).contains("The person added the app expenses to the Library."),
        "{next:?}"
    );
    wait_status(t.rt.store(), &next.id, RunStatus::Succeeded, 30).await;
    let (_, config) = t.rt.apps().get("expenses").await.unwrap();
    assert_eq!(config.version, 1);
    assert_eq!(config.origin, AppOrigin::Proposed);
    assert!(!config.can_start_dots && !config.every_dot);
    assert!(!kept.exists());
    let again = t.rt.decide_app_proposal(&p.id, Decision::Discard).await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
    t.rt.shutdown();
}

#[tokio::test]
async fn a_proposal_that_fails_its_checks_goes_back_to_the_dot() {
    let t = start().await;
    in_workspace(
        &t,
        "untested",
        &expenses_with("tests/expenses.test.js", None),
    );
    in_workspace(
        &t,
        "failing",
        &expenses_tested(
            "test(\"wrong\", () => assert.equal(call(\"month_total\", { month: \"2026-10\" }), 5));\n",
        ),
    );
    in_workspace(&t, "broken", &expenses_with("app.json", Some("{")));
    in_workspace(
        &t,
        "badsql",
        &expenses_with(
            "migrations/0001_expenses.sql",
            Some("CREATE TABL expenses;"),
        ),
    );
    let llm = Llm::start(vec![
        llm_call(
            "c1",
            "propose_app",
            json!({"path": "untested", "note": "n"}),
        ),
        llm_call("c2", "propose_app", json!({"path": "failing", "note": "n"})),
        llm_call("c3", "propose_app", json!({"path": "broken", "note": "n"})),
        llm_call("c4", "propose_app", json!({"path": "badsql", "note": "n"})),
        llm_call(
            "c5",
            "propose_app",
            json!({"path": "../elsewhere", "note": "n"}),
        ),
        llm_call("c6", "propose_app", json!({"path": "failing", "note": " "})),
        llm_text("Nothing passed."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "builder"), None, None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    let said = |i: usize| tool_result(&reqs[i], &format!("c{i}"));
    assert_eq!(said(1), NO_TESTS);
    assert_eq!(
        said(2),
        "1 of 1 tests failed:\n- tests/check.test.js › wrong: expected 5, got 0"
    );
    assert!(said(3).contains("app.json"), "{}", said(3));
    assert!(
        said(4).starts_with("the migrations failed on an empty database: "),
        "{}",
        said(4)
    );
    assert!(said(5).contains("'..' is not allowed"), "{}", said(5));
    assert_eq!(said(6), "the note must be 1 to 2000 characters");
    assert!(proposals(&t, &dot.id).await.is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_waiting_card_counts_in_what_needs_the_person() {
    let t = start().await;
    let folder = in_workspace(&t, "expenses", &expenses());
    let llm = Llm::start(dones(2)).await;
    let run = finished_run(&t, &llm, "builder").await;
    let before = t.rt.inbox_count().await.unwrap();
    t.rt.apps()
        .propose(&run.id, folder, "Tracks expenses.")
        .await
        .unwrap();
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.app_reviews.len(), 1);
    assert_eq!(inbox.app_reviews[0].dot_name, "builder");
    assert_eq!(inbox.app_reviews[0].proposal.note, "Tracks expenses.");
    assert_eq!(t.rt.inbox_count().await.unwrap(), before + 1);
    assert_eq!(t.rt.store().dot_counts(&run.dot_id).await.unwrap().0, 1);

    let p = proposals(&t, &run.dot_id).await.remove(0);
    t.rt.decide_app_proposal(&p.id, Decision::Discard)
        .await
        .unwrap();
    assert!(t.rt.inbox().await.unwrap().app_reviews.is_empty());
    assert_eq!(t.rt.store().dot_counts(&run.dot_id).await.unwrap().0, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn asking_for_changes_and_discarding_continue_the_dots_thread() {
    let t = start().await;
    let folder = in_workspace(&t, "expenses", &expenses());
    let llm = Llm::start(dones(4)).await;
    let run = finished_run(&t, &llm, "builder").await;
    let apps = t.rt.apps();
    assert_eq!(
        apps.propose(&run.id, folder.clone(), "First try.").await,
        Ok(SENT.to_string())
    );
    let p = proposals(&t, &run.dot_id).await.remove(0);
    let blank =
        t.rt.decide_app_proposal(&p.id, Decision::AskForChanges { text: "  ".into() })
            .await;
    assert!(matches!(blank, Err(Error::Invalid(_))), "{blank:?}");
    let (decided, next) =
        t.rt.decide_app_proposal(
            &p.id,
            Decision::AskForChanges {
                text: "Add a category.".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(decided.status, ProposalStatus::ChangesAsked);
    assert_eq!(decided.answer.as_deref(), Some("Add a category."));
    assert!(
        message(&next)
            .contains("The person asked for changes to the app expenses: Add a category."),
        "{next:?}"
    );
    assert_eq!(next.payload.as_ref().unwrap()["answer"], "Add a category.");
    assert!(!apps.proposed_dir(&p.id).exists());
    let next = wait_status(t.rt.store(), &next.id, RunStatus::Succeeded, 30).await;

    // The continuation proposes again, and the person discards it.
    apps.propose(&next.id, folder, "Second try.").await.unwrap();
    let p = proposals(&t, &run.dot_id).await.remove(0);
    assert_eq!(p.root_run_id, next.root_run_id);
    let (decided, last) =
        t.rt.decide_app_proposal(&p.id, Decision::Discard)
            .await
            .unwrap();
    assert_eq!(decided.status, ProposalStatus::Discarded);
    assert!(message(&last).contains(DISCARDED), "{last:?}");
    assert!(!apps.proposed_dir(&p.id).exists());
    assert!(apps.get("expenses").await.is_err());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_change_keeps_the_migrations_that_ran_and_waits_for_its_base_version() {
    let t = start().await;
    let apps = t.rt.apps();
    let first = in_workspace(&t, "v1", &expenses());
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    // The sample names its columns: it loads after the new migration too.
    let mut v2 = expenses_with(
        "sample.sql",
        Some(
            "INSERT INTO expenses (id, item, amount, on_day, added_at, added_by) VALUES \
             ('s1', 'Coffee', 4.5, '2026-10-22', '2026-10-22T09:00:00+05:30', 'you');\n",
        ),
    );
    v2.push((
        "migrations/0002_category.sql",
        "ALTER TABLE expenses ADD COLUMN category TEXT;\n".to_string(),
    ));
    let next = in_workspace(&t, "v2", &v2);
    let rewritten = in_workspace(
        &t,
        "rewritten",
        &expenses_with(
            "migrations/0001_expenses.sql",
            Some(&MIGRATION.replace("NOT NULL, on_day", "NOT NULL, note TEXT, on_day")),
        ),
    );
    let llm = Llm::start(dones(4)).await;
    let run = finished_run(&t, &llm, "builder").await;

    let refused = apps
        .propose(&run.id, rewritten, "Rewrite.")
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        "migrations/0001_expenses.sql differs from version 1's: keep the migrations that ran as \
         they are and add new ones after them"
    );
    apps.propose(&run.id, next.clone(), "Categories.")
        .await
        .unwrap();
    let p = proposals(&t, &run.dot_id).await.remove(0);
    assert_eq!(p.base_version, Some(1));
    assert!(p.app_id.is_some());
    assert_eq!(p.summary.update_from, Some(1));
    assert_eq!(p.summary.migrations, ["0002_category.sql"]);
    let (_, cont) =
        t.rt.decide_app_proposal(&p.id, Decision::Add)
            .await
            .unwrap();
    assert!(
        message(&cont)
            .contains("The person accepted your change to the app expenses; it is now version 2."),
        "{cont:?}"
    );
    let (_, config) = apps.get("expenses").await.unwrap();
    assert_eq!(config.version, 2);
    let cont = wait_status(t.rt.store(), &cont.id, RunStatus::Succeeded, 30).await;

    // A change checked against version 2 waits while the app moves on to 3: accepting it is
    // refused, and the card still waits.
    apps.propose(&cont.id, next.clone(), "Again.")
        .await
        .unwrap();
    let stale = proposals(&t, &run.dot_id).await.remove(0);
    apps.update("expenses", read_folder(&next).unwrap(), false)
        .await
        .unwrap();
    let late = t.rt.decide_app_proposal(&stale.id, Decision::Add).await;
    match late {
        Err(Error::Conflict(m)) => assert!(m.contains("changed after this was proposed"), "{m}"),
        other => panic!("expected a conflict, got {other:?}"),
    }
    let still = t.rt.store().get_app_proposal(&stale.id).await.unwrap();
    assert_eq!(still.status, ProposalStatus::Pending);
    // The failed accept left the thread unmarked: a chat to it goes through.
    let chat = t.rt.chat(&cont.dot_id, "Go on.", Some(&cont.id)).await;
    assert!(chat.is_ok(), "{chat:?}");
    let (_, config) = apps.get("expenses").await.unwrap();
    assert_eq!(config.version, 3);
    assert!(matches!(
        apps.get("expenses").await.unwrap().0.config,
        LibraryConfig::App(_)
    ));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_local_dot_copies_an_installed_app_into_its_workspace() {
    let t = start().await;
    let src = in_workspace(&t, "first", &expenses());
    t.rt.apps()
        .install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let llm = Llm::start(vec![
        llm_call("c1", "copy_app", json!({"app": "expenses", "path": "copy"})),
        llm_text("Copied."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "fixer"), None, None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    let names: Vec<&str> = reqs[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"copy_app"), "{names:?}");
    assert!(
        tool_result(&reqs[1], "c1").starts_with("Copied version 1 of the app expenses into copy."),
        "{}",
        tool_result(&reqs[1], "c1")
    );
    assert!(t.dir.path().join("copy").join("app.json").is_file());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_copies_an_installed_app_through_dots_own_server() {
    let script = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_call": {"server": "dots", "tool": "copy_app",
                      "arguments": {"app": "expenses", "path": "copy"}}},
        {"mcp_call": {"server": "dots", "tool": "copy_app",
                      "arguments": {"path": "other"}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    let t = start_with(|cfg, dir| {
        let path = dir.join("script.json");
        std::fs::write(&path, script.to_string()).unwrap();
        cfg.claude_env = vec![
            (
                "FAKE_CLAUDE_SCRIPT".into(),
                path.to_string_lossy().into_owned(),
            ),
            (
                "FAKE_CLAUDE_ARGS_OUT".into(),
                dir.join("args.json").to_string_lossy().into_owned(),
            ),
        ];
    })
    .await;
    let src = in_workspace(&t, "first", &expenses());
    t.rt.apps()
        .install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let mut s = spec("claude");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(t.dir.path().join("args.json")).unwrap())
            .unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        args.contains(&"mcp__dots__copy_app".to_string()),
        "{args:?}"
    );
    assert!(t.dir.path().join("copy").join("logic.js").is_file());
    // A call without `app` is refused before the gate: no approval is stored for the run.
    let results: Vec<Value> =
        t.rt.store()
            .list_events(&run.id, 0)
            .await
            .unwrap()
            .into_iter()
            .filter(|ev| ev.kind == "tool_result")
            .map(|ev| ev.data)
            .collect();
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[1]["is_error"], true, "{results:?}");
    assert!(
        results[1].to_string().contains("copy_app needs"),
        "{results:?}"
    );
    assert!(t
        .rt
        .store()
        .approvals_for_run(&run.id)
        .await
        .unwrap()
        .is_empty());
    assert!(!t.dir.path().join("other").exists());
    t.rt.shutdown();
}

#[tokio::test]
async fn while_an_update_migrates_its_thread_waits_and_other_threads_go_on() {
    let t = start().await;
    let first = in_workspace(&t, "first", &expenses());
    let apps = t.rt.apps();
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let ink = json!({ "item": "Ink", "amount": 3, "on": "2026-10-20" });
    apps.call(
        "expenses",
        "add_expense",
        ink,
        dots_core::apps::call::Actor::Person,
    )
    .await
    .unwrap();
    // Slow on real data only: the tests' empty databases seed no recursion.
    let slow = "CREATE TABLE pad AS WITH RECURSIVE c(x) AS (SELECT 1 FROM expenses UNION ALL \
                SELECT x + 1 FROM c WHERE x < 4000000) SELECT max(x) AS x FROM c;\n";
    let mut files = expenses();
    files.push(("migrations/0002_pad.sql", slow.to_string()));
    let next = in_workspace(&t, "next", &files);
    let llm = Llm::start(dones(6)).await;
    let fixer = finished_run(&t, &llm, "fixer").await;
    let other = finished_run(&t, &llm, "other").await;
    apps.propose(&fixer.id, next, "Pads.").await.unwrap();
    let p = proposals(&t, &fixer.dot_id).await.remove(0);
    let done = std::sync::atomic::AtomicBool::new(false);
    let decide = async {
        let decided = t.rt.decide_app_proposal(&p.id, Decision::Add).await;
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        decided
    };
    let meanwhile = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let same = t.rt.chat(&fixer.dot_id, "And?", Some(&fixer.id)).await;
        let elsewhere = t.rt.chat(&other.dot_id, "Hello", Some(&other.id)).await;
        (
            same,
            elsewhere,
            done.load(std::sync::atomic::Ordering::SeqCst),
        )
    };
    let (decided, (same, elsewhere, finished)) = tokio::join!(decide, meanwhile);
    assert!(
        !finished,
        "the migration outlasted the chats; make it slower"
    );
    match same {
        Err(Error::Conflict(m)) => assert_eq!(m, dots_core::checkins::DECIDING),
        other => panic!("{other:?}"),
    }
    assert!(elsewhere.is_ok(), "{elsewhere:?}");
    let (decided, _) = decided.unwrap();
    assert_eq!(decided.status, ProposalStatus::Added);
    let (_, config) = apps.get("expenses").await.unwrap();
    assert_eq!(config.version, 2);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_under_its_policy() {
    let script = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "propose_app",
                      "arguments": {"path": "expenses", "note": "Tracks expenses."}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    let t = start_with(|cfg, dir| {
        let path = dir.join("script.json");
        std::fs::write(&path, script.to_string()).unwrap();
        cfg.claude_env = vec![
            (
                "FAKE_CLAUDE_SCRIPT".into(),
                path.to_string_lossy().into_owned(),
            ),
            (
                "FAKE_CLAUDE_ARGS_OUT".into(),
                dir.join("args.json").to_string_lossy().into_owned(),
            ),
        ];
    })
    .await;
    in_workspace(&t, "expenses", &expenses());
    let claude_spec = |name: &str| {
        let mut s = spec(name);
        s.workdir = t.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.engine = EngineKind::Claude;
        s
    };
    let dot =
        t.rt.create_dot(claude_spec("claude"), None, None)
            .await
            .unwrap();
    let run = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(t.dir.path().join("args.json")).unwrap())
            .unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        args.contains(&"mcp__dots__propose_app".to_string()),
        "{args:?}"
    );
    let list = proposals(&t, &dot.id).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].note, "Tracks expenses.");

    // A deny rule wins over the preset's allow: the call is refused and nothing is proposed.
    let mut denying = claude_spec("denying");
    denying
        .policy
        .rules
        .insert(0, Rule::new("ProposeApp", Action::Deny));
    let denying = t.rt.create_dot(denying, None, None).await.unwrap();
    let run = t.rt.run_now(&denying.id).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await;
    let results: Vec<Value> =
        t.rt.store()
            .list_events(&run.id, 0)
            .await
            .unwrap()
            .into_iter()
            .filter(|ev| ev.kind == "tool_result")
            .map(|ev| ev.data)
            .collect();
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["is_error"], true);
    assert!(proposals(&t, &denying.id).await.is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_change_is_refused_for_an_app_replaced_since_it_was_checked() {
    let t = start().await;
    let apps = t.rt.apps();
    let first = in_workspace(&t, "v1", &expenses());
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let llm = Llm::start(dones(2)).await;
    let run = finished_run(&t, &llm, "builder").await;
    let next = in_workspace(&t, "v2", &expenses());
    apps.propose(&run.id, next, "Same again.").await.unwrap();
    let p = proposals(&t, &run.dot_id).await.remove(0);
    // The app is removed and another of the same name installed: version 1 again, a new id.
    apps.remove("expenses", true, true).await.unwrap();
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    match t.rt.decide_app_proposal(&p.id, Decision::Add).await {
        Err(Error::Conflict(m)) => {
            assert!(
                m.contains("was replaced after this change was proposed"),
                "{m}"
            )
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    let still = t.rt.store().get_app_proposal(&p.id).await.unwrap();
    assert_eq!(still.status, ProposalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn deleting_a_dot_deletes_its_proposals_files() {
    let t = start().await;
    let apps = t.rt.apps();
    let llm = Llm::start(dones(1)).await;
    let run = finished_run(&t, &llm, "builder").await;
    let folder = in_workspace(&t, "app", &expenses());
    apps.propose(&run.id, folder, "An app.").await.unwrap();
    let p = proposals(&t, &run.dot_id).await.remove(0);
    assert!(apps.proposed_dir(&p.id).exists());
    t.rt.delete_dot(&run.dot_id).await.unwrap();
    assert!(!apps.proposed_dir(&p.id).exists());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_dot_has_at_most_five_proposals_waiting() {
    let t = start().await;
    let apps = t.rt.apps();
    let llm = Llm::start(dones(6)).await;
    let first = finished_run(&t, &llm, "builder").await;
    let mut runs = vec![first.clone()];
    for _ in 0..5 {
        let run = t.rt.run_now(&first.dot_id).await.unwrap();
        runs.push(wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 30).await);
    }
    for (i, run) in runs.iter().enumerate() {
        let folder = in_workspace(&t, &format!("app{i}"), &expenses());
        let sent = apps.propose(&run.id, folder, "An app.").await;
        if i < 5 {
            sent.unwrap();
        } else {
            assert_eq!(
                sent.unwrap_err(),
                dots_core::apps::propose::TOO_MANY_PENDING
            );
        }
    }
    t.rt.shutdown();
}

#[test]
fn the_deciding_notice_keeps_its_exact_words() {
    assert_eq!(
        dots_core::checkins::DECIDING,
        "Dots is still adding the app you accepted in this chat; send your message again in a moment"
    );
}

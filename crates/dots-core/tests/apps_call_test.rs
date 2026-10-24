//! One call of an app's code (app library spec sections 5, 7 and 9).

mod common;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use dots_core::apps::call::{call, Actor, Call, CallError, DotRef, StartGate, MAX_DOT_RESULT};
use dots_core::apps::data;
use dots_core::apps::engine::{Failure, Limits};
use dots_core::apps::manifest::Manifest;
use dots_core::apps::source::Migration;
use rusqlite::Connection;
use serde_json::{json, Value};

const NOW: &str = "2026-10-23T09:12:00+05:30";

struct App {
    manifest: Manifest,
    logic: String,
    conn: Rc<RefCell<Connection>>,
    _dir: tempfile::TempDir,
}

/// The Expenses app on a fresh database, with `extra` added to its logic and a tool `f` listed
/// that takes any input and writes.
fn app(extra: &str) -> App {
    let mut manifest: Value = serde_json::from_str(common::apps::MANIFEST).unwrap();
    manifest["tools"].as_array_mut().unwrap().push(json!({
        "name": "f", "description": "test", "input": {"type": "object"}, "writes": true
    }));
    manifest["permissions"] = json!(["start_dots"]);
    manifest["on_run_end"] = json!("run_ended");
    let manifest = Manifest::parse(&manifest.to_string()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut conn = data::open(&dir.path().join("data.db")).unwrap();
    data::migrate(
        &mut conn,
        &[Migration {
            number: 1,
            file: "0001_expenses.sql".into(),
            sql: common::apps::MIGRATION.into(),
        }],
    )
    .unwrap();
    App {
        manifest,
        logic: format!(
            "{}\nexport function run_ended(e, ctx) {{ return e; }}\n{extra}",
            common::apps::LOGIC
        ),
        conn: Rc::new(RefCell::new(conn)),
        _dir: dir,
    }
}

fn dot() -> Actor {
    Actor::Dot {
        id: "d1".into(),
        name: "builder".into(),
        run_id: "r1".into(),
    }
}

fn run(
    app: &App,
    function: &str,
    input: Value,
    actor: Actor,
) -> Result<dots_core::apps::call::Outcome, CallError> {
    run_with(app, function, input, actor, None, Limits::default())
}

fn run_with(
    app: &App,
    function: &str,
    input: Value,
    actor: Actor,
    start: Option<StartGate>,
    limits: Limits,
) -> Result<dots_core::apps::call::Outcome, CallError> {
    call(
        &app.conn,
        Call {
            manifest: &app.manifest,
            logic: &app.logic,
            function,
            input: &input,
            actor,
            now: NOW.into(),
            dots: vec![DotRef {
                id: "d1".into(),
                name: "builder".into(),
            }],
            start,
            limits,
        },
    )
}

fn count(app: &App) -> usize {
    data::query(&app.conn.borrow(), "SELECT id FROM expenses", &[])
        .unwrap()
        .len()
}

#[test]
fn a_call_commits_its_writes_and_says_what_it_did() {
    let app = app("");
    let out = run(
        &app,
        "add_expense",
        json!({"item": "Coffee", "amount": 4.5, "on": "2026-10-22"}),
        dot(),
    )
    .unwrap();
    assert_eq!(out.ids.len(), 1);
    assert_eq!(out.result, json!({"id": out.ids[0]}));
    assert_eq!(out.changes, 1);
    assert!(out.starts.is_empty());
    let rows = data::query(
        &app.conn.borrow(),
        "SELECT added_at, added_by FROM expenses",
        &[],
    )
    .unwrap();
    assert_eq!(rows, vec![json!({"added_at": NOW, "added_by": "builder"})]);
    let total = run(
        &app,
        "month_total",
        json!({"month": "2026-10"}),
        Actor::Person,
    )
    .unwrap();
    assert_eq!(total.result, json!(4.5));
    assert_eq!(total.changes, 0);
}

#[test]
fn a_bad_input_changes_nothing() {
    let app = app("");
    let err = run(
        &app,
        "add_expense",
        json!({"item": "Coffee", "amount": "4.5", "on": "2026-10-22"}),
        dot(),
    )
    .unwrap_err();
    assert_eq!(
        err,
        CallError::BadInput("input.amount must be a number".into())
    );
    let err = run(
        &app,
        "add_expense",
        json!({"item": "Coffee", "amount": 1, "on": "x", "by": "me"}),
        dot(),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "input.by is not allowed");
    assert_eq!(count(&app), 0);
}

#[test]
fn only_listed_tools_can_be_called() {
    let app = app("export function hidden() { return 1; }");
    assert_eq!(
        run(&app, "hidden", json!({}), Actor::Person).unwrap_err(),
        CallError::UnknownTool("hidden".into())
    );
    // `on_run_end` is Dots' alone.
    assert_eq!(
        run(&app, "run_ended", json!({}), dot())
            .unwrap_err()
            .to_string(),
        "the app has no tool run_ended"
    );
    assert_eq!(
        run(&app, "run_ended", json!({"x": 1}), Actor::Dots)
            .unwrap()
            .result,
        json!({"x": 1})
    );
    assert!(run(
        &app,
        "month_total",
        json!({"month": "2026-10"}),
        Actor::Dots
    )
    .is_err());
}

#[test]
fn a_throw_after_a_write_rolls_it_back() {
    let app = app(r#"export function f(_, ctx) {
        ctx.db.exec("INSERT INTO expenses VALUES ('x', 'a', 1, '2026-10-01', 't', 'b')");
        throw new Error("changed my mind");
    }"#);
    assert_eq!(
        run(&app, "f", json!({}), dot()).unwrap_err(),
        CallError::Failed(Failure::Thrown("Error: changed my mind".into()))
    );
    assert_eq!(count(&app), 0);
}

#[test]
fn a_result_that_is_not_json_rolls_back() {
    let app = app(r#"export function f(_, ctx) {
        ctx.db.exec("INSERT INTO expenses VALUES ('x', 'a', 1, '2026-10-01', 't', 'b')");
        return () => 1;
    }"#);
    assert_eq!(
        run(&app, "f", json!({}), dot()).unwrap_err(),
        CallError::Failed(Failure::NotJson)
    );
    assert_eq!(count(&app), 0);
}

#[test]
fn a_timeout_rolls_back() {
    let app = app(r#"export function f(_, ctx) {
        ctx.db.exec("INSERT INTO expenses VALUES ('x', 'a', 1, '2026-10-01', 't', 'b')");
        for (;;) {}
    }"#);
    let limits = Limits {
        time: Duration::from_millis(200),
        ..Limits::default()
    };
    assert_eq!(
        run_with(&app, "f", json!({}), dot(), None, limits).unwrap_err(),
        CallError::Failed(Failure::TimedOut)
    );
    assert_eq!(count(&app), 0);
    // The database is usable after.
    run(
        &app,
        "add_expense",
        json!({"item": "a", "amount": 1, "on": "2026-10-01"}),
        dot(),
    )
    .unwrap();
    assert_eq!(count(&app), 1);
}

#[test]
fn a_result_too_large_for_a_dot_rolls_back_but_fits_the_screen() {
    let app = app(r#"export function f({ n }, ctx) {
        ctx.db.exec("INSERT INTO expenses VALUES (?, 'a', 1, '2026-10-01', 't', 'b')", [ctx.id()]);
        return "x".repeat(n);
    }"#);
    let err = run(&app, "f", json!({"n": MAX_DOT_RESULT}), dot()).unwrap_err();
    assert_eq!(
        err,
        CallError::TooLarge {
            bytes: MAX_DOT_RESULT + 2,
            limit: MAX_DOT_RESULT
        }
    );
    assert_eq!(count(&app), 0);
    run(&app, "f", json!({"n": MAX_DOT_RESULT}), Actor::Person).unwrap();
    assert_eq!(count(&app), 1);
}

#[test]
fn the_app_sees_who_calls_and_when() {
    let app = app("export function f(_, ctx) { return { actor: ctx.actor, now: ctx.now }; }");
    assert_eq!(
        run(&app, "f", json!({}), dot()).unwrap().result,
        json!({"actor": {"kind": "dot", "id": "d1", "name": "builder", "run_id": "r1"}, "now": NOW})
    );
    assert_eq!(
        run(&app, "f", json!({}), Actor::Person).unwrap().result["actor"],
        json!({"kind": "person"})
    );
}

#[test]
fn the_apps_sql_rules_hold_inside_a_call() {
    let app = app(r#"export function f({ sql }, ctx) { ctx.db.exec(sql); }"#);
    for sql in ["ATTACH 'C:/x.db' AS x", "COMMIT", "PRAGMA user_version = 3"] {
        let err = run(&app, "f", json!({"sql": sql}), dot())
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("Error: SQL: not authorized"),
            "{sql}: {err}"
        );
    }
    assert_eq!(data::schema_version(&app.conn.borrow()).unwrap(), 1);
}

#[test]
fn an_sql_error_can_be_caught_by_the_app() {
    let app = app(r#"export function f(_, ctx) {
        try { ctx.db.query("SELECT nope FROM expenses"); } catch (e) { return e.message; }
    }"#);
    assert_eq!(
        run(&app, "f", json!({}), dot()).unwrap().result,
        json!("SQL: no such column: nope")
    );
}

#[test]
fn dots_are_listed_but_start_needs_the_grant() {
    let app = app("export function f(_, ctx) { return [ctx.dots.list(), typeof ctx.dots.start]; }");
    assert_eq!(
        run(&app, "f", json!({}), Actor::Person).unwrap().result,
        json!([[{"id": "d1", "name": "builder"}], "undefined"])
    );
    let gate = StartGate {
        open: BTreeMap::new(),
        budget: 20,
    };
    assert_eq!(
        run_with(
            &app,
            "f",
            json!({}),
            Actor::Person,
            Some(gate),
            Limits::default()
        )
        .unwrap()
        .result[1],
        json!("function")
    );
}

#[test]
fn a_start_is_recorded_and_its_run_id_returned() {
    let app = app(r#"export function f({ tag }, ctx) {
        return [ctx.dots.start("d1", "Work on " + tag, tag), ctx.dots.start("d1", "again", tag)];
    }"#);
    let gate = StartGate {
        open: BTreeMap::new(),
        budget: 20,
    };
    let out = run_with(
        &app,
        "f",
        json!({"tag": "t1"}),
        Actor::Person,
        Some(gate),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(out.starts.len(), 1);
    let s = &out.starts[0];
    assert_eq!(
        (s.dot_id.as_str(), s.message.as_str(), s.tag.as_str()),
        ("d1", "Work on t1", "t1")
    );
    assert_eq!(out.result, json!([s.run_id, s.run_id]));
}

#[test]
fn an_open_topic_is_returned_instead_of_a_new_start() {
    let app = app(r#"export function f(_, ctx) { return ctx.dots.start("d1", "go", "t1"); }"#);
    let gate = StartGate {
        open: BTreeMap::from([(("d1".to_string(), "t1".to_string()), "root9".to_string())]),
        budget: 20,
    };
    let out = run_with(
        &app,
        "f",
        json!({}),
        Actor::Person,
        Some(gate),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(out.result, json!("root9"));
    assert!(out.starts.is_empty());
}

#[test]
fn a_start_breaking_a_rule_throws() {
    let app = app(r#"export function f({ dot, message, tag }, ctx) {
        try { return ctx.dots.start(dot, message, tag); } catch (e) { return e.message; }
    }"#);
    let go = |input: Value, budget: u32| {
        let gate = StartGate {
            open: BTreeMap::new(),
            budget,
        };
        run_with(
            &app,
            "f",
            input,
            Actor::Person,
            Some(gate),
            Limits::default(),
        )
        .unwrap()
        .result
    };
    assert_eq!(
        go(json!({"dot": "d9", "message": "m", "tag": "t"}), 20),
        json!("no dot d9 uses this app")
    );
    assert_eq!(
        go(json!({"dot": "d1", "message": " ", "tag": "t"}), 20),
        json!("the message must be 1 to 4000 characters")
    );
    assert_eq!(
        go(json!({"dot": "d1", "message": "m", "tag": ""}), 20),
        json!("the tag must be 1 to 200 characters")
    );
    assert_eq!(
        go(json!({"dot": "d1", "message": "m", "tag": "t"}), 0),
        json!("this app has started as many dots as the hour allows; try again later")
    );
}

#[test]
fn a_rolled_back_call_starts_nothing() {
    let app = app(
        r#"export function f(_, ctx) { ctx.dots.start("d1", "go", "t1"); throw new Error("no"); }"#,
    );
    let gate = StartGate {
        open: BTreeMap::new(),
        budget: 20,
    };
    assert!(run_with(
        &app,
        "f",
        json!({}),
        Actor::Person,
        Some(gate),
        Limits::default()
    )
    .is_err());
}

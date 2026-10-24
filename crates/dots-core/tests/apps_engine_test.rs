//! The sandbox apps' logic runs in (app library spec sections 5 and 9).

use std::time::Duration;

use dots_core::apps::engine::{function, run, Failure, Limits};
use rquickjs::{Ctx, Object};
use serde_json::{json, Value};

fn empty_ctx<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    Object::new(ctx.clone())
}

fn call(source: &str, function: &str, input: Value) -> Result<Value, Failure> {
    run(source, function, &input, Limits::default(), empty_ctx)
}

#[test]
fn an_export_gets_the_input_and_ctx_and_returns_json() {
    let source = "export function add({ a, b }, ctx) { return { sum: a + b, now: ctx.now }; }";
    let out = run(
        source,
        "add",
        &json!({"a": 2, "b": 3}),
        Limits::default(),
        |ctx| {
            let o = Object::new(ctx.clone())?;
            o.set("now", "2026-10-23T09:00:00+05:30")?;
            Ok(o)
        },
    )
    .unwrap();
    assert_eq!(out, json!({"sum": 5, "now": "2026-10-23T09:00:00+05:30"}));
}

#[test]
fn a_rust_function_on_ctx_can_be_called() {
    let source = "export function f(_, ctx) { return ctx.id() + ctx.id(); }";
    let out = run(source, "f", &json!({}), Limits::default(), |ctx| {
        let o = Object::new(ctx.clone())?;
        o.set("id", function(ctx, || "x".to_string())?)?;
        Ok(o)
    })
    .unwrap();
    assert_eq!(out, json!("xx"));
}

#[test]
fn the_clock_and_random_numbers_are_refused() {
    for (expr, what) in [
        (
            "Date.now()",
            "Date.now is not available in an app: use ctx.now",
        ),
        (
            "new Date()",
            "new Date() is not available in an app: use new Date(ctx.now)",
        ),
        ("Date()", "Date() is not available in an app: use ctx.now"),
        (
            "Math.random()",
            "Math.random is not available in an app: use ctx.id()",
        ),
        (
            "new (Object.getPrototypeOf(new Date(0)).constructor)()",
            "new Date() is not available in an app: use new Date(ctx.now)",
        ),
    ] {
        let source = format!("export function f() {{ return {expr}; }}");
        assert_eq!(
            call(&source, "f", json!({})),
            Err(Failure::Thrown(format!("Error: {what}"))),
            "{expr}"
        );
    }
}

#[test]
fn a_date_from_a_value_still_works() {
    let source = r#"export function f() {
        return [new Date("2026-10-23T09:00:00Z").toISOString(), Date.UTC(2026, 9, 23),
                Date.parse("2026-10-23T00:00:00Z")];
    }"#;
    assert_eq!(
        call(source, "f", json!({})).unwrap(),
        json!([
            "2026-10-23T09:00:00.000Z",
            1792713600000i64,
            1792713600000i64
        ])
    );
}

#[test]
fn nothing_outside_is_reachable() {
    let source = r#"export function f() {
        return [typeof fetch, typeof require, typeof std, typeof os, typeof process,
                typeof XMLHttpRequest, typeof setTimeout];
    }"#;
    assert_eq!(
        call(source, "f", json!({})).unwrap(),
        Value::Array(vec![json!("undefined"); 7])
    );
}

#[test]
fn an_import_is_a_load_failure() {
    let source = "import { x } from './other.js'; export function f() { return x; }";
    assert!(matches!(
        call(source, "f", json!({})),
        Err(Failure::Load(_))
    ));
}

#[test]
fn a_syntax_error_is_a_load_failure() {
    assert!(matches!(
        call("export function f( {", "f", json!({})),
        Err(Failure::Load(_))
    ));
}

#[test]
fn a_missing_export_is_named() {
    assert_eq!(
        call("export function f() { return 1; }", "g", json!({})),
        Err(Failure::NoFunction("g".into()))
    );
    assert_eq!(
        call("export const g = 1;", "g", json!({})),
        Err(Failure::NoFunction("g".into()))
    );
}

#[test]
fn a_throw_is_reported_with_its_text() {
    assert_eq!(
        call(
            "export function f() { throw new TypeError('bad task'); }",
            "f",
            json!({})
        ),
        Err(Failure::Thrown("TypeError: bad task".into()))
    );
    assert_eq!(
        call("export function f() { throw 'plain'; }", "f", json!({})),
        Err(Failure::Thrown("plain".into()))
    );
}

#[test]
fn a_promise_is_run_to_its_end() {
    let source = "export async function f({ n }) { await null; return n * 2; }";
    assert_eq!(call(source, "f", json!({"n": 21})).unwrap(), json!(42));
    let source = "export async function f() { await null; throw new Error('later'); }";
    assert_eq!(
        call(source, "f", json!({})),
        Err(Failure::Thrown("Error: later".into()))
    );
    let source = "export function f() { return new Promise(() => {}); }";
    assert_eq!(
        call(source, "f", json!({})),
        Err(Failure::Thrown("the call's promise never settled".into()))
    );
}

#[test]
fn a_result_that_is_not_json_is_refused() {
    assert_eq!(
        call("export function f() { return () => 1; }", "f", json!({})),
        Err(Failure::NotJson)
    );
    assert_eq!(
        call(
            "export function f() { const a = {}; a.a = a; return a; }",
            "f",
            json!({})
        ),
        Err(Failure::NotJson)
    );
    assert_eq!(
        call("export function f() { return 1n; }", "f", json!({})),
        Err(Failure::NotJson)
    );
    assert_eq!(
        call("export function f() {}", "f", json!({})).unwrap(),
        Value::Null
    );
}

#[test]
fn a_runaway_loop_times_out() {
    let limits = Limits {
        time: Duration::from_millis(200),
        ..Limits::default()
    };
    let source = "export function f() { for (;;) {} }";
    assert_eq!(
        run(source, "f", &json!({}), limits, empty_ctx),
        Err(Failure::TimedOut)
    );
    // A catch in the app does not stop the timeout.
    let source = "export function f() { try { for (;;) {} } catch (e) { return 'caught'; } }";
    assert_eq!(
        run(source, "f", &json!({}), limits, empty_ctx),
        Err(Failure::TimedOut)
    );
}

#[test]
fn a_memory_hog_runs_out_of_memory() {
    let limits = Limits {
        memory: 8 * 1024 * 1024,
        ..Limits::default()
    };
    let source = "export function f() { const a = []; for (;;) a.push('x'.repeat(1024)); }";
    assert_eq!(
        run(source, "f", &json!({}), limits, empty_ctx),
        Err(Failure::OutOfMemory)
    );
}

#[test]
fn each_call_starts_fresh() {
    let source =
        "globalThis.count = (globalThis.count || 0) + 1; export function f() { return count; }";
    assert_eq!(call(source, "f", json!({})).unwrap(), json!(1));
    assert_eq!(call(source, "f", json!({})).unwrap(), json!(1));
}

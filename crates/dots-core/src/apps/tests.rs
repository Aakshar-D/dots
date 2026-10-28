//! An app's tests (app library spec section 9, item 4): each `tests/*.test.js` runs as a script
//! in the sandbox with `test(name, fn)`, `assert` and `call(tool, input, options)`. Each test gets
//! a fresh database with the app's migrations applied, and `call` goes through the real call
//! path: the schema check, the transaction, the limits.
//!
//! `options` of `call`: `actor` (`{ kind: "person" }`, the default; `{ kind: "dot", id, name,
//! run_id? }`; `{ kind: "dots" }` for `on_run_end`), `now` (the call's `ctx.now`), `dots` (what
//! `ctx.dots.list()` gives) and `can_start` (`ctx.dots.start` is there). `started()` lists the
//! starts the test's committed calls asked for.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{Ctx, Function, Value as JsValue};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::call::{self, Actor, Call, DotRef, Start, StartGate, STARTS_PER_HOUR};
use super::data;
use super::engine::{self, Failure, Limits, MEMORY_LIMIT};
use super::manifest::Manifest;
use super::source::{AppSource, Migration, TESTS_DIR};

/// The longest one test file may run, its calls included.
pub const FILE_TIME_LIMIT: Duration = Duration::from_secs(10);
/// The longest an app's test files may run together: a file past it does not run.
pub const TOTAL_TIME_LIMIT: Duration = Duration::from_secs(60);
/// The most tests one file may hold.
pub const MAX_TESTS: usize = 500;
/// `ctx.now` of a call whose options name none.
pub const DEFAULT_NOW: &str = "2026-01-01T09:00:00+00:00";
/// The longest test name and error a report keeps, in characters.
pub const MAX_NAME: usize = 200;
pub const MAX_ERROR: usize = 1000;
/// The most failures `TestReport::problem` names.
const MAX_SHOWN: usize = 20;
/// What a proposal without a test is told (spec section 9, item 4).
pub const NO_TESTS: &str = "the app has no tests: add tests/<name>.test.js with at least one \
    test(name, fn)";

/// The test API, defined before a test file runs. It evaluates to `[tests, describe]`, which Rust
/// holds: the tests registered, each `[name, fn]`, and the text of a thrown value. The globals
/// are fixed, `assert` is frozen, and `__call` and `__started` are taken away, so a test file
/// cannot reach the runner or replace what it judges by (spec section 9, item 4).
const PRELUDE: &str = r#"
(() => {
  const tests = [];
  const native_call = globalThis.__call, native_started = globalThis.__started;
  delete globalThis.__call;
  delete globalThis.__started;
  const Str = String, stringify = JSON.stringify, parse = JSON.parse;
  const show = (v) => { try { return stringify(v) ?? Str(v); } catch { return Str(v); } };
  const canon = (v) => Array.isArray(v) ? v.map(canon)
    : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map(k => [k, canon(v[k])]))
    : v;
  const fail = (message, fallback) => { throw new Error(message ?? fallback); };
  const text = (e) => e && e.message !== undefined ? (e.name && e.name !== "Error" ? `${e.name}: ${e.message}` : e.message) : Str(e);
  const describe = (e) => {
    try { const said = text(e); return typeof said === "string" ? said : Str(said); }
    catch { return "the test failed"; }
  };
  const fixed = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: false, configurable: false });
  fixed("test", (name, fn) => {
    if (typeof name !== "string" || typeof fn !== "function") throw new Error("test(name, fn) takes a name and a function");
    tests[tests.length] = [name, fn];
  });
  fixed("assert", Object.freeze({
    ok: (value, message) => { if (!value) fail(message, `expected a true value, got ${show(value)}`); },
    equal: (actual, expected, message) => {
      if (actual !== expected) fail(message, `expected ${show(expected)}, got ${show(actual)}`);
    },
    deepEqual: (actual, expected, message) => {
      if (show(canon(actual)) !== show(canon(expected))) fail(message, `expected ${show(expected)}, got ${show(actual)}`);
    },
    throws: (fn, expected, message) => {
      try { fn(); } catch (e) {
        const said = describe(e);
        if (expected === undefined) return;
        if (expected instanceof RegExp ? expected.test(said) : said.includes(String(expected))) return;
        fail(message, `expected an error with ${show(String(expected))}, got ${show(said)}`);
      }
      fail(message, "expected the call to throw");
    },
  }));
  fixed("call", (tool, input, options) =>
    parse(native_call(Str(tool), stringify(input ?? {}), stringify(options ?? {}))));
  fixed("started", () => parse(native_started()));
  return [tests, describe];
})()
"#;

/// One test's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    /// The test file, `tests/<name>.test.js`.
    pub file: String,
    /// The test's name; empty for a file that could not run.
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// Every test file's results, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestReport {
    pub results: Vec<TestResult>,
}

impl TestReport {
    /// There is at least one test, and every test passed.
    pub fn passed(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.passed)
    }

    /// What the dot is told when the report does not pass; `None` when it does.
    pub fn problem(&self) -> Option<String> {
        if self.results.is_empty() {
            return Some(NO_TESTS.to_string());
        }
        let failed: Vec<String> = self
            .results
            .iter()
            .filter(|r| !r.passed)
            .take(MAX_SHOWN)
            .map(|r| {
                let what = if r.name.is_empty() {
                    r.file.clone()
                } else {
                    format!("{} › {}", r.file, r.name)
                };
                format!("- {what}: {}", r.error.as_deref().unwrap_or("failed"))
            })
            .collect();
        let count = self.results.iter().filter(|r| !r.passed).count();
        (count > 0).then(|| {
            let more = if count > MAX_SHOWN {
                format!("\n… and {} more", count - MAX_SHOWN)
            } else {
                String::new()
            };
            format!(
                "{count} of {} tests failed:\n{}{more}",
                self.results.len(),
                failed.join("\n")
            )
        })
    }
}

/// Runs every test file of `src` (spec section 9, item 4). Blocks: call it from
/// `spawn_blocking`.
pub fn run_tests(src: &AppSource) -> TestReport {
    run_tests_bounded(src, FILE_TIME_LIMIT, TOTAL_TIME_LIMIT)
}

/// `run_tests`, each file bounded by `limit` (tests).
pub fn run_tests_within(src: &AppSource, limit: Duration) -> TestReport {
    run_tests_bounded(src, limit, TOTAL_TIME_LIMIT)
}

/// `run_tests`, each file bounded by `limit` and all of them by `total`: a file starts with
/// what is left of `total`, and once nothing is left the files after it fail without running.
pub fn run_tests_bounded(src: &AppSource, limit: Duration, total: Duration) -> TestReport {
    let started = Instant::now();
    let mut results = Vec::new();
    for (name, code) in &src.tests {
        let file = format!("{TESTS_DIR}/{name}");
        let left = total.saturating_sub(started.elapsed());
        if left.is_zero() {
            results.push(TestResult {
                file: file.clone(),
                name: String::new(),
                passed: false,
                error: Some(format!(
                    "{file} did not run: the app's tests ran past {} s in all",
                    total.as_secs_f32()
                )),
            });
            continue;
        }
        // A file that starts with less than its own limit is stopped by the total, and says so.
        let cut = (left < limit).then_some(total);
        results.extend(run_file(src, &file, code, limit.min(left), cut));
    }
    TestReport { results }
}

/// What the test file's calls share: the current test's database and the starts its calls asked
/// for.
#[derive(Default)]
struct Bench {
    conn: Option<Rc<RefCell<Connection>>>,
    starts: Vec<Start>,
}

fn run_file(
    src: &AppSource,
    file: &str,
    code: &str,
    limit: Duration,
    cut_by: Option<Duration>,
) -> Vec<TestResult> {
    let failed = |name: &str, error: String| TestResult {
        file: file.to_string(),
        name: bounded(name.to_string(), MAX_NAME),
        passed: false,
        error: Some(bounded(error, MAX_ERROR)),
    };
    let bench = Rc::new(RefCell::new(Bench::default()));
    let manifest = Rc::new(src.manifest.clone());
    let logic = Rc::new(src.logic.clone());
    let limits = Limits {
        time: limit,
        memory: MEMORY_LIMIT,
    };
    let mut results = Vec::new();
    let ran = engine::with_sandbox(limits, |ctx, judge| {
        let load = |e| judge.failure(ctx, e, Failure::Load);
        // `__call` and `__started` first: the prelude takes them into its closure.
        define_call(ctx, &bench, &manifest, &logic).map_err(load)?;
        let held: rquickjs::Array = ctx.eval(PRELUDE).map_err(load)?;
        let tests: rquickjs::Array = held.get(0).map_err(load)?;
        let describe: Function = held.get(1).map_err(load)?;
        ctx.eval::<(), _>(code).map_err(load)?;
        if tests.len() > MAX_TESTS {
            return Err(Failure::Load(format!(
                "a file may hold at most {MAX_TESTS} tests"
            )));
        }
        let began = Instant::now();
        let mut names = Vec::new();
        for i in 0..tests.len() {
            let entry: rquickjs::Array = tests.get(i).map_err(load)?;
            let name: String = entry.get(0).map_err(load)?;
            let test: Function = entry.get(1).map_err(load)?;
            let name = bounded(name, MAX_NAME);
            // The file's time covers each test's migrations too.
            let left = limit
                .checked_sub(began.elapsed())
                .ok_or(Failure::TimedOut)?;
            match fresh_database(&src.migrations, left) {
                Ok(conn) => {
                    *bench.borrow_mut() = Bench {
                        conn: Some(Rc::new(RefCell::new(conn))),
                        starts: Vec::new(),
                    };
                }
                Err(e) => {
                    results.push(failed(&name, format!("the migrations failed: {e}")));
                    names.push(name);
                    continue;
                }
            }
            // The verdict is Rust's: a test passes when its function returns, or the promise it
            // returns resolves; whatever it throws or rejects with fails it.
            let error = match test.call::<_, JsValue>(()) {
                Ok(value) => match value.as_promise().cloned() {
                    None => None,
                    Some(promise) => match promise.finish::<JsValue>() {
                        Ok(_) => None,
                        Err(rquickjs::Error::WouldBlock) => {
                            Some("the test's promise never settled".to_string())
                        }
                        Err(e) => Some(said(ctx, judge, &describe, judge.caught(ctx, e)?)?),
                    },
                },
                Err(e) => Some(said(ctx, judge, &describe, judge.caught(ctx, e)?)?),
            };
            results.push(TestResult {
                file: file.to_string(),
                name: name.clone(),
                passed: error.is_none(),
                error: error.map(|e| bounded(e, MAX_ERROR)),
            });
            names.push(name);
        }
        // A `test()` called while the tests ran registers a test that would never run.
        if tests.len() != names.len() {
            results.push(failed(
                "",
                "test() was called while the tests ran: call it at the top of the file".into(),
            ));
        }
        Ok(names)
    });
    bench.borrow_mut().conn = None;
    match ran {
        Ok(_) => results,
        // A file that could not load has no tests to name.
        Err(Failure::Load(e)) if results.is_empty() => {
            vec![failed("", format!("{file} could not be loaded: {e}"))]
        }
        // Every test the file did not finish fails with what stopped it.
        Err(stop) => {
            let stop = match stop {
                Failure::TimedOut => match cut_by {
                    Some(total) => {
                        format!("the app's tests ran past {} s in all", total.as_secs_f32())
                    }
                    None => format!("the tests ran over {} s", limit.as_secs_f32()),
                },
                other => other.to_string(),
            };
            let done = results.len();
            results.push(failed(
                "",
                format!("{file} stopped after {done} tests: {stop}"),
            ));
            results
        }
    }
}

/// What a failed test says: the prelude's text of what it threw, or "the test failed".
/// A timeout or a full memory while describing it stops the file, as anywhere else.
fn said<'js>(
    ctx: &Ctx<'js>,
    judge: &engine::Judge,
    describe: &Function<'js>,
    thrown: JsValue<'js>,
) -> Result<String, Failure> {
    match describe.call::<_, String>((thrown,)) {
        Ok(text) => Ok(text),
        Err(e) => match judge.caught(ctx, e) {
            Err(stop @ (Failure::TimedOut | Failure::OutOfMemory)) => Err(stop),
            _ => Ok("the test failed".to_string()),
        },
    }
}

/// `text` cut to `max` characters, the cut marked.
fn bounded(text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text,
    }
}

/// A database with the migrations applied, within `time`.
fn fresh_database(migrations: &[Migration], time: Duration) -> crate::Result<Connection> {
    let mut conn = data::open_in_memory()?;
    data::migrate_within(&mut conn, migrations, time.min(data::SETUP_TIME))?;
    Ok(conn)
}

/// `__call(tool, input, options)` and `__started()`, which `call` and `started` wrap.
fn define_call<'js>(
    ctx: &Ctx<'js>,
    bench: &Rc<RefCell<Bench>>,
    manifest: &Rc<Manifest>,
    logic: &Rc<String>,
) -> rquickjs::Result<()> {
    let (b, manifest, logic) = (bench.clone(), manifest.clone(), logic.clone());
    let call_fn = engine::function(
        ctx,
        move |ctx: Ctx<'js>, tool: String, input: String, options: String| {
            let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
            let options: Value = serde_json::from_str(&options).unwrap_or_else(|_| json!({}));
            let actor = actor_of(&options).map_err(|e| engine::throw(&ctx, &e))?;
            let now = options["now"].as_str().unwrap_or(DEFAULT_NOW).to_string();
            let dots = options["dots"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|d| DotRef {
                            id: d["id"].as_str().unwrap_or_default().to_string(),
                            name: d["name"].as_str().unwrap_or_default().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            // As the switch gives it: only to an app that asks for `start_dots`, with the
            // topics this test's earlier calls opened still open.
            let start = (options["can_start"] == true && manifest.can_start_dots()).then(|| {
                let made = &b.borrow().starts;
                StartGate {
                    open: made
                        .iter()
                        .map(|s| ((s.dot_id.clone(), s.tag.clone()), s.run_id.clone()))
                        .collect::<BTreeMap<_, _>>(),
                    budget: STARTS_PER_HOUR.saturating_sub(made.len() as u32),
                    paused: false,
                }
            });
            let Some(conn) = b.borrow().conn.clone() else {
                return Err(engine::throw(&ctx, "call() works inside a test"));
            };
            let out = call::call(
                &conn,
                Call {
                    manifest: &manifest,
                    logic: &logic,
                    function: &tool,
                    input: &input,
                    actor,
                    now,
                    dots,
                    start,
                    limits: Limits::default(),
                },
            )
            .map_err(|e| engine::throw(&ctx, &e.to_string()))?;
            b.borrow_mut().starts.extend(out.starts);
            Ok::<String, rquickjs::Error>(out.result.to_string())
        },
    )?;
    ctx.globals().set("__call", call_fn)?;
    let b = bench.clone();
    let started = engine::function(ctx, move || {
        let list: Vec<Value> = b
            .borrow()
            .starts
            .iter()
            .map(|s| json!({ "run_id": s.run_id, "dot_id": s.dot_id, "message": s.message, "tag": s.tag }))
            .collect();
        Value::Array(list).to_string()
    })?;
    ctx.globals().set("__started", started)
}

/// The actor a call's options name: the person when they name none.
fn actor_of(options: &Value) -> Result<Actor, String> {
    let actor = &options["actor"];
    let field = |key: &str| actor[key].as_str().map(str::to_string);
    match actor["kind"].as_str() {
        None | Some("person") => Ok(Actor::Person),
        Some("dots") => Ok(Actor::Dots),
        Some("dot") => Ok(Actor::Dot {
            id: field("id").ok_or("a dot actor needs an id")?,
            name: field("name").ok_or("a dot actor needs a name")?,
            run_id: field("run_id").unwrap_or_else(|| "test-run".to_string()),
        }),
        Some(other) => Err(format!(
            "actor kind {other:?}: use \"person\", \"dot\" or \"dots\""
        )),
    }
}

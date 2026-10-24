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
use std::time::Duration;

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
/// The most tests one file may hold.
pub const MAX_TESTS: usize = 500;
/// `ctx.now` of a call whose options name none.
pub const DEFAULT_NOW: &str = "2026-01-01T09:00:00+00:00";
/// What a proposal without a test is told (spec section 9, item 4).
pub const NO_TESTS: &str = "the app has no tests: add tests/<name>.test.js with at least one \
    test(name, fn)";

/// The test API, defined before a test file runs.
const PRELUDE: &str = r#"
(() => {
  const tests = [];
  const show = (v) => { try { return JSON.stringify(v) ?? String(v); } catch { return String(v); } };
  const canon = (v) => Array.isArray(v) ? v.map(canon)
    : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map(k => [k, canon(v[k])]))
    : v;
  const fail = (message, fallback) => { throw new Error(message ?? fallback); };
  const text = (e) => e && e.message !== undefined ? (e.name && e.name !== "Error" ? `${e.name}: ${e.message}` : e.message) : String(e);
  globalThis.test = (name, fn) => {
    if (typeof name !== "string" || typeof fn !== "function") throw new Error("test(name, fn) takes a name and a function");
    tests.push([name, fn]);
  };
  globalThis.assert = {
    ok: (value, message) => { if (!value) fail(message, `expected a true value, got ${show(value)}`); },
    equal: (actual, expected, message) => {
      if (actual !== expected) fail(message, `expected ${show(expected)}, got ${show(actual)}`);
    },
    deepEqual: (actual, expected, message) => {
      if (show(canon(actual)) !== show(canon(expected))) fail(message, `expected ${show(expected)}, got ${show(actual)}`);
    },
    throws: (fn, expected, message) => {
      try { fn(); } catch (e) {
        const said = text(e);
        if (expected === undefined) return;
        if (expected instanceof RegExp ? expected.test(said) : said.includes(String(expected))) return;
        fail(message, `expected an error with ${show(String(expected))}, got ${show(said)}`);
      }
      fail(message, "expected the call to throw");
    },
  };
  globalThis.call = (tool, input, options) =>
    JSON.parse(__call(String(tool), JSON.stringify(input ?? {}), JSON.stringify(options ?? {})));
  globalThis.started = () => JSON.parse(__started());
  globalThis.__names = () => JSON.stringify(tests.map(t => t[0]));
  globalThis.__run = (i) => {
    try {
      const r = tests[i][1]();
      if (r && typeof r.then === "function") return r.then(() => null, text);
      return null;
    } catch (e) {
      return text(e);
    }
  };
})();
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
            .map(|r| {
                let what = if r.name.is_empty() {
                    r.file.clone()
                } else {
                    format!("{} › {}", r.file, r.name)
                };
                format!("- {what}: {}", r.error.as_deref().unwrap_or("failed"))
            })
            .collect();
        (!failed.is_empty()).then(|| {
            format!(
                "{} of {} tests failed:\n{}",
                failed.len(),
                self.results.len(),
                failed.join("\n")
            )
        })
    }
}

/// Runs every test file of `src` (spec section 9, item 4). Blocks: call it from
/// `spawn_blocking`.
pub fn run_tests(src: &AppSource) -> TestReport {
    run_tests_within(src, FILE_TIME_LIMIT)
}

/// `run_tests`, each file bounded by `limit` (tests).
pub fn run_tests_within(src: &AppSource, limit: Duration) -> TestReport {
    let mut results = Vec::new();
    for (name, code) in &src.tests {
        let file = format!("{TESTS_DIR}/{name}");
        results.extend(run_file(src, &file, code, limit));
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

fn run_file(src: &AppSource, file: &str, code: &str, limit: Duration) -> Vec<TestResult> {
    let failed = |name: &str, error: String| TestResult {
        file: file.to_string(),
        name: name.to_string(),
        passed: false,
        error: Some(error),
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
        ctx.eval::<(), _>(PRELUDE).map_err(load)?;
        define_call(ctx, &bench, &manifest, &logic).map_err(load)?;
        ctx.eval::<(), _>(code).map_err(load)?;
        let names: String = global(ctx, "__names")
            .and_then(|f| f.call(()))
            .map_err(load)?;
        let names: Vec<String> = serde_json::from_str(&names).unwrap_or_default();
        if names.len() > MAX_TESTS {
            return Err(Failure::Load(format!(
                "a file may hold at most {MAX_TESTS} tests"
            )));
        }
        for (i, name) in names.iter().enumerate() {
            match fresh_database(&src.migrations) {
                Ok(conn) => {
                    *bench.borrow_mut() = Bench {
                        conn: Some(Rc::new(RefCell::new(conn))),
                        starts: Vec::new(),
                    };
                }
                Err(e) => {
                    results.push(failed(name, format!("the migrations failed: {e}")));
                    continue;
                }
            }
            let thrown = |e| judge.failure(ctx, e, Failure::Thrown);
            let mut outcome: JsValue = global(ctx, "__run")
                .and_then(|f| f.call((i,)))
                .map_err(thrown)?;
            if let Some(promise) = outcome.as_promise().cloned() {
                outcome = match promise.finish::<JsValue>() {
                    Ok(v) => v,
                    Err(rquickjs::Error::WouldBlock) => {
                        results.push(failed(name, "the test's promise never settled".into()));
                        continue;
                    }
                    Err(e) => return Err(thrown(e)),
                };
            }
            let error = outcome.as_string().and_then(|s| s.to_string().ok());
            results.push(TestResult {
                file: file.to_string(),
                name: name.clone(),
                passed: error.is_none(),
                error,
            });
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
                Failure::TimedOut => format!("the tests ran over {} s", limit.as_secs_f32()),
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

fn global<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Function<'js>> {
    ctx.globals().get(name)
}

fn fresh_database(migrations: &[Migration]) -> crate::Result<Connection> {
    let mut conn = data::open_in_memory()?;
    data::migrate(&mut conn, migrations)?;
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

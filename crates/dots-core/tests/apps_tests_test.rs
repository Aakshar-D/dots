//! An app's tests (app library spec section 9, item 4): `test`, `assert` and `call` in the
//! sandbox, a fresh database per test, failures named, and a proposal without tests refused.

mod common;

use std::time::Duration;

use common::apps::{expenses, expenses_with, write_app, MANIFEST};
use dots_core::apps::source::{read_folder, AppSource};
use dots_core::apps::tests::{
    run_tests, run_tests_bounded, run_tests_within, TestReport, NO_TESTS,
};

fn source(files: &[(&str, String)]) -> AppSource {
    let dir = tempfile::tempdir().unwrap();
    write_app(dir.path(), files);
    read_folder(dir.path()).unwrap()
}

/// Expenses with `tests/check.test.js` holding `code` in place of its own tests.
fn expenses_tested(code: &str) -> AppSource {
    let mut files = expenses_with("tests/expenses.test.js", None);
    files.push(("tests/check.test.js", code.to_string()));
    source(&files)
}

fn outcomes(report: &TestReport) -> Vec<(String, bool)> {
    report
        .results
        .iter()
        .map(|r| (r.name.clone(), r.passed))
        .collect()
}

#[test]
fn the_expenses_tests_pass_through_the_real_call_path() {
    let report = run_tests(&source(&expenses()));
    assert_eq!(
        outcomes(&report),
        [
            ("a month's total adds its expenses".to_string(), true),
            ("a dot's expense names the dot".to_string(), true),
        ],
        "{report:?}"
    );
    assert!(report.passed());
    assert_eq!(report.problem(), None);
    assert_eq!(report.results[0].file, "tests/expenses.test.js");
}

#[test]
fn each_test_gets_a_fresh_database() {
    let report = run_tests(&expenses_tested(
        r#"
test("one", () => { call("add_expense", { item: "A", amount: 1, on: "2026-10-01" }); });
test("two", () => { assert.deepEqual(call("list_expenses", { month: "2026-10" }), []); });
"#,
    ));
    assert!(report.passed(), "{report:?}");
}

#[test]
fn a_failing_test_is_named_with_what_it_expected() {
    let report = run_tests(&expenses_tested(
        r#"
test("right", () => assert.equal(call("month_total", { month: "2026-10" }), 0));
test("wrong", () => assert.equal(call("month_total", { month: "2026-10" }), 5));
test("deep", () => assert.deepEqual({ b: 1, a: [2] }, { a: [2], b: 1 }));
"#,
    ));
    assert_eq!(
        outcomes(&report),
        [
            ("right".to_string(), true),
            ("wrong".to_string(), false),
            ("deep".to_string(), true),
        ]
    );
    assert!(!report.passed());
    assert_eq!(
        report.results[1].error.as_deref(),
        Some("expected 5, got 0")
    );
    assert_eq!(
        report.problem().unwrap(),
        "1 of 3 tests failed:\n- tests/check.test.js › wrong: expected 5, got 0"
    );
}

#[test]
fn a_call_the_app_refuses_throws_and_changes_nothing() {
    let report = run_tests(&expenses_tested(
        r#"
test("bad input", () => {
  assert.throws(() => call("add_expense", { item: "Tea" }), "amount is required");
  assert.throws(() => call("nope", {}), /no tool nope/);
  assert.deepEqual(call("list_expenses", { month: "2026-10" }), []);
});
test("throws expects a throw", () => {
  assert.throws(() => call("month_total", { month: "2026-10" }));
});
"#,
    ));
    assert_eq!(
        outcomes(&report),
        [
            ("bad input".to_string(), true),
            ("throws expects a throw".to_string(), false),
        ],
        "{report:?}"
    );
    assert_eq!(
        report.results[1].error.as_deref(),
        Some("expected the call to throw")
    );
}

#[test]
fn an_app_without_tests_does_not_pass() {
    let report = run_tests(&source(&expenses_with("tests/expenses.test.js", None)));
    assert!(report.results.is_empty());
    assert!(!report.passed());
    assert_eq!(report.problem().as_deref(), Some(NO_TESTS));
    // A file that defines no test counts as none.
    let report = run_tests(&expenses_tested("const x = 1;\n"));
    assert_eq!(report.problem().as_deref(), Some(NO_TESTS));
}

#[test]
fn a_file_that_cannot_load_fails_by_its_name() {
    let report = run_tests(&expenses_tested("test(\"x\", () => {\n"));
    assert_eq!(report.results.len(), 1);
    let r = &report.results[0];
    assert_eq!(r.name, "");
    assert!(!r.passed);
    let error = r.error.as_deref().unwrap();
    assert!(
        error.starts_with("tests/check.test.js could not be loaded: "),
        "{error}"
    );
}

#[test]
fn a_test_that_runs_forever_is_stopped() {
    let src = expenses_tested(
        r#"
test("quick", () => {});
test("forever", () => { for (;;) {} });
test("never reached", () => {});
"#,
    );
    let report = run_tests_within(&src, Duration::from_millis(300));
    assert_eq!(report.results.len(), 2, "{report:?}");
    assert!(report.results[0].passed);
    let error = report.results[1].error.as_deref().unwrap();
    assert_eq!(
        error,
        "tests/check.test.js stopped after 1 tests: the tests ran over 0.3 s"
    );
    assert!(!report.passed());
}

#[test]
fn the_test_files_together_are_bounded_too() {
    let forever = "test(\"forever\", () => { for (;;) {} });\n".to_string();
    let mut files = expenses_with("tests/expenses.test.js", None);
    for name in ["tests/a.test.js", "tests/b.test.js", "tests/c.test.js"] {
        files.push((name, forever.clone()));
    }
    let report = run_tests_bounded(
        &source(&files),
        Duration::from_millis(300),
        Duration::from_millis(450),
    );
    let second = report
        .results
        .iter()
        .find(|r| r.file == "tests/b.test.js")
        .unwrap();
    let error = second.error.as_deref().unwrap();
    assert!(error.contains("in all"), "{error}");
    assert!(error.contains("0.45 s"), "{error}");
    let last = report.results.last().unwrap();
    assert_eq!(last.file, "tests/c.test.js", "{report:?}");
    assert_eq!(
        last.error.as_deref(),
        Some("tests/c.test.js did not run: the app's tests ran past 0.45 s in all")
    );
    assert!(!report.passed());
}

#[test]
fn tests_see_no_clock_or_random_numbers() {
    let report = run_tests(&expenses_tested(
        r#"
test("no clock", () => { assert.throws(() => Date.now(), "use ctx.now"); });
test("no random", () => { assert.throws(() => Math.random(), "use ctx.id()"); });
"#,
    ));
    assert!(report.passed(), "{report:?}");
}

#[test]
fn a_test_can_call_as_a_dot_and_see_the_starts_it_asked_for() {
    let manifest = MANIFEST
        .replace(
            r#""tools": ["#,
            r#""permissions": ["start_dots"],
  "tools": [
    {
      "name": "assign",
      "description": "Start a dot on the books",
      "input": { "type": "object", "properties": { "dot": { "type": "string" } }, "required": ["dot"] },
      "writes": true
    },"#,
        );
    let logic = format!(
        "{}\nexport function assign({{ dot }}, ctx) {{ return ctx.dots.start(dot, \"Check the books.\", \"books\"); }}\n",
        common::apps::LOGIC
    );
    let mut files = expenses_with("tests/expenses.test.js", None);
    files.retain(|(p, _)| *p != "app.json" && *p != "logic.js");
    files.push(("app.json", manifest));
    files.push(("logic.js", logic));
    files.push((
        "tests/start.test.js",
        r#"
const dots = [{ id: "d1", name: "builder" }];
test("assigning starts the dot once", () => {
  const run = call("assign", { dot: "d1" }, { can_start: true, dots });
  assert.equal(call("assign", { dot: "d1" }, { can_start: true, dots }), run);
  assert.deepEqual(started().map(s => [s.dot_id, s.tag, s.run_id]), [["d1", "books", run]]);
});
test("a dot that does not use the app is not started", () => {
  assert.throws(() => call("assign", { dot: "d9" }, { can_start: true, dots }), "no dot d9 uses this app");
  assert.deepEqual(started(), []);
});
test("without the switch there is no start", () => {
  assert.throws(() => call("assign", { dot: "d1" }, { dots }), /not a function/);
});
test("a bad actor is refused", () => {
  assert.throws(() => call("month_total", { month: "2026-10" }, { actor: { kind: "robot" } }), "actor kind");
});
"#
        .to_string(),
    ));
    let report = run_tests(&source(&files));
    assert!(report.passed(), "{report:?}");
    assert_eq!(report.results.len(), 4);
}

#[test]
fn a_test_file_cannot_fake_a_pass() {
    for code in [
        // Replacing the runner's hooks and the API does nothing.
        "globalThis.__names = () => '[\"fake\"]'; globalThis.__run = () => null;\n\
         test(\"real\", () => assert.equal(1, 2));",
        "try { assert.equal = () => {}; } catch {}\ntest(\"real\", () => assert.equal(1, 2));",
        "try { globalThis.assert = { equal() {} }; } catch {}\n\
         test(\"real\", () => assert.equal(1, 2));",
        // A throw that is not an error still fails.
        "test(\"real\", () => { throw { message: null }; });",
        "test(\"real\", () => { throw 5; });",
        // So does a rejected promise.
        "test(\"real\", async () => { throw new Error(\"no\"); });",
    ] {
        let report = run_tests(&expenses_tested(code));
        assert_eq!(report.results.len(), 1, "{code}: {report:?}");
        assert!(!report.passed(), "{code}: {report:?}");
        assert_eq!(report.results[0].name, "real", "{code}");
    }
}

#[test]
fn a_report_says_a_bounded_amount() {
    let report = run_tests(&expenses_tested(
        "for (let i = 0; i < 30; i++) \
         test(\"t\" + i + \"x\".repeat(500), () => { throw new Error(\"e\".repeat(100000)); });",
    ));
    assert_eq!(report.results.len(), 30);
    for r in &report.results {
        assert!(r.name.chars().count() <= dots_core::apps::tests::MAX_NAME + 1);
        assert!(r.error.as_ref().unwrap().chars().count() <= dots_core::apps::tests::MAX_ERROR + 1);
    }
    assert!(report.problem().unwrap().ends_with("… and 10 more"));
}

#[test]
fn an_error_that_runs_over_stops_the_file() {
    let report = run_tests_within(
        &expenses_tested(
            "test(\"a\", () => { throw { get message() { while (true) {} } }; });\n\
             test(\"b\", () => {});",
        ),
        std::time::Duration::from_millis(500),
    );
    assert!(!report.passed());
    assert!(
        report.results.iter().all(|r| r.name != "b" || !r.passed),
        "{report:?}"
    );
    let last = report.results.last().unwrap().error.as_deref().unwrap();
    assert!(last.contains("ran over"), "{report:?}");
}

#[test]
fn a_test_added_while_the_tests_run_fails_the_file() {
    let report = run_tests(&expenses_tested(
        "test(\"a\", () => { test(\"b\", () => {}); });",
    ));
    assert!(!report.passed(), "{report:?}");
    assert_eq!(
        report.results.last().unwrap().error.as_deref(),
        Some("test() was called while the tests ran: call it at the top of the file")
    );
}

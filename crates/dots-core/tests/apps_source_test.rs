//! An app's folder, read and checked (app library spec sections 4 and 6.2).

mod common;

use common::apps::{expenses, expenses_with, write_app};
use dots_core::apps::source::read_folder;

fn read(files: &[(&str, String)]) -> Result<dots_core::apps::source::AppSource, String> {
    let dir = tempfile::tempdir().unwrap();
    write_app(dir.path(), files);
    read_folder(dir.path()).map_err(|e| e.to_string())
}

#[test]
fn the_expenses_app_reads_whole() {
    let app = read(&expenses()).unwrap();
    assert_eq!(app.manifest.name, "expenses");
    assert!(app.logic.contains("export function month_total"));
    assert_eq!(app.ui.keys().collect::<Vec<_>>(), ["app.js", "index.html"]);
    assert_eq!(app.migrations.len(), 1);
    assert_eq!(app.migrations[0].number, 1);
    assert_eq!(app.migrations[0].file, "0001_expenses.sql");
    assert_eq!(app.schema_version(), 1);
    assert!(app.sample.is_some());
    assert_eq!(app.tests.len(), 1);
    assert_eq!(app.tests[0].0, "expenses.test.js");
    assert_eq!(app.readme, None);
    let files: Vec<String> = app.files().into_keys().collect();
    assert_eq!(
        files,
        [
            "app.json",
            "logic.js",
            "migrations/0001_expenses.sql",
            "sample.sql",
            "tests/expenses.test.js",
            "ui/app.js",
            "ui/index.html"
        ]
    );
}

#[test]
fn a_missing_part_is_named() {
    assert_eq!(
        read(&expenses_with("logic.js", None)).unwrap_err(),
        "invalid input: the app has no logic.js"
    );
    assert_eq!(
        read(&expenses_with("ui/index.html", None)).unwrap_err(),
        "invalid input: the app has no ui/index.html"
    );
    assert_eq!(
        read(&expenses_with("app.json", None)).unwrap_err(),
        "invalid input: the app has no app.json"
    );
}

#[test]
fn a_file_outside_the_layout_is_refused() {
    let mut files = expenses();
    files.push(("notes.txt", "x".into()));
    assert_eq!(
        read(&files).unwrap_err(),
        "invalid input: notes.txt: an app holds app.json, logic.js, ui/, migrations/, \
         sample.sql, tests/ and README.md only"
    );
}

#[test]
fn hidden_files_are_refused() {
    let mut files = expenses();
    files.push(("ui/.env", "SECRET=1".into()));
    assert_eq!(
        read(&files).unwrap_err(),
        "invalid input: ui/.env: hidden files are not allowed in an app"
    );
}

#[test]
fn migrations_are_named_and_numbered_in_order() {
    let mut files = expenses();
    files.push(("migrations/init.sql", "SELECT 1;".into()));
    assert!(read(&files)
        .unwrap_err()
        .contains("migrations/init.sql: name migrations NNNN_name.sql"));
    let mut files = expenses();
    files.push(("migrations/0003_more.sql", "SELECT 1;".into()));
    assert_eq!(
        read(&files).unwrap_err(),
        "invalid input: migrations/0003_more.sql: migrations are numbered 0001, 0002, … with \
         no gap or repeat"
    );
    let mut files = expenses();
    files.push(("migrations/0001_again.sql", "SELECT 1;".into()));
    assert!(read(&files).unwrap_err().contains("no gap or repeat"));
}

#[test]
fn tests_are_named_test_js() {
    let mut files = expenses();
    files.push(("tests/helpers.js", "".into()));
    assert_eq!(
        read(&files).unwrap_err(),
        "invalid input: tests/helpers.js: tests are tests/<name>.test.js"
    );
}

#[test]
fn logic_must_load_and_export_every_tool() {
    let err = read(&expenses_with("logic.js", Some("export function ("))).unwrap_err();
    assert!(
        err.starts_with("invalid input: logic.js: the app's code could not be loaded"),
        "{err}"
    );
    let only_two =
        common::apps::LOGIC.replace("export function month_total", "function month_total");
    assert_eq!(
        read(&expenses_with("logic.js", Some(&only_two))).unwrap_err(),
        "invalid input: logic.js exports no function month_total"
    );
}

#[test]
fn on_run_end_must_be_exported() {
    let mut manifest: serde_json::Value = serde_json::from_str(common::apps::MANIFEST).unwrap();
    manifest["permissions"] = serde_json::json!(["start_dots"]);
    manifest["on_run_end"] = serde_json::json!("run_ended");
    let err = read(&expenses_with("app.json", Some(&manifest.to_string()))).unwrap_err();
    assert_eq!(err, "invalid input: logic.js exports no function run_ended");
}

#[test]
fn the_size_is_bounded() {
    let mut files = expenses();
    files.push(("ui/big.js", "x".repeat(2 * 1024 * 1024)));
    assert_eq!(
        read(&files).unwrap_err(),
        "invalid input: the app's files hold more than 2 MB"
    );
}

#[test]
fn a_path_that_is_not_a_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_folder(&dir.path().join("missing"))
        .unwrap_err()
        .to_string();
    assert!(err.ends_with("is not a folder"), "{err}");
}

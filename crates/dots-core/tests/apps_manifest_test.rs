//! `app.json` (app library spec section 4).

mod common;

use dots_core::apps::manifest::{
    check_app_name, check_tool_name, split_tool_key, tool_key, Manifest, Permission, MAX_APP_NAME,
    MAX_TOOL_NAME,
};
use dots_core::Error;
use serde_json::{json, Value};

fn parse(v: Value) -> Result<Manifest, String> {
    Manifest::parse(&v.to_string()).map_err(|e| match e {
        Error::Invalid(m) => m,
        other => panic!("not Invalid: {other}"),
    })
}

fn base() -> Value {
    serde_json::from_str(common::apps::MANIFEST).unwrap()
}

fn with(key: &str, value: Value) -> Value {
    let mut m = base();
    m[key] = value;
    m
}

fn with_tool(key: &str, value: Value) -> Value {
    let mut m = base();
    m["tools"][0][key] = value;
    m
}

#[test]
fn the_expenses_manifest_reads() {
    let m = Manifest::parse(common::apps::MANIFEST).unwrap();
    assert_eq!(m.name, "expenses");
    assert_eq!(m.title, "Expenses");
    let names: Vec<&str> = m.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["add_expense", "list_expenses", "month_total"]);
    assert!(m.tool("add_expense").unwrap().writes);
    assert!(!m.tool("month_total").unwrap().writes);
    assert!(m.permissions.is_empty());
    assert!(!m.can_start_dots());
    assert_eq!(m.on_run_end, None);
}

#[test]
fn a_tool_is_offered_as_app_and_tool() {
    assert_eq!(tool_key("projects", "get_task"), "projects__get_task");
    assert_eq!(
        split_tool_key("projects__get_task"),
        Some(("projects", "get_task"))
    );
    // The longest names still fit a Claude tool name, `mcp__dots__<app>__<tool>`.
    let longest = format!(
        "mcp__dots__{}",
        tool_key(&"a".repeat(MAX_APP_NAME), &"b".repeat(MAX_TOOL_NAME))
    );
    assert_eq!(longest.len(), 64);
}

#[test]
fn names_follow_the_rule() {
    for ok in ["projects", "q4_invoices", "a", "a1"] {
        check_app_name(ok).unwrap();
        check_tool_name(ok).unwrap();
    }
    for bad in [
        "", "Projects", "1st", "my__app", "app_", "_app", "my-app", "my app",
    ] {
        assert!(check_app_name(bad).is_err(), "{bad:?}");
        assert!(check_tool_name(bad).is_err(), "{bad:?}");
    }
    assert!(check_app_name(&"a".repeat(MAX_APP_NAME + 1)).is_err());
    assert!(check_tool_name(&"a".repeat(MAX_TOOL_NAME)).is_ok());
    assert!(check_tool_name(&"a".repeat(MAX_TOOL_NAME + 1)).is_err());
}

#[test]
fn an_unknown_key_is_refused() {
    let err = parse(with("version", json!(3))).unwrap_err();
    assert!(err.contains("unknown field `version`"), "{err}");
    let err = parse(with_tool("returns", json!({}))).unwrap_err();
    assert!(err.contains("unknown field `returns`"), "{err}");
}

#[test]
fn a_bad_app_name_is_refused() {
    let err = parse(with("name", json!("My App"))).unwrap_err();
    assert!(err.starts_with("app name \"My App\""), "{err}");
}

#[test]
fn the_title_and_descriptions_are_bounded() {
    assert_eq!(
        parse(with("title", json!(" "))).unwrap_err(),
        "app.json: title must be 1 to 60 characters"
    );
    assert!(parse(with("title", json!("t".repeat(61)))).is_err());
    assert_eq!(
        parse(with("description", json!("d".repeat(301)))).unwrap_err(),
        "app.json: description must be at most 300 characters"
    );
    assert_eq!(
        parse(with_tool("description", json!(""))).unwrap_err(),
        "app.json: tool add_expense: description must be 1 to 1000 characters"
    );
}

#[test]
fn a_tool_listed_twice_is_refused() {
    let mut m = base();
    let first = m["tools"][0].clone();
    m["tools"].as_array_mut().unwrap().push(first);
    assert_eq!(
        parse(m).unwrap_err(),
        "app.json: the tool add_expense is listed twice"
    );
}

#[test]
fn a_tool_input_must_be_an_object_schema_in_the_subset() {
    assert_eq!(
        parse(with_tool("input", json!({"type": "string"}))).unwrap_err(),
        "app.json: tool add_expense: input: the schema's type must be \"object\""
    );
    assert_eq!(
        parse(with_tool(
            "input",
            json!({"type": "object", "properties": {"x": {"type": "string", "pattern": "^a"}}})
        ))
        .unwrap_err(),
        "app.json: tool add_expense: input.x: the keyword \"pattern\" is not supported"
    );
}

#[test]
fn only_start_dots_may_be_asked_for() {
    let m = parse(with("permissions", json!(["start_dots"]))).unwrap();
    assert_eq!(m.permissions, [Permission::StartDots]);
    assert!(m.can_start_dots());
    let err = parse(with("permissions", json!(["network"]))).unwrap_err();
    assert!(err.contains("unknown variant `network`"), "{err}");
}

#[test]
fn on_run_end_needs_start_dots_and_is_not_a_tool() {
    assert_eq!(
        parse(with("on_run_end", json!("run_ended"))).unwrap_err(),
        "app.json: on_run_end needs the permission \"start_dots\""
    );
    let mut m = with("permissions", json!(["start_dots"]));
    m["on_run_end"] = json!("month_total");
    assert_eq!(
        parse(m).unwrap_err(),
        "app.json: on_run_end \"month_total\" is also a tool; Dots alone calls it"
    );
    let mut m = with("permissions", json!(["start_dots"]));
    m["on_run_end"] = json!("run-ended");
    assert_eq!(
        parse(m).unwrap_err(),
        "app.json: on_run_end \"run-ended\" is not a function name"
    );
}

#[test]
fn a_large_manifest_is_refused() {
    let big = with("description", json!("x".repeat(40_000))).to_string();
    let err = Manifest::parse(&big).unwrap_err().to_string();
    assert!(err.contains("the limit is 32768"), "{err}");
}

//! The input check every app call goes through (app library spec sections 4 and 9, item 2).

use dots_core::apps::schema::{check_input_schema, validate};
use serde_json::{json, Value};

fn task_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string", "minLength": 1, "maxLength": 5 },
            "status": { "enum": ["todo", "done"] },
            "points": { "type": "integer", "minimum": 1, "maximum": 8 },
            "due_on": { "type": ["string", "null"] },
            "tags": { "type": "array", "items": { "type": "string" }, "maxItems": 2 },
            "owner": {
                "type": "object",
                "properties": { "kind": { "type": "string" } },
                "required": ["kind"],
                "additionalProperties": false
            }
        },
        "required": ["title"]
    })
}

fn check(v: Value) -> Result<(), String> {
    validate(&task_schema(), &v)
}

#[test]
fn the_schema_is_accepted() {
    check_input_schema(&task_schema()).unwrap();
}

#[test]
fn a_good_input_passes() {
    check(
        json!({"title": "Ship", "status": "done", "points": 3, "due_on": null,
                 "tags": ["a"], "owner": {"kind": "dot"}, "extra": 1}),
    )
    .unwrap();
}

#[test]
fn each_rule_names_the_place() {
    for (input, error) in [
        (json!({}), "input.title is required"),
        (json!({"title": 7}), "input.title must be a string"),
        (
            json!({"title": ""}),
            "input.title must be at least 1 characters",
        ),
        (
            json!({"title": "éééééé"}),
            "input.title must be at most 5 characters",
        ),
        (
            json!({"title": "a", "status": "doing"}),
            "input.status must be one of \"todo\", \"done\"",
        ),
        (
            json!({"title": "a", "points": 3.5}),
            "input.points must be an integer",
        ),
        (
            json!({"title": "a", "points": 0}),
            "input.points must be at least 1",
        ),
        (
            json!({"title": "a", "points": 9}),
            "input.points must be at most 8",
        ),
        (
            json!({"title": "a", "due_on": 5}),
            "input.due_on must be a string or null",
        ),
        (
            json!({"title": "a", "tags": ["x", 1]}),
            "input.tags[1] must be a string",
        ),
        (
            json!({"title": "a", "tags": ["x", "y", "z"]}),
            "input.tags must have at most 2 items",
        ),
        (
            json!({"title": "a", "owner": {}}),
            "input.owner.kind is required",
        ),
        (
            json!({"title": "a", "owner": {"kind": "dot", "id": 1}}),
            "input.owner.id is not allowed",
        ),
        (json!([]), "input must be an object"),
    ] {
        assert_eq!(check(input.clone()), Err(error.to_string()), "{input}");
    }
}

#[test]
fn a_whole_float_is_an_integer() {
    check(json!({"title": "a", "points": 3.0})).unwrap();
}

#[test]
fn a_schema_outside_the_subset_is_refused() {
    for (schema, error) in [
        (
            json!({"type": "object", "oneOf": []}),
            "input: the keyword \"oneOf\" is not supported",
        ),
        (
            json!({"type": "thing"}),
            "input: type must be one of string, integer, number, boolean, array, object, null",
        ),
        (
            json!({"type": "object", "required": "title"}),
            "input: required must be a list of names",
        ),
        (
            json!({"type": "object", "additionalProperties": {}}),
            "input: additionalProperties must be true or false",
        ),
        (
            json!({"type": "object", "properties": {"a": {"maxLength": -1}}}),
            "input.a: maxLength must be a whole number",
        ),
        (
            json!({"type": "object", "properties": {"a": {"enum": []}}}),
            "input.a: enum must be a list of values",
        ),
        (
            json!({"type": "array"}),
            "input: the schema's type must be \"object\"",
        ),
        (json!("object"), "input: a schema must be an object"),
    ] {
        assert_eq!(
            check_input_schema(&schema),
            Err(error.to_string()),
            "{schema}"
        );
    }
}

/// A schema of `levels` levels: each level but the last is an object whose `a` is the next level.
fn nested(levels: usize) -> Value {
    let mut schema = json!({"type": "string"});
    for _ in 1..levels {
        schema = json!({"type": "object", "properties": {"a": schema}});
    }
    schema
}

#[test]
fn schemas_nest_at_most_16_deep() {
    check_input_schema(&nested(16)).unwrap();
    assert_eq!(
        check_input_schema(&nested(17)),
        Err("input.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a: schemas nest at most 16 deep".to_string())
    );
}

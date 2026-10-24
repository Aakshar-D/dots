//! The JSON Schema subset an app's tool inputs are written in, and the check every call's input
//! goes through before the app's code runs (app library spec sections 4 and 9, item 2).
//!
//! Keywords: `type` (`string`, `integer`, `number`, `boolean`, `array`, `object`, `null`, or a
//! list of them), `properties`, `required`, `additionalProperties` (a boolean), `items`, `enum`,
//! `minLength`, `maxLength`, `minimum`, `maximum`, `minItems`, `maxItems`, and the notes
//! `description`, `title` and `default`, which nothing checks.

use serde_json::{Map, Value};

const TYPES: &[&str] = &[
    "string", "integer", "number", "boolean", "array", "object", "null",
];
const KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
    "description",
    "title",
    "default",
];
/// How deep a schema may nest.
const MAX_DEPTH: usize = 8;

/// Checks that `schema` is a tool's input schema: an object schema in the subset above. `Err`
/// names the place and the problem.
pub fn check_input_schema(schema: &Value) -> Result<(), String> {
    check(schema, "input", 0)?;
    if schema.get("type") != Some(&Value::String("object".into())) {
        return Err("input: the schema's type must be \"object\"".into());
    }
    Ok(())
}

fn check(schema: &Value, at: &str, depth: usize) -> Result<(), String> {
    let Some(map) = schema.as_object() else {
        return Err(format!("{at}: a schema must be an object"));
    };
    if depth > MAX_DEPTH {
        return Err(format!("{at}: schemas nest at most {MAX_DEPTH} deep"));
    }
    if let Some(key) = map.keys().find(|k| !KEYWORDS.contains(&k.as_str())) {
        return Err(format!("{at}: the keyword {key:?} is not supported"));
    }
    if let Some(t) = map.get("type") {
        let ok = match t {
            Value::String(s) => TYPES.contains(&s.as_str()),
            Value::Array(list) => {
                !list.is_empty()
                    && list
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| TYPES.contains(&s)))
            }
            _ => false,
        };
        if !ok {
            return Err(format!("{at}: type must be one of {}", TYPES.join(", ")));
        }
    }
    if let Some(props) = map.get("properties") {
        let Some(props) = props.as_object() else {
            return Err(format!("{at}: properties must be an object"));
        };
        for (name, sub) in props {
            check(sub, &format!("{at}.{name}"), depth + 1)?;
        }
    }
    if let Some(req) = map.get("required") {
        let ok = req
            .as_array()
            .is_some_and(|l| l.iter().all(Value::is_string));
        if !ok {
            return Err(format!("{at}: required must be a list of names"));
        }
    }
    if let Some(extra) = map.get("additionalProperties") {
        if !extra.is_boolean() {
            return Err(format!("{at}: additionalProperties must be true or false"));
        }
    }
    if let Some(items) = map.get("items") {
        check(items, &format!("{at}[]"), depth + 1)?;
    }
    if let Some(e) = map.get("enum") {
        if e.as_array().is_none_or(|l| l.is_empty()) {
            return Err(format!("{at}: enum must be a list of values"));
        }
    }
    for key in ["minLength", "maxLength", "minItems", "maxItems"] {
        if let Some(n) = map.get(key) {
            if n.as_u64().is_none() {
                return Err(format!("{at}: {key} must be a whole number"));
            }
        }
    }
    for key in ["minimum", "maximum"] {
        if let Some(n) = map.get(key) {
            if !n.is_number() {
                return Err(format!("{at}: {key} must be a number"));
            }
        }
    }
    Ok(())
}

/// Checks `value` against a schema `check_input_schema` accepted. `Err` is the first problem,
/// naming the place: `input.status must be one of "todo", "done"`.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    validate_at(schema, value, "input")
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_type(value: &Value, t: &str) -> bool {
    match t {
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(n) => {
                n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            _ => false,
        },
        other => type_name(value) == other,
    }
}

fn validate_at(schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let empty = Map::new();
    let map = schema.as_object().unwrap_or(&empty);
    if let Some(t) = map.get("type") {
        let types: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(l) => l.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !types.iter().any(|t| is_type(value, t)) {
            return Err(format!("{at} must be {}", a_type(&types)));
        }
    }
    if let Some(Value::Array(allowed)) = map.get("enum") {
        if !allowed.contains(value) {
            let list: Vec<String> = allowed.iter().map(Value::to_string).collect();
            return Err(format!("{at} must be one of {}", list.join(", ")));
        }
    }
    match value {
        Value::String(s) => {
            let len = s.chars().count() as u64;
            if let Some(min) = map.get("minLength").and_then(Value::as_u64) {
                if len < min {
                    return Err(format!("{at} must be at least {min} characters"));
                }
            }
            if let Some(max) = map.get("maxLength").and_then(Value::as_u64) {
                if len > max {
                    return Err(format!("{at} must be at most {max} characters"));
                }
            }
        }
        Value::Number(n) => {
            let n = n.as_f64().unwrap_or_default();
            if let Some(min) = map.get("minimum").and_then(Value::as_f64) {
                if n < min {
                    return Err(format!("{at} must be at least {min}"));
                }
            }
            if let Some(max) = map.get("maximum").and_then(Value::as_f64) {
                if n > max {
                    return Err(format!("{at} must be at most {max}"));
                }
            }
        }
        Value::Array(items) => {
            let len = items.len() as u64;
            if let Some(min) = map.get("minItems").and_then(Value::as_u64) {
                if len < min {
                    return Err(format!("{at} must have at least {min} items"));
                }
            }
            if let Some(max) = map.get("maxItems").and_then(Value::as_u64) {
                if len > max {
                    return Err(format!("{at} must have at most {max} items"));
                }
            }
            if let Some(item) = map.get("items") {
                for (i, v) in items.iter().enumerate() {
                    validate_at(item, v, &format!("{at}[{i}]"))?;
                }
            }
        }
        Value::Object(fields) => {
            let props = map.get("properties").and_then(Value::as_object);
            if let Some(Value::Array(req)) = map.get("required") {
                for name in req.iter().filter_map(Value::as_str) {
                    if !fields.contains_key(name) {
                        return Err(format!("{at}.{name} is required"));
                    }
                }
            }
            for (name, v) in fields {
                match props.and_then(|p| p.get(name)) {
                    Some(sub) => validate_at(sub, v, &format!("{at}.{name}"))?,
                    None if map.get("additionalProperties") == Some(&Value::Bool(false)) => {
                        return Err(format!("{at}.{name} is not allowed"));
                    }
                    None => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn a_type(types: &[&str]) -> String {
    let named: Vec<String> = types
        .iter()
        .map(|t| match *t {
            "integer" | "array" | "object" => format!("an {t}"),
            "null" => "null".to_string(),
            other => format!("a {other}"),
        })
        .collect();
    named.join(" or ")
}

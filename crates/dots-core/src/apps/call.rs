//! One call of an app's code (app library spec section 5): the input checked against the tool's
//! schema, one transaction on the app's database, the function run in the sandbox with `ctx`,
//! then a commit, or a rollback when it throws, times out, returns what JSON cannot hold or
//! returns too much. A call applies fully or not at all (spec section 9, items 2 and 3).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rquickjs::{Ctx, Object};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use super::data;
use super::engine::{self, Failure, Limits};
use super::manifest::Manifest;
use super::schema;

/// The largest result a dot gets from an app tool (spec section 5).
pub const MAX_DOT_RESULT: usize = 30 * 1024;
/// The largest result the app's screen gets (spec section 5).
pub const MAX_SCREEN_RESULT: usize = 1024 * 1024;
/// Bounds of `ctx.dots.start`'s arguments.
pub const MAX_START_MESSAGE: usize = 4000;
pub const MAX_START_TAG: usize = 200;

/// Who is calling: set by Dots, never by the caller (spec section 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// The person, through the app's screen.
    Person,
    /// A dot, from a run.
    Dot {
        id: String,
        name: String,
        run_id: String,
    },
    /// Dots itself: `on_run_end` (spec section 7).
    Dots,
}

/// A dot that uses the app, as `ctx.dots.list()` gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DotRef {
    pub id: String,
    pub name: String,
}

/// `ctx.dots.start` for an app whose "Can start dots" is on (spec section 7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartGate {
    /// The app-started topics still working or waiting, by (dot id, tag): their root run ids.
    pub open: BTreeMap<(String, String), String>,
    /// How many more starts the hour allows.
    pub budget: u32,
}

/// A start the call asked for. It is queued only after the call commits (spec section 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// The id the run gets; `start` returned it to the app.
    pub run_id: String,
    pub dot_id: String,
    pub message: String,
    pub tag: String,
}

/// One call.
pub struct Call<'a> {
    pub manifest: &'a Manifest,
    pub logic: &'a str,
    /// A listed tool, or for `Actor::Dots` the manifest's `on_run_end`.
    pub function: &'a str,
    pub input: &'a Value,
    pub actor: Actor,
    /// `ctx.now`: the call's time, RFC 3339 with the local offset.
    pub now: String,
    /// `ctx.dots.list()`.
    pub dots: Vec<DotRef>,
    /// `ctx.dots.start`, when the app may start dots.
    pub start: Option<StartGate>,
    pub limits: Limits,
}

/// What a committed call did.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub result: Value,
    /// The ids `ctx.id()` gave, in order (recorded with the call).
    pub ids: Vec<String>,
    pub starts: Vec<Start>,
    /// Rows the call changed.
    pub changes: u64,
}

/// Why a call did nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    UnknownTool(String),
    BadInput(String),
    Failed(Failure),
    TooLarge { bytes: usize, limit: usize },
    Db(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::UnknownTool(name) => write!(f, "the app has no tool {name}"),
            CallError::BadInput(e) => write!(f, "{e}"),
            CallError::Failed(e) => write!(f, "{e}"),
            CallError::TooLarge { bytes, limit } => {
                write!(f, "the result is {bytes} bytes; the limit is {limit}")
            }
            CallError::Db(e) => write!(f, "the app's database failed: {e}"),
        }
    }
}

struct State {
    conn: Rc<RefCell<Connection>>,
    ids: Vec<String>,
    starts: Vec<Start>,
    dots: Vec<DotRef>,
    gate: Option<StartGate>,
}

/// Runs `call` against the app database `conn` (spec section 5). Blocks: call it from
/// `spawn_blocking`.
pub fn call(conn: &Rc<RefCell<Connection>>, call: Call<'_>) -> Result<Outcome, CallError> {
    let is_entry = match &call.actor {
        Actor::Dots => call.manifest.on_run_end.as_deref() == Some(call.function),
        _ => call.manifest.tool(call.function).is_some(),
    };
    if !is_entry {
        return Err(CallError::UnknownTool(call.function.to_string()));
    }
    if let Some(tool) = call.manifest.tool(call.function) {
        schema::validate(&tool.input, call.input).map_err(CallError::BadInput)?;
    }
    let limit = match call.actor {
        Actor::Person => MAX_SCREEN_RESULT,
        _ => MAX_DOT_RESULT,
    };
    let db = |e: rusqlite::Error| CallError::Db(e.to_string());
    let before = {
        let c = conn.borrow();
        c.execute_batch("BEGIN IMMEDIATE").map_err(db)?;
        c.total_changes()
    };
    let state = Rc::new(RefCell::new(State {
        conn: conn.clone(),
        ids: Vec::new(),
        starts: Vec::new(),
        dots: call.dots.clone(),
        gate: call.start.clone(),
    }));
    let actor = serde_json::to_value(&call.actor).unwrap_or(Value::Null);
    let now = call.now.clone();
    data::guard(&conn.borrow());
    let result = engine::run(call.logic, call.function, call.input, call.limits, |ctx| {
        make_ctx(ctx, &state, &actor, &now)
    });
    data::unguard(&conn.borrow());
    let checked = result.map_err(CallError::Failed).and_then(|value| {
        let bytes = value.to_string().len();
        if bytes > limit {
            Err(CallError::TooLarge { bytes, limit })
        } else {
            Ok(value)
        }
    });
    let c = conn.borrow();
    match checked {
        Ok(result) => {
            let changes = c.total_changes() - before;
            c.execute_batch("COMMIT").map_err(db)?;
            let state = state.borrow();
            Ok(Outcome {
                result,
                ids: state.ids.clone(),
                starts: state.starts.clone(),
                changes,
            })
        }
        Err(e) => {
            let _ = c.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The `ctx` object (spec section 5): `now`, `actor`, `id()`, `db.query`, `db.exec`,
/// `dots.list()` and, with the grant, `dots.start`.
fn make_ctx<'js>(
    ctx: &Ctx<'js>,
    state: &Rc<RefCell<State>>,
    actor: &Value,
    now: &str,
) -> rquickjs::Result<Object<'js>> {
    let o = Object::new(ctx.clone())?;
    o.set("now", now)?;
    o.set("actor", engine::to_js(ctx, actor)?)?;
    let s = state.clone();
    o.set(
        "id",
        engine::function(ctx, move || {
            let id = crate::util::new_id();
            s.borrow_mut().ids.push(id.clone());
            id
        })?,
    )?;

    let db = Object::new(ctx.clone())?;
    let s = state.clone();
    db.set(
        "query",
        engine::function(
            ctx,
            move |ctx: Ctx<'js>,
                  sql: String,
                  params: rquickjs::function::Opt<rquickjs::Value<'js>>| {
                let params = params_of(&ctx, params.0)?;
                let conn = s.borrow().conn.clone();
                let rows = data::query(&conn.borrow(), &sql, &params)
                    .map_err(|e| engine::throw(&ctx, &format!("SQL: {e}")))?;
                engine::to_js(&ctx, &Value::Array(rows))
            },
        )?,
    )?;
    let s = state.clone();
    db.set(
        "exec",
        engine::function(
            ctx,
            move |ctx: Ctx<'js>,
                  sql: String,
                  params: rquickjs::function::Opt<rquickjs::Value<'js>>| {
                let params = params_of(&ctx, params.0)?;
                let conn = s.borrow().conn.clone();
                let changes = data::exec(&conn.borrow(), &sql, &params)
                    .map_err(|e| engine::throw(&ctx, &format!("SQL: {e}")))?;
                engine::to_js(&ctx, &json!({ "changes": changes }))
            },
        )?,
    )?;
    o.set("db", db)?;

    let dots = Object::new(ctx.clone())?;
    let s = state.clone();
    dots.set(
        "list",
        engine::function(ctx, move |ctx: Ctx<'js>| {
            let list = serde_json::to_value(&s.borrow().dots).unwrap_or(Value::Null);
            engine::to_js(&ctx, &list)
        })?,
    )?;
    if state.borrow().gate.is_some() {
        let s = state.clone();
        dots.set(
            "start",
            engine::function(
                ctx,
                move |ctx: Ctx<'js>, dot_id: String, message: String, tag: String| {
                    start(&mut s.borrow_mut(), dot_id, message, tag)
                        .map_err(|e| engine::throw(&ctx, &e))
                },
            )?,
        )?;
    }
    o.set("dots", dots)?;
    Ok(o)
}

fn params_of<'js>(
    ctx: &Ctx<'js>,
    value: Option<rquickjs::Value<'js>>,
) -> rquickjs::Result<Vec<Value>> {
    match value.and_then(|v| engine::from_js(ctx, v)) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => Ok(list),
        Some(_) => Err(engine::throw(ctx, "SQL parameters must be a list")),
    }
}

/// `ctx.dots.start(dot_id, message, tag)` (spec section 7): the run id, an open topic's for the
/// same dot and tag, or the rule it breaks.
fn start(
    state: &mut State,
    dot_id: String,
    message: String,
    tag: String,
) -> Result<String, String> {
    if !state.dots.iter().any(|d| d.id == dot_id) {
        return Err(format!("no dot {dot_id} uses this app"));
    }
    let len = message.trim().chars().count();
    if len == 0 || len > MAX_START_MESSAGE {
        return Err(format!(
            "the message must be 1 to {MAX_START_MESSAGE} characters"
        ));
    }
    let tag_len = tag.chars().count();
    if tag_len == 0 || tag_len > MAX_START_TAG {
        return Err(format!("the tag must be 1 to {MAX_START_TAG} characters"));
    }
    let gate = state
        .gate
        .as_ref()
        .expect("start is offered only with a gate");
    if let Some(root) = gate.open.get(&(dot_id.clone(), tag.clone())) {
        return Ok(root.clone());
    }
    if let Some(s) = state
        .starts
        .iter()
        .find(|s| s.dot_id == dot_id && s.tag == tag)
    {
        return Ok(s.run_id.clone());
    }
    if state.starts.len() as u32 >= gate.budget {
        return Err("this app has started as many dots as the hour allows; try again later".into());
    }
    let run_id = crate::util::new_id();
    state.starts.push(Start {
        run_id: run_id.clone(),
        dot_id,
        message,
        tag,
    });
    Ok(run_id)
}

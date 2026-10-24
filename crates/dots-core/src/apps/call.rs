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
    // A tool that says it does not write cannot: policy rules and the call log go by `writes`
    // (spec sections 4 to 6). `on_run_end` is not a tool, and may write.
    let read_only = call.manifest.tool(call.function).is_some_and(|t| !t.writes);
    let before = {
        let c = conn.borrow();
        c.execute_batch("BEGIN IMMEDIATE").map_err(db)?;
        // Set after BEGIN, since SQLite will not begin a write transaction under query_only.
        if let Err(e) = c.pragma_update(None, "query_only", read_only) {
            return Err(end(&c, db(e)));
        }
        c.total_changes()
    };
    let state = Rc::new(RefCell::new(State {
        conn: conn.clone(),
        ids: Vec::new(),
        starts: Vec::new(),
        dots: call.dots.clone(),
        // Starting a dot is a write: a tool that does not write gets no `dots.start`.
        gate: call.start.clone().filter(|_| !read_only),
    }));
    let actor = serde_json::to_value(&call.actor).unwrap_or(Value::Null);
    let now = call.now.clone();
    // The engine's limit cannot stop a statement inside SQLite; the deadline can.
    let deadline = std::time::Instant::now() + call.limits.time;
    data::set_deadline(&conn.borrow(), Some(deadline));
    data::guard(&conn.borrow());
    let result = engine::run(call.logic, call.function, call.input, call.limits, |ctx| {
        make_ctx(ctx, &state, &actor, &now)
    });
    data::unguard(&conn.borrow());
    data::set_deadline(&conn.borrow(), None);
    // The code may have caught the SQL the deadline interrupted: a call over its time is a
    // timeout whatever it returned (spec section 11).
    let result = if std::time::Instant::now() >= deadline {
        Err(Failure::TimedOut)
    } else {
        result
    };
    // The code's error text reaches the caller and the call log: bound it like a result.
    let result = result.map_err(|e| bounded(e, limit));
    let reset = conn.borrow().pragma_update(None, "query_only", false);
    let checked = result.map_err(CallError::Failed).and_then(|value| {
        reset.map_err(db)?;
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
            if let Err(e) = c.execute_batch("COMMIT") {
                // A COMMIT that fails (a deferred constraint, a busy database) leaves the
                // transaction open: end it, or a later call would publish these writes.
                return Err(end(&c, db(e)));
            }
            let state = state.borrow();
            Ok(Outcome {
                result,
                ids: state.ids.clone(),
                starts: state.starts.clone(),
                changes,
            })
        }
        Err(e) => Err(end(&c, e)),
    }
}

/// Ends the call's transaction after `e` with a ROLLBACK. When SQLite has ended it already, `e`
/// stands. When the rollback fails or leaves it open, the error says so, and the connection's
/// next BEGIN fails rather than publish these writes.
fn end(c: &Connection, e: CallError) -> CallError {
    if c.is_autocommit() {
        return e;
    }
    match c.execute_batch("ROLLBACK") {
        Ok(()) if c.is_autocommit() => e,
        Ok(()) => CallError::Db(format!("{e}; the transaction is still open")),
        Err(r) => CallError::Db(format!("{e}; the rollback failed: {r}")),
    }
}

/// `failure` with the text the app's code chose (a thrown error, a load error) cut to `limit`
/// bytes and its full length said.
fn bounded(failure: Failure, limit: usize) -> Failure {
    let cut = |text: String| {
        if text.len() <= limit {
            return text;
        }
        let mut at = limit;
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        format!("{}… ({} bytes)", &text[..at], text.len())
    };
    match failure {
        Failure::Thrown(text) => Failure::Thrown(cut(text)),
        Failure::Load(text) => Failure::Load(cut(text)),
        other => other,
    }
}

/// The app's SQL runs only inside the call's transaction. SQLite ends that transaction by
/// itself after some errors (an interrupted write, a full disk), and code that caught the error
/// would otherwise write outside it, each statement committed at once.
fn in_transaction(ctx: &Ctx<'_>, conn: &Connection) -> rquickjs::Result<()> {
    if conn.is_autocommit() {
        return Err(engine::throw(ctx, "SQL: the call's transaction has ended"));
    }
    Ok(())
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
                in_transaction(&ctx, &conn.borrow())?;
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
                in_transaction(&ctx, &conn.borrow())?;
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
    let message = message.trim().to_string();
    let len = message.chars().count();
    if len == 0 || len > MAX_START_MESSAGE {
        return Err(format!(
            "the message must be 1 to {MAX_START_MESSAGE} characters"
        ));
    }
    let tag = tag.trim().to_string();
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

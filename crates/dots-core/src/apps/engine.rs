//! The sandbox an app's `logic.js` runs in (app library spec section 5): QuickJS, embedded, with
//! no file, network or process access, a memory limit and a time limit. The clock and random
//! numbers are taken out, so a call gives the same result for the same data, input, `ctx.now`
//! and ids. Each call gets a fresh runtime: nothing an app leaves in memory survives a call.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Exception, Function, Module, Object, Runtime, Value};

/// The longest a call may run (spec section 5).
pub const TIME_LIMIT: Duration = Duration::from_secs(2);
/// The most memory a call's runtime may hold (spec section 5).
pub const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// The module name errors and stacks show for an app's logic.
pub const LOGIC_FILE: &str = "logic.js";

/// Takes the clock, random numbers and garbage-collection timing out of the engine (spec section
/// 9, item 1): `Date.now`, `Date()`, `new Date()` without a value and `Math.random` throw,
/// naming what to use instead. `new Date(value)`, `Date.UTC` and `Date.parse` stay. `performance`
/// (a clock), `WeakRef` and `FinalizationRegistry` (whose results follow garbage collection) are
/// deleted.
const LOCKDOWN: &str = r#"
(() => {
  delete globalThis.performance;
  delete globalThis.WeakRef;
  delete globalThis.FinalizationRegistry;
  const RealDate = Date;
  const refuse = (what, instead) => () => {
    throw new Error(`${what} is not available in an app: ${instead}`);
  };
  function AppDate(...args) {
    if (!new.target) refuse("Date()", "use ctx.now")();
    if (args.length === 0) refuse("new Date()", "use new Date(ctx.now)")();
    return Reflect.construct(RealDate, args, new.target);
  }
  AppDate.prototype = RealDate.prototype;
  Object.defineProperty(RealDate.prototype, "constructor", { value: AppDate });
  AppDate.UTC = RealDate.UTC;
  AppDate.parse = RealDate.parse;
  AppDate.now = refuse("Date.now", "use ctx.now");
  globalThis.Date = AppDate;
  Math.random = refuse("Math.random", "use ctx.id()");
})();
"#;

/// The bounds of one sandbox.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub time: Duration,
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            time: TIME_LIMIT,
            memory: MEMORY_LIMIT,
        }
    }
}

/// Why a sandboxed call gave no result. Every failure rolls the call back (spec section 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The code could not be loaded: a syntax error, an `import`, a throw at the top level.
    Load(String),
    /// The module exports no function of that name.
    NoFunction(String),
    /// The code threw, or its promise was rejected: the error's text.
    Thrown(String),
    /// The code ran over the time limit.
    TimedOut,
    /// The code ran over the memory limit.
    OutOfMemory,
    /// The result is not JSON (a function, a cycle, a BigInt).
    NotJson,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Load(e) => write!(f, "the app's code could not be loaded: {e}"),
            Failure::NoFunction(name) => write!(f, "the app's code exports no function {name}"),
            Failure::Thrown(e) => write!(f, "{e}"),
            Failure::TimedOut => write!(f, "the call ran over {} s", TIME_LIMIT.as_secs()),
            Failure::OutOfMemory => {
                write!(
                    f,
                    "the call ran over {} MB of memory",
                    MEMORY_LIMIT / (1024 * 1024)
                )
            }
            Failure::NotJson => write!(f, "the result is not JSON"),
        }
    }
}

/// One sandbox: a fresh runtime and context, locked down, with `limits`. `f` gets the context
/// and a judge that turns the engine's errors into `Failure`s.
pub fn with_sandbox<R>(
    limits: Limits,
    f: impl for<'js> FnOnce(&Ctx<'js>, &Judge) -> Result<R, Failure>,
) -> Result<R, Failure> {
    let runtime = Runtime::new().map_err(|e| Failure::Load(e.to_string()))?;
    runtime.set_memory_limit(limits.memory);
    // 256 KB, not more: a call must fit any caller's thread, not only spawn_blocking's 2 MB.
    runtime.set_max_stack_size(256 * 1024);
    let timed_out = Rc::new(Cell::new(false));
    let deadline = Instant::now() + limits.time;
    let flag = timed_out.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let over = Instant::now() >= deadline;
        if over {
            flag.set(true);
        }
        over
    })));
    let context = Context::full(&runtime).map_err(|e| Failure::Load(e.to_string()))?;
    let judge = Judge { timed_out };
    context.with(|ctx| {
        ctx.eval::<(), _>(LOCKDOWN)
            .map_err(|e| judge.failure(&ctx, e, Failure::Load))?;
        f(&ctx, &judge)
    })
}

/// Turns an engine error into a `Failure`, telling a timeout and a full memory from a throw.
pub struct Judge {
    timed_out: Rc<Cell<bool>>,
}

impl Judge {
    pub fn failure(
        &self,
        ctx: &Ctx<'_>,
        error: rquickjs::Error,
        wrap: fn(String) -> Failure,
    ) -> Failure {
        if self.timed_out.get() {
            return Failure::TimedOut;
        }
        let text = match error {
            rquickjs::Error::Exception => describe(&ctx.catch()),
            rquickjs::Error::Allocation => return Failure::OutOfMemory,
            other => other.to_string(),
        };
        if text.contains("out of memory") {
            Failure::OutOfMemory
        } else {
            wrap(text)
        }
    }

    /// The value the code threw, for a caller that judges it itself; `Err` when the error is a
    /// timeout, a full memory, or not a throw.
    pub fn caught<'js>(
        &self,
        ctx: &Ctx<'js>,
        error: rquickjs::Error,
    ) -> Result<Value<'js>, Failure> {
        if self.timed_out.get() {
            return Err(Failure::TimedOut);
        }
        match error {
            rquickjs::Error::Exception => {
                let thrown = ctx.catch();
                if describe(&thrown).contains("out of memory") {
                    Err(Failure::OutOfMemory)
                } else {
                    Ok(thrown)
                }
            }
            rquickjs::Error::Allocation => Err(Failure::OutOfMemory),
            other => Err(Failure::Thrown(other.to_string())),
        }
    }
}

/// The text of a thrown value: `Name: message` for an error, else the value as a string.
fn describe(value: &Value<'_>) -> String {
    if let Some(exception) = value.as_exception() {
        let message = exception.message().unwrap_or_default();
        let name: Option<String> = exception.get("name").ok();
        return match name {
            Some(name) if !name.is_empty() => format!("{name}: {message}"),
            _ => message,
        };
    }
    value
        .as_string()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| format!("{value:?}"))
}

/// Loads `source` as the ES module `name` (no imports: there is no loader) and returns its
/// exports.
pub fn load<'js>(
    ctx: &Ctx<'js>,
    judge: &Judge,
    name: &str,
    source: &str,
) -> Result<Object<'js>, Failure> {
    let load = |e| judge.failure(ctx, e, Failure::Load);
    let declared = Module::declare(ctx.clone(), name, source).map_err(load)?;
    let (module, done) = declared.eval().map_err(load)?;
    done.finish::<()>().map_err(load)?;
    module.namespace().map_err(load)
}

/// Converts a JSON value into the engine's.
pub fn to_js<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> rquickjs::Result<Value<'js>> {
    ctx.json_parse(value.to_string())
}

/// Converts an engine value into JSON: `undefined` is `null`; a value JSON cannot hold is `None`.
pub fn from_js<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Option<serde_json::Value> {
    if value.is_undefined() {
        return Some(serde_json::Value::Null);
    }
    match ctx.json_stringify(value) {
        Ok(Some(text)) => serde_json::from_str(&text.to_string().ok()?).ok(),
        _ => None,
    }
}

/// Throws `message` as an `Error` in the app's code.
pub fn throw(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    Exception::throw_message(ctx, message)
}

/// Calls the export `function` of `exports` with `args`; a promise it returns is run to the end.
pub fn call_export<'js>(
    ctx: &Ctx<'js>,
    judge: &Judge,
    exports: &Object<'js>,
    function: &str,
    args: (Value<'js>, Value<'js>),
) -> Result<serde_json::Value, Failure> {
    let thrown = |e| judge.failure(ctx, e, Failure::Thrown);
    let export: Value = exports.get(function).map_err(thrown)?;
    let Some(func) = export.as_function().cloned() else {
        return Err(Failure::NoFunction(function.to_string()));
    };
    let mut result: Value = func.call(args).map_err(thrown)?;
    if let Some(promise) = result.as_promise().cloned() {
        result = match promise.finish::<Value>() {
            Ok(v) => v,
            Err(rquickjs::Error::WouldBlock) => {
                return Err(Failure::Thrown("the call's promise never settled".into()))
            }
            Err(e) => return Err(thrown(e)),
        };
    }
    from_js(ctx, result).ok_or(Failure::NotJson)
}

/// Runs `function` of the module `source` with `input` and a `ctx` object made by `make_ctx`, in
/// a fresh sandbox (spec section 5).
pub fn run(
    source: &str,
    function: &str,
    input: &serde_json::Value,
    limits: Limits,
    make_ctx: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<Object<'js>>,
) -> Result<serde_json::Value, Failure> {
    with_sandbox(limits, |ctx, judge| {
        let exports = load(ctx, judge, LOGIC_FILE, source)?;
        let setup = |e| judge.failure(ctx, e, Failure::Load);
        let input = to_js(ctx, input).map_err(setup)?;
        let app_ctx = make_ctx(ctx).map_err(setup)?;
        call_export(
            ctx,
            judge,
            &exports,
            function,
            (input, app_ctx.into_value()),
        )
    })
}

/// A function the app's code can call, from a Rust closure.
pub fn function<'js, P, F>(ctx: &Ctx<'js>, f: F) -> rquickjs::Result<Function<'js>>
where
    F: rquickjs::function::IntoJsFunc<'js, P> + 'js,
{
    Function::new(ctx.clone(), f)
}

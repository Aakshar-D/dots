//! An app's own database (app library spec section 5): a SQLite file per app that only its code
//! reaches, through `ctx.db`, inside the one transaction Dots opens for each call. The app's SQL
//! runs under an authorizer: no `ATTACH`, no `PRAGMA` but a few that only read, no transaction
//! statements (Dots owns the transaction), no virtual tables, no temporary objects (they would
//! sit outside the size bound), no extensions, no `random()` (a call is deterministic, spec
//! section 9). `query` and `exec` take one statement, and a deadline stops runaway SQL.

use std::path::Path;
use std::time::{Duration, Instant};

use base64::Engine as _;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Batch, Connection, Statement};
use serde_json::{Map, Value};

use super::source::Migration;
use crate::{Error, Result};

/// The largest an app's database may grow (spec section 5).
pub const MAX_DB_BYTES: u64 = 512 * 1024 * 1024;
/// The most a `ctx.db.query` may return, counted as the JSON it becomes: the call's memory
/// limit (spec section 5).
pub const MAX_QUERY_BYTES: usize = 64 * 1024 * 1024;
/// How long migrations or a `sample.sql` may run (my call: the spec bounds a call at 2 s, and
/// migrations run once, on data up to `MAX_DB_BYTES`).
pub const SETUP_TIME: Duration = Duration::from_secs(60);
/// How many SQLite steps run between two looks at the deadline.
const STEPS_PER_CHECK: i32 = 1000;
/// Functions the app's SQL may not call: the extension loader, and random numbers.
const DENIED_FUNCTIONS: &[&str] = &["load_extension", "random", "randomblob"];
/// Pragmas the app's SQL may use: they read the schema and change nothing.
const READ_PRAGMAS: &[&str] = &[
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
];

/// Opens (creating it if needed) the app database at `path`, bounded to `MAX_DB_BYTES`, with
/// foreign keys on and a busy timeout. Blocks.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path).map_err(sql_error)?;
    configure(&conn)?;
    Ok(conn)
}

/// A database in memory, for a review preview and the tests of a proposal (spec sections 6.2
/// and 9, item 4).
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory().map_err(sql_error)?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(sql_error)?;
    // The bound is counted in the file's own pages, whatever page size made it.
    let page_size: u64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(sql_error)?;
    conn.execute_batch(&format!(
        "PRAGMA foreign_keys = ON; PRAGMA max_page_count = {};",
        MAX_DB_BYTES / page_size
    ))
    .map_err(sql_error)?;
    // WAL lets the screen read while a dot's call writes; an in-memory database keeps its own.
    let _: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .map_err(sql_error)?;
    Ok(())
}

pub(crate) fn sql_error(e: rusqlite::Error) -> Error {
    Error::Other(format!("app database: {e}"))
}

/// What the app's SQL may do (module docs).
fn authorize(ctx: AuthContext<'_>) -> Authorization {
    // `CREATE TABLE temp.t` is reported as a plain create, in the database "temp".
    let in_temp = ctx
        .database_name
        .is_some_and(|d| d.eq_ignore_ascii_case("temp"));
    match ctx.action {
        AuthAction::CreateTable { .. }
        | AuthAction::CreateIndex { .. }
        | AuthAction::CreateTrigger { .. }
        | AuthAction::CreateView { .. }
            if in_temp =>
        {
            Authorization::Deny
        }
        AuthAction::Attach { .. }
        | AuthAction::Detach { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::CreateVtable { .. }
        | AuthAction::DropVtable { .. }
        | AuthAction::CreateTempIndex { .. }
        | AuthAction::CreateTempTable { .. }
        | AuthAction::CreateTempTrigger { .. }
        | AuthAction::CreateTempView { .. } => Authorization::Deny,
        AuthAction::Pragma { pragma_name, .. } => {
            if READ_PRAGMAS.contains(&pragma_name.to_ascii_lowercase().as_str()) {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }
        AuthAction::Function { function_name }
            if DENIED_FUNCTIONS
                .iter()
                .any(|f| function_name.eq_ignore_ascii_case(f)) =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
}

/// Runs `f` with the app's SQL rules on, and takes them off after.
pub fn guarded<T>(conn: &Connection, f: impl FnOnce(&Connection) -> T) -> T {
    conn.authorizer(Some(authorize));
    let out = f(conn);
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    out
}

/// Stops the SQL running on `conn` once `deadline` passes (the statement fails with
/// "interrupted"), or lifts the deadline with `None`. The engine's own time limit cannot stop a
/// statement while it runs inside SQLite.
pub fn set_deadline(conn: &Connection, deadline: Option<Instant>) {
    match deadline {
        Some(at) => conn.progress_handler(STEPS_PER_CHECK, Some(move || Instant::now() > at)),
        None => conn.progress_handler(STEPS_PER_CHECK, None::<fn() -> bool>),
    }
}

/// The schema version the database is at: the number of the newest migration applied.
pub fn schema_version(conn: &Connection) -> Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql_error)
}

/// Applies the migrations newer than the database's schema version, in order, in one transaction
/// under the app's SQL rules, and records the newest (spec section 5). Returns how many ran. On
/// a failure nothing is applied and `Err(Invalid)` names the migration. A database newer than the
/// migrations is refused. The migrations have `SETUP_TIME`. Blocks.
pub fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<usize> {
    migrate_within(conn, migrations, SETUP_TIME)
}

/// `migrate`, with the migrations given `time` instead of `SETUP_TIME`.
pub fn migrate_within(
    conn: &mut Connection,
    migrations: &[Migration],
    time: Duration,
) -> Result<usize> {
    let current = schema_version(conn)?;
    let newest = migrations.last().map_or(0, |m| m.number);
    if current > newest {
        return Err(Error::Invalid(format!(
            "the app's data is at schema {current}, newer than its migrations (up to {newest})"
        )));
    }
    let pending: Vec<&Migration> = migrations.iter().filter(|m| m.number > current).collect();
    if pending.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction().map_err(sql_error)?;
    set_deadline(&tx, Some(Instant::now() + time));
    let applied = pending.iter().try_for_each(|m| {
        guarded(&tx, |c| c.execute_batch(&m.sql))
            .map_err(|e| Error::Invalid(format!("migrations/{}: {e}", m.file)))
    });
    set_deadline(&tx, None);
    applied?;
    tx.pragma_update(None, "user_version", newest)
        .map_err(sql_error)?;
    tx.commit().map_err(sql_error)?;
    Ok(pending.len())
}

/// Loads a proposal's `sample.sql` into a preview database, under the app's SQL rules and within
/// `SETUP_TIME`.
pub fn load_sample(conn: &Connection, sql: &str) -> Result<()> {
    set_deadline(conn, Some(Instant::now() + SETUP_TIME));
    let loaded = guarded(conn, |c| c.execute_batch(sql));
    set_deadline(conn, None);
    loaded.map_err(|e| Error::Invalid(format!("sample.sql: {e}")))
}

/// A JSON parameter as SQLite holds it: booleans as 0 and 1, lists and objects as JSON text.
pub fn to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => SqlValue::Integer(i),
            None => SqlValue::Real(n.as_f64().unwrap_or_default()),
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

fn to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::String(base64::engine::general_purpose::STANDARD.encode(b)),
    }
}

/// About how many bytes `value` takes as JSON, counted before it is converted.
fn json_size(value: ValueRef<'_>) -> usize {
    match value {
        ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) => 8,
        ValueRef::Text(t) => t.len() + 2,
        ValueRef::Blob(b) => b.len().div_ceil(3) * 4 + 2,
    }
}

/// The one statement `sql` holds. `Err` when it holds none or several: rusqlite would run the
/// first and drop the rest without a word.
fn one_statement<'c>(
    conn: &'c Connection,
    sql: &str,
) -> std::result::Result<Statement<'c>, String> {
    let mut batch = Batch::new(conn, sql);
    let stmt = batch
        .next()
        .map_err(|e| e.to_string())?
        .ok_or("no SQL statement")?;
    if batch.next().map_err(|e| e.to_string())?.is_some() {
        return Err("only one SQL statement at a time".into());
    }
    Ok(stmt)
}

/// `ctx.db.query`: the rows of `sql` as objects by column name, up to `MAX_QUERY_BYTES`. `Err`
/// is SQLite's message.
pub fn query(
    conn: &Connection,
    sql: &str,
    params: &[Value],
) -> std::result::Result<Vec<Value>, String> {
    let mut stmt = one_statement(conn, sql)?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut bytes = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut obj = Map::new();
        for (i, name) in names.iter().enumerate() {
            let value = row.get_ref(i).map_err(|e| e.to_string())?;
            bytes += name.len() + json_size(value);
            if bytes > MAX_QUERY_BYTES {
                return Err(format!(
                    "the rows are over {} MB",
                    MAX_QUERY_BYTES / (1024 * 1024)
                ));
            }
            obj.insert(name.clone(), to_json(value));
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

/// `ctx.db.exec`: runs the one statement `sql` holds; the count of rows it changed (0 for a
/// statement that changes no rows, such as `CREATE TABLE`).
pub fn exec(conn: &Connection, sql: &str, params: &[Value]) -> std::result::Result<u64, String> {
    let mut stmt = one_statement(conn, sql)?;
    let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
    let before = conn.total_changes();
    stmt.execute(rusqlite::params_from_iter(params))
        .map_err(|e| e.to_string())?;
    Ok(conn.total_changes() - before)
}

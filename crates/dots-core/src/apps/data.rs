//! An app's own database (app library spec section 5): a SQLite file per app that only its code
//! reaches, through `ctx.db`, inside the one transaction Dots opens for each call. The app's SQL
//! runs under an authorizer: no `ATTACH`, no `PRAGMA` but a few that only read, no transaction
//! statements (Dots owns the transaction), no virtual tables, no extensions.

use std::path::Path;

use base64::Engine as _;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::Connection;
use serde_json::{Map, Value};

use super::source::Migration;
use crate::{Error, Result};

/// The largest an app's database may grow (spec section 5).
pub const MAX_DB_BYTES: u64 = 512 * 1024 * 1024;
/// The page size the bound is counted in (SQLite's default).
const PAGE_SIZE: u64 = 4096;
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
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(sql_error)?;
    conn.execute_batch(&format!(
        "PRAGMA foreign_keys = ON; PRAGMA max_page_count = {};",
        MAX_DB_BYTES / PAGE_SIZE
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
    match ctx.action {
        AuthAction::Attach { .. }
        | AuthAction::Detach { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::CreateVtable { .. }
        | AuthAction::DropVtable { .. } => Authorization::Deny,
        AuthAction::Pragma { pragma_name, .. } => {
            if READ_PRAGMAS.contains(&pragma_name.to_ascii_lowercase().as_str()) {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }
        AuthAction::Function { function_name }
            if function_name.eq_ignore_ascii_case("load_extension") =>
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

/// The schema version the database is at: the number of the newest migration applied.
pub fn schema_version(conn: &Connection) -> Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(sql_error)
}

/// Applies the migrations newer than the database's schema version, in order, in one transaction
/// under the app's SQL rules, and records the newest (spec section 5). Returns how many ran. On
/// a failure nothing is applied and `Err(Invalid)` names the migration. A database newer than the
/// migrations is refused. Blocks.
pub fn migrate(conn: &mut Connection, migrations: &[Migration]) -> Result<usize> {
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
    for m in &pending {
        guarded(&tx, |c| c.execute_batch(&m.sql))
            .map_err(|e| Error::Invalid(format!("migrations/{}: {e}", m.file)))?;
    }
    tx.pragma_update(None, "user_version", newest)
        .map_err(sql_error)?;
    tx.commit().map_err(sql_error)?;
    Ok(pending.len())
}

/// Loads a proposal's `sample.sql` into a preview database, under the app's SQL rules.
pub fn load_sample(conn: &Connection, sql: &str) -> Result<()> {
    guarded(conn, |c| c.execute_batch(sql)).map_err(|e| Error::Invalid(format!("sample.sql: {e}")))
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

/// `ctx.db.query`: the rows of `sql` as objects by column name. `Err` is SQLite's message.
pub fn query(
    conn: &Connection,
    sql: &str,
    params: &[Value],
) -> std::result::Result<Vec<Value>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut obj = Map::new();
        for (i, name) in names.iter().enumerate() {
            obj.insert(
                name.clone(),
                to_json(row.get_ref(i).map_err(|e| e.to_string())?),
            );
        }
        out.push(Value::Object(obj));
    }
    Ok(out)
}

/// `ctx.db.exec`: runs one statement of `sql`; the count of rows it changed.
pub fn exec(conn: &Connection, sql: &str, params: &[Value]) -> std::result::Result<u64, String> {
    let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
    conn.execute(sql, rusqlite::params_from_iter(params))
        .map(|n| n as u64)
        .map_err(|e| e.to_string())
}

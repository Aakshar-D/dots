//! An app's own database (app library spec section 5).

mod common;

use dots_core::apps::data::{self, guarded};
use dots_core::apps::source::Migration;
use serde_json::json;

fn migration(number: u32, file: &str, sql: &str) -> Migration {
    Migration {
        number,
        file: file.into(),
        sql: sql.into(),
    }
}

fn expenses_db() -> rusqlite::Connection {
    let mut conn = data::open_in_memory().unwrap();
    data::migrate(
        &mut conn,
        &[migration(1, "0001_expenses.sql", common::apps::MIGRATION)],
    )
    .unwrap();
    conn
}

#[test]
fn migrations_run_once_in_order_and_record_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let mut conn = data::open(&path).unwrap();
    let all = [
        migration(1, "0001_a.sql", "CREATE TABLE a (x INTEGER);"),
        migration(2, "0002_b.sql", "ALTER TABLE a ADD COLUMN y TEXT;"),
    ];
    assert_eq!(data::migrate(&mut conn, &all[..1]).unwrap(), 1);
    assert_eq!(data::schema_version(&conn).unwrap(), 1);
    assert_eq!(data::migrate(&mut conn, &all).unwrap(), 1);
    assert_eq!(data::migrate(&mut conn, &all).unwrap(), 0);
    drop(conn);
    let conn = data::open(&path).unwrap();
    assert_eq!(data::schema_version(&conn).unwrap(), 2);
    assert_eq!(
        data::query(
            &conn,
            "SELECT name FROM pragma_table_info('a') ORDER BY cid",
            &[]
        )
        .unwrap(),
        vec![json!({"name": "x"}), json!({"name": "y"})]
    );
}

#[test]
fn a_failing_migration_applies_nothing() {
    let mut conn = data::open_in_memory().unwrap();
    let err = data::migrate(
        &mut conn,
        &[
            migration(1, "0001_a.sql", "CREATE TABLE a (x INTEGER);"),
            migration(2, "0002_bad.sql", "CREATE TABLE a (x INTEGER);"),
        ],
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        err,
        "invalid input: migrations/0002_bad.sql: table a already exists"
    );
    assert_eq!(data::schema_version(&conn).unwrap(), 0);
    assert!(data::query(&conn, "SELECT * FROM a", &[]).is_err());
}

#[test]
fn a_database_newer_than_the_migrations_is_refused() {
    let mut conn = expenses_db();
    let err = data::migrate(&mut conn, &[]).unwrap_err().to_string();
    assert_eq!(
        err,
        "invalid input: the app's data is at schema 1, newer than its migrations (up to 0)"
    );
}

#[test]
fn query_and_exec_carry_json_values() {
    let conn = expenses_db();
    let n = data::exec(
        &conn,
        "INSERT INTO expenses VALUES (?, ?, ?, ?, ?, ?)",
        &[
            json!("e1"),
            json!("Coffee"),
            json!(4.5),
            json!("2026-10-22"),
            json!("t"),
            json!("you"),
        ],
    )
    .unwrap();
    assert_eq!(n, 1);
    let rows = data::query(
        &conn,
        "SELECT id, amount, ? AS flag, ? AS list, NULL AS none, x'0102' AS bytes FROM expenses",
        &[json!(true), json!(["a", 1])],
    )
    .unwrap();
    assert_eq!(
        rows,
        vec![
            json!({"id": "e1", "amount": 4.5, "flag": 1, "list": "[\"a\",1]", "none": null, "bytes": "AQI="})
        ]
    );
}

#[test]
fn the_apps_sql_cannot_reach_past_its_database() {
    let conn = expenses_db();
    for sql in [
        "ATTACH DATABASE 'C:/other.db' AS other",
        "PRAGMA journal_mode = DELETE",
        "PRAGMA max_page_count = 999999999",
        "PRAGMA user_version = 7",
        "BEGIN",
        "COMMIT",
        "SAVEPOINT s1",
        "SELECT load_extension('evil')",
    ] {
        let err = guarded(&conn, |c| data::exec(c, sql, &[])).unwrap_err();
        assert!(err.contains("not authorized"), "{sql}: {err}");
    }
    // Pragmas that only read the schema work.
    let rows = guarded(&conn, |c| {
        data::query(c, "PRAGMA table_info(expenses)", &[])
    })
    .unwrap();
    assert_eq!(rows.len(), 6);
    // Outside `guarded`, Dots' own statements still run.
    assert_eq!(data::schema_version(&conn).unwrap(), 1);
}

#[test]
fn a_migration_runs_under_the_same_rules() {
    let mut conn = data::open_in_memory().unwrap();
    let err = data::migrate(
        &mut conn,
        &[migration(
            1,
            "0001_a.sql",
            "CREATE TABLE a (x); PRAGMA user_version = 9;",
        )],
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.starts_with("invalid input: migrations/0001_a.sql: not authorized"),
        "{err}"
    );
}

#[test]
fn the_sample_loads_under_the_rules() {
    let conn = expenses_db();
    data::load_sample(&conn, common::apps::SAMPLE).unwrap();
    assert_eq!(
        data::query(&conn, "SELECT id FROM expenses", &[])
            .unwrap()
            .len(),
        2
    );
    let err = data::load_sample(&conn, "ATTACH 'x.db' AS x;")
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with("invalid input: sample.sql: not authorized"),
        "{err}"
    );
}

#[test]
fn the_database_is_bounded() {
    let conn = data::open_in_memory().unwrap();
    let pages: u64 = conn
        .query_row("PRAGMA max_page_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pages * 4096, data::MAX_DB_BYTES);
}

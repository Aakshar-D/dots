mod common;

use chrono::{Local, TimeDelta, TimeZone, Utc};
use common::{spec, temp_store};
use dots_core::library::LibraryConfig;
use dots_core::model::{Dot, EngineKind};
use dots_core::policy::{Policy, Preset};
use dots_core::schedule::{next_slot, Rule, Schedule};
use dots_core::scheduler::parse_cron;
use dots_core::store::Store;
use dots_core::Error;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

fn schedule(v: Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

async fn raw_pool(dir: &tempfile::TempDir) -> SqlitePool {
    SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap()
}

fn never_runs(e: &Error) -> bool {
    matches!(e, Error::Invalid(m) if m == "this schedule would never run")
}

#[tokio::test]
async fn create_and_get_round_trip() {
    let (_d, store) = temp_store().await;
    let mut s = spec("triage");
    s.schedule = Some(Schedule::cron("0 0 8 * * Mon-Fri"));
    let sf = LibraryConfig::McpServer(
        serde_json::from_value(json!({"transport": "stdio", "command": "sf-mcp"})).unwrap(),
    );
    store.add_library_item("sf", &sf, None).await.unwrap();
    s.uses.mcp_servers = vec!["sf".into()];
    let dot = store.create_dot(&s, None).await.unwrap();
    assert_eq!(dot.spec, s);
    assert_eq!(dot.id.len(), 26);
    // No webhook token until one is made (env vault spec §11).
    assert!(!dot.has_webhook_token);
    assert_eq!(dot.created_at, dot.updated_at);
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);
    assert_eq!(dot.spec.policy.preset, Preset::Sandboxed);
    // A dot is Personal unless it is made in an org.
    assert_eq!(dot.org_id, None);
}

#[tokio::test]
async fn duplicate_name_conflicts() {
    let (_d, store) = temp_store().await;
    store.create_dot(&spec("a"), None).await.unwrap();
    let err = store.create_dot(&spec("a"), None).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn invalid_specs_are_rejected() {
    let (_d, store) = temp_store().await;
    let bad_name = spec("has space");
    let mut local_no_endpoint = spec("loc");
    local_no_endpoint.engine = EngineKind::Local;
    let mut five_field_cron = spec("cron5");
    five_field_cron.schedule = Some(Schedule::cron("0 8 * * *"));
    let mut every_zero = spec("every0");
    every_zero.schedule = Some(schedule(json!({"rule": {"kind": "every", "minutes": 0}})));
    let mut weekly_no_day = spec("weekly");
    weekly_no_day.schedule = Some(schedule(
        json!({"rule": {"kind": "weekly", "days": [], "time": "09:00"}}),
    ));
    let mut once_bounded = spec("once");
    once_bounded.schedule = Some(schedule(json!({
        "rule": {"kind": "once", "at": "2100-01-01T09:00"},
        "until": {"count": 2}
    })));
    let mut count_zero = spec("count0");
    count_zero.schedule = Some(schedule(json!({
        "rule": {"kind": "daily", "time": "09:00"},
        "until": {"count": 0}
    })));
    let mut unknown_server = spec("mcp");
    unknown_server.uses.mcp_servers = vec!["sf".into()];
    let mut empty_skill = spec("skill");
    empty_skill.uses.skills = vec!["".into()];
    let mut zero_turns = spec("zero");
    zero_turns.max_turns = 0;
    let mut local_https = spec("https");
    local_https.engine = EngineKind::Local;
    local_https.endpoint_url = Some("https://example.com".into());
    let mut local_no_host = spec("nohost");
    local_no_host.engine = EngineKind::Local;
    local_no_host.endpoint_url = Some("http:///v1".into());
    for s in [
        bad_name,
        local_no_endpoint,
        local_https,
        local_no_host,
        five_field_cron,
        every_zero,
        weekly_no_day,
        once_bounded,
        count_zero,
        unknown_server,
        empty_skill,
        zero_turns,
    ] {
        let err = store.create_dot(&s, None).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{}: {err:?}", s.name);
    }
    assert!(store.list_dots().await.unwrap().is_empty());
}

#[tokio::test]
async fn update_changes_fields_and_timestamp() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let mut s = dot.spec.clone();
    s.instructions = "New instructions".into();
    s.max_turns = 7;
    assert!(s.use_global_instructions);
    s.use_global_instructions = false;
    let updated = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(updated.spec.instructions, "New instructions");
    assert_eq!(updated.spec.max_turns, 7);
    assert!(!updated.spec.use_global_instructions);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec, s);
    assert_eq!(updated.has_webhook_token, dot.has_webhook_token);
    assert!(updated.updated_at > dot.updated_at);
    assert!(matches!(
        store.update_dot("missing", &s).await,
        Err(Error::NotFound(_))
    ));
}

/// A dot's org is fixed when it is made (env vault spec §5.2): no change of the dot moves it.
#[tokio::test]
async fn no_change_of_a_dot_changes_its_org() {
    let (_d, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("a"), Some("acme-eng"))
        .await
        .unwrap();
    assert_eq!(dot.org_id.as_deref(), Some("acme-eng"));
    let org = |d: Dot| d.org_id;
    let mut s = dot.spec.clone();
    s.name = "renamed".into();
    s.instructions = "New instructions".into();
    s.enabled = false;
    let updated = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(updated.spec.name, "renamed");
    assert_eq!(org(updated), dot.org_id);
    let enabled = store.set_dot_enabled(&dot.id, true).await.unwrap();
    assert_eq!(org(enabled), dot.org_id);
    assert_eq!(org(store.mark_dot_seen(&dot.id).await.unwrap()), dot.org_id);
    store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_eq!(org(store.get_dot(&dot.id).await.unwrap()), dot.org_id);
    // A Personal dot stays Personal.
    let personal = store.create_dot(&spec("b"), None).await.unwrap();
    let saved = store
        .update_dot(&personal.id, &personal.spec)
        .await
        .unwrap();
    assert_eq!(org(saved), None);
}

#[tokio::test]
async fn list_is_sorted_by_name() {
    let (_d, store) = temp_store().await;
    for n in ["c", "a", "b"] {
        store.create_dot(&spec(n), None).await.unwrap();
    }
    let names: Vec<String> = store
        .list_dots()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.spec.name)
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
}

#[tokio::test]
async fn delete_enable_and_token_regeneration() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None).await.unwrap();
    let disabled = store.set_dot_enabled(&dot.id, false).await.unwrap();
    assert!(!disabled.spec.enabled);
    assert!(!store.check_webhook_token(&dot.id, "").await.unwrap());
    let first = store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_eq!(first.len(), 43);
    assert!(store.get_dot(&dot.id).await.unwrap().has_webhook_token);
    assert!(store.check_webhook_token(&dot.id, &first).await.unwrap());
    let token = store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_ne!(token, first);
    assert!(!store.check_webhook_token(&dot.id, &first).await.unwrap());
    assert!(store.check_webhook_token(&dot.id, &token).await.unwrap());
    // Only its hash is stored.
    let stored: String = sqlx::query_scalar("SELECT webhook_token FROM dots WHERE id = ?")
        .bind(&dot.id)
        .fetch_one(
            &sqlx::SqlitePool::connect_with(
                sqlx::sqlite::SqliteConnectOptions::new().filename(_d.path().join("dots.db")),
            )
            .await
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(stored.starts_with("sha256:") && !stored.contains(&token));
    store.delete_dot(&dot.id).await.unwrap();
    assert!(matches!(
        store.get_dot(&dot.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.delete_dot(&dot.id).await,
        Err(Error::NotFound(_))
    ));
}

#[test]
fn cron_requires_seconds_field() {
    assert!(parse_cron("0 0 8 * * Mon-Fri").is_ok());
    assert!(parse_cron("0 0 8 * * * 2027").is_ok());
    assert!(parse_cron("0 8 * * *").is_err());
    assert!(parse_cron("not cron at all x").is_err());
}

#[test]
fn the_next_cron_slot_is_strictly_after() {
    let cron = Schedule::cron("0 0 8 * * *");
    let at = Local.with_ymd_and_hms(2026, 10, 8, 7, 0, 0).unwrap();
    let next = next_slot(&cron, &at, &Local).unwrap().unwrap();
    assert_eq!(next, Local.with_ymd_and_hms(2026, 10, 8, 8, 0, 0).unwrap());
    let again = next_slot(&cron, &next, &Local).unwrap().unwrap();
    assert_eq!(again, Local.with_ymd_and_hms(2026, 10, 9, 8, 0, 0).unwrap());
}

#[tokio::test]
async fn a_schedule_that_would_never_run_is_refused_on_create_and_on_change() {
    let (_d, store) = temp_store().await;
    for (name, past) in [
        (
            "once",
            json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}),
        ),
        (
            "until",
            json!({
                "rule": {"kind": "daily", "time": "09:00"},
                "until": {"date": "2020-01-01T09:00"}
            }),
        ),
    ] {
        let mut s = spec(name);
        s.schedule = Some(schedule(past));
        let err = store.create_dot(&s, None).await.unwrap_err();
        assert!(never_runs(&err), "{name}: {err:?}");
    }
    assert!(store.list_dots().await.unwrap().is_empty());

    let mut s = spec("later");
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "once", "at": "2100-01-01T09:00"}}),
    ));
    let dot = store.create_dot(&s, None).await.unwrap();
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}),
    ));
    let err = store.update_dot(&dot.id, &s).await.unwrap_err();
    assert!(never_runs(&err), "{err:?}");
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);
}

#[tokio::test]
async fn a_stored_schedule_whose_date_passed_loads_and_saves_unchanged() {
    let (dir, store) = temp_store().await;
    let mut s = spec("once");
    s.schedule = Some(schedule(
        json!({"rule": {"kind": "once", "at": "2100-01-01T09:00"}}),
    ));
    let dot = store.create_dot(&s, None).await.unwrap();
    // The date passes (written behind the store's back, as time would do it).
    sqlx::query("UPDATE dots SET schedule = ? WHERE id = ?")
        .bind(r#"{"rule":{"kind":"once","at":"2020-01-01T09:00"},"catch_up":true,"until":null}"#)
        .bind(&dot.id)
        .execute(&raw_pool(&dir).await)
        .await
        .unwrap();
    let loaded = store.get_dot(&dot.id).await.unwrap();
    let past = schedule(json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}));
    assert_eq!(loaded.spec.schedule.as_ref(), Some(&past));

    // A change to something else saves: the schedule did not change, so it is not checked.
    let mut s = loaded.spec.clone();
    s.instructions = "Something else.".into();
    let saved = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(saved.spec.instructions, "Something else.");
    assert_eq!(saved.spec.schedule, Some(past));
}

#[tokio::test]
async fn the_core_sets_the_anchor_of_an_every_schedule() {
    fn anchor(dot: &Dot) -> chrono::DateTime<Utc> {
        match &dot.spec.schedule.as_ref().unwrap().rule {
            Rule::Every {
                anchor: Some(a), ..
            } => *a,
            other => panic!("no anchor: {other:?}"),
        }
    }
    let (_d, store) = temp_store().await;
    let mut s = spec("every");
    // As the UI sends it: no anchor.
    s.schedule = Some(schedule(json!({"rule": {"kind": "every", "minutes": 30}})));
    let before = Utc::now();
    let dot = store.create_dot(&s, None).await.unwrap();
    let first = anchor(&dot);
    assert!(first >= before - TimeDelta::milliseconds(1) && first <= Utc::now());
    assert_eq!(first.timestamp_subsec_nanos() % 1_000_000, 0, "whole ms");
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);

    // Saved again without an anchor: the phase stays.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let saved = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(anchor(&saved), first);

    // New minutes: a new phase.
    s.schedule = Some(schedule(json!({"rule": {"kind": "every", "minutes": 60}})));
    let changed = store.update_dot(&dot.id, &s).await.unwrap();
    assert!(anchor(&changed) > first);
}

/// Migration 0007 on a database Dots left at 0006: every stored policy gets a `Context` allow
/// rule unless a rule names `Context`, and every dot starts with the global instructions.
#[tokio::test]
async fn migration_0007_lets_every_stored_policy_read_context() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 7)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0006 left it: the preset's rules without `Context`, without the check-in
    // tools, which 0008 adds after it, without `Memory`, which 0012 adds after them, and without
    // `Workflow`, which 0014 adds last.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| {
            r.tool != "Context"
                && r.tool != "Memory"
                && r.tool != "Workflow"
                && !dots_core::policy::CHECK_IN_TOOLS.contains(&r.tool.as_str())
        });
        p
    };
    let mut denies = old(Preset::Custom);
    denies.rules.push(dots_core::policy::Rule::new(
        "Context(kb)",
        dots_core::policy::Action::Deny,
    ));
    for (id, name, policy) in [
        ("D1", "sandboxed", old(Preset::Sandboxed)),
        ("D2", "read-only", old(Preset::ReadOnly)),
        ("D3", "denies", denies.clone()),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(serde_json::to_string(&policy).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    for (id, preset) in [("D1", Preset::Sandboxed), ("D2", Preset::ReadOnly)] {
        let dot = store.get_dot(id).await.unwrap();
        assert_eq!(dot.spec.policy, Policy::preset(preset), "{id}");
        assert!(dot.spec.use_global_instructions);
        assert!(dot.spec.uses.context.is_empty());
    }
    // A rule that names `Context` already is the person's: nothing is added.
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, denies);
}

/// Migration 0013 on a database Dots left at 0012: every stored dot is Personal, and no org is
/// joined (env vault spec §5.2).
#[tokio::test]
async fn migration_0013_leaves_every_stored_dot_in_personal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 13)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
         webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
         created_at, updated_at) VALUES ('D1', 'old', 'Do it.', 'claude', 'sonnet', \
         'C:/tmp/repo', 'worktree', '', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(serde_json::to_string(&Policy::preset(Preset::Sandboxed)).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let old = store.get_dot("D1").await.unwrap();
    assert_eq!(old.org_id, None);
    assert_eq!(old.spec.name, "old");
    assert!(store.list_orgs().await.unwrap().is_empty());
    let made = store
        .create_dot(&spec("new"), Some("acme-eng"))
        .await
        .unwrap();
    assert_eq!(made.org_id.as_deref(), Some("acme-eng"));
    assert_eq!(store.get_dot("D1").await.unwrap(), old);
}

/// Migration 0005 on a database that Dots left with a cron string (Scheduler v2 spec §4).
#[tokio::test]
async fn migration_0005_turns_a_cron_string_into_a_schedule_without_catch_up() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 5)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    let policy = serde_json::to_string(&Policy::preset(Preset::Sandboxed)).unwrap();
    for (id, name, schedule) in [
        ("D1", "cron", Some("0 0 9 * * Mon-Fri")),
        ("D2", "manual", None),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             schedule, webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', ?, 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(schedule)
        .bind(&policy)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let cron = store.get_dot("D1").await.unwrap().spec.schedule.unwrap();
    assert_eq!(cron, Schedule::cron("0 0 9 * * Mon-Fri"));
    assert!(!cron.catch_up);
    assert_eq!(store.get_dot("D2").await.unwrap().spec.schedule, None);

    let pool = raw_pool(&dir).await;
    let raw: String = sqlx::query_scalar("SELECT schedule FROM dots WHERE id = 'D1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        raw,
        r#"{"rule":{"kind":"cron","expr":"0 0 9 * * Mon-Fri"},"catch_up":false,"until":null}"#
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schedule_state")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 0,
        "schedule_state is empty until the first pass or save"
    );
    let retries: Vec<Option<String>> = sqlx::query_scalar("SELECT retry_of FROM runs")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(retries.is_empty());
}

#[test]
fn timeout_and_approval_wait_are_capped() {
    let mut s = spec("caps");
    s.timeout_secs = 604_800;
    s.approval_wait_secs = 280;
    s.validate().unwrap();
    s.timeout_secs = 604_801;
    assert!(matches!(s.validate(), Err(Error::Invalid(_))));
    s.timeout_secs = 1800;
    s.approval_wait_secs = 281;
    assert!(matches!(s.validate(), Err(Error::Invalid(_))));
}

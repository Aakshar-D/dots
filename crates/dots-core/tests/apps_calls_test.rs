//! Calling an installed app: the lock, the log, the event (app library spec section 5).

mod common;

use std::sync::Arc;

use chrono::{Local, SecondsFormat, TimeZone, Utc};
use common::apps::{expenses, write_app};
use common::clock::FakeClock;
use dots_core::apps::call::Actor;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::{Bus, RuntimeEvent};
use dots_core::library::{AppOrigin, Uses};
use dots_core::store::Store;
use serde_json::{json, Value};

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    bus: Bus,
    apps: Apps,
}

async fn fixture(files: &[(&str, String)]) -> Fixture {
    let (dir, store) = common::temp_store().await;
    let bus = Bus::new(64);
    let clock = FakeClock::new(Utc.with_ymd_and_hms(2026, 10, 23, 3, 42, 0).unwrap());
    let apps = Apps::new(store.clone(), bus.clone(), dir.path().join("apps")).with_clock(clock);
    let src_dir = tempfile::tempdir().unwrap();
    write_app(src_dir.path(), files);
    apps.install(read_folder(src_dir.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    Fixture {
        _dir: dir,
        store,
        bus,
        apps,
    }
}

fn dot() -> Actor {
    Actor::Dot {
        id: "d1".into(),
        name: "builder".into(),
        run_id: "r1".into(),
    }
}

fn expense(item: &str) -> Value {
    json!({"item": item, "amount": 2, "on": "2026-10-22"})
}

#[tokio::test]
async fn a_person_and_a_dot_work_on_the_same_data() {
    let f = fixture(&expenses()).await;
    f.apps
        .call("expenses", "add_expense", expense("Coffee"), Actor::Person)
        .await
        .unwrap();
    let rows = f
        .apps
        .call(
            "expenses",
            "list_expenses",
            json!({"month": "2026-10"}),
            dot(),
        )
        .await
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["item"], "Coffee");
}

#[tokio::test]
async fn a_write_is_logged_with_who_and_what_and_a_read_is_not() {
    let f = fixture(&expenses()).await;
    let out = f
        .apps
        .call("expenses", "add_expense", expense("Coffee"), dot())
        .await
        .unwrap();
    f.apps
        .call(
            "expenses",
            "month_total",
            json!({"month": "2026-10"}),
            dot(),
        )
        .await
        .unwrap();
    let (item, _) = f.apps.get("expenses").await.unwrap();
    let log = f.store.app_calls(&item.id, 10).await.unwrap();
    assert_eq!(log.len(), 1);
    let c = &log[0];
    assert_eq!(c.tool, "add_expense");
    assert_eq!(c.actor, dot());
    assert_eq!(c.run_id.as_deref(), Some("r1"));
    assert_eq!(c.input, expense("Coffee"));
    assert_eq!(c.result, Some(out.clone()));
    assert_eq!(c.ids, vec![out["id"].as_str().unwrap().to_string()]);
    assert_eq!((c.changes, c.error.as_deref()), (1, None));
    let local = Utc
        .with_ymd_and_hms(2026, 10, 23, 3, 42, 0)
        .unwrap()
        .with_timezone(&Local);
    assert_eq!(c.now, local.to_rfc3339_opts(SecondsFormat::Secs, false));
}

#[tokio::test]
async fn a_failed_write_is_logged_with_its_error() {
    let f = fixture(&expenses()).await;
    let err = f
        .apps
        .call(
            "expenses",
            "add_expense",
            json!({"item": "x"}),
            Actor::Person,
        )
        .await
        .unwrap_err();
    assert_eq!(err, "input.amount is required");
    let (item, _) = f.apps.get("expenses").await.unwrap();
    let log = f.store.app_calls(&item.id, 10).await.unwrap();
    assert_eq!(log[0].error.as_deref(), Some("input.amount is required"));
    assert_eq!((log[0].changes, log[0].result.clone()), (0, None));
}

#[tokio::test]
async fn a_change_tells_the_screen() {
    let f = fixture(&expenses()).await;
    let mut events = f.bus.subscribe();
    f.apps
        .call(
            "expenses",
            "month_total",
            json!({"month": "2026-10"}),
            Actor::Person,
        )
        .await
        .unwrap();
    f.apps
        .call("expenses", "add_expense", expense("Coffee"), dot())
        .await
        .unwrap();
    match events.try_recv() {
        Ok(RuntimeEvent::AppDataChanged { app }) => assert_eq!(app, "expenses"),
        other => panic!("expected AppDataChanged, got {other:?}"),
    }
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn an_unknown_app_or_tool_is_named() {
    let f = fixture(&expenses()).await;
    assert_eq!(
        f.apps
            .call("nope", "x", json!({}), Actor::Person)
            .await
            .unwrap_err(),
        "there is no app nope"
    );
    assert_eq!(
        f.apps
            .call("expenses", "drop_all", json!({}), Actor::Person)
            .await
            .unwrap_err(),
        "the app has no tool drop_all"
    );
}

#[tokio::test]
async fn the_app_sees_the_time_and_the_dots_that_use_it() {
    let mut manifest: Value = serde_json::from_str(common::apps::MANIFEST).unwrap();
    manifest["tools"].as_array_mut().unwrap().push(json!({
        "name": "whoami", "description": "who", "input": {"type": "object"}
    }));
    let logic = format!(
        "{}\nexport function whoami(_, ctx) {{ return {{ now: ctx.now, dots: ctx.dots.list() }}; }}",
        common::apps::LOGIC
    );
    let mut files = common::apps::expenses_with("app.json", Some(&manifest.to_string()));
    files.retain(|(p, _)| *p != "logic.js");
    files.push(("logic.js", logic));
    let f = fixture(&files).await;
    let mut spec = common::spec("builder");
    spec.uses = Uses {
        apps: vec!["expenses".into()],
        ..Uses::default()
    };
    let b = f.store.create_dot(&spec).await.unwrap();
    f.store.create_dot(&common::spec("other")).await.unwrap();
    let out = f
        .apps
        .call("expenses", "whoami", json!({}), Actor::Person)
        .await
        .unwrap();
    assert_eq!(out["dots"], json!([{"id": b.id, "name": "builder"}]));
    let local = Utc
        .with_ymd_and_hms(2026, 10, 23, 3, 42, 0)
        .unwrap()
        .with_timezone(&Local);
    assert_eq!(
        out["now"],
        local.to_rfc3339_opts(SecondsFormat::Secs, false)
    );
}

#[tokio::test]
async fn calls_to_one_app_take_turns() {
    let f = fixture(&expenses()).await;
    let apps = Arc::new(f.apps.clone());
    let calls: Vec<_> = (0..20)
        .map(|i| {
            let apps = apps.clone();
            tokio::spawn(async move {
                apps.call(
                    "expenses",
                    "add_expense",
                    expense(&format!("e{i}")),
                    Actor::Person,
                )
                .await
            })
        })
        .collect();
    for c in calls {
        c.await.unwrap().unwrap();
    }
    let total = f
        .apps
        .call(
            "expenses",
            "month_total",
            json!({"month": "2026-10"}),
            Actor::Person,
        )
        .await
        .unwrap();
    assert_eq!(total, json!(40));
}

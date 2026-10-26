//! The person's side of the apps (app library spec sections 5, 6.2 and 10): the Apps screen's
//! rows, the screen's calls as the person, the files, and the call log.

mod common;

use common::apps::{expenses, write_app};
use common::spec;
use dots_core::apps::call::Actor;
use dots_core::apps::screen::MAX_CALLS_LISTED;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Store, Apps) {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let src = tempfile::tempdir().unwrap();
    write_app(src.path(), &expenses());
    apps.install(read_folder(src.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    (dir, store, apps)
}

#[tokio::test]
async fn the_rows_name_the_dots_and_the_error_that_keeps_an_app_out() {
    let (dir, store, apps) = fixture().await;
    let mut s = spec("builder");
    s.uses.apps = vec!["expenses".into()];
    store.create_dot(&s, None).await.unwrap();
    store.create_dot(&spec("other"), None).await.unwrap();
    let rows = apps.rows().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.name, "expenses");
    assert_eq!(rows[0].dots, ["builder"]);
    assert_eq!(rows[0].error, None);

    // A code folder broken outside Dots shows on the row of a fresh start (spec section 11).
    let (item, config) = apps.get("expenses").await.unwrap();
    std::fs::write(
        apps.code_dir(&item.id, config.version).join("app.json"),
        "{",
    )
    .unwrap();
    let fresh = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let rows = fresh.rows().await.unwrap();
    assert!(
        rows[0].error.as_deref().unwrap().contains("app.json"),
        "{rows:?}"
    );
}

#[tokio::test]
async fn the_screen_calls_as_the_person_and_only_listed_tools() {
    let (_dir, store, apps) = fixture().await;
    let out = apps
        .screen_call(
            "expenses",
            "add_expense",
            json!({"item": "Coffee", "amount": 4.5, "on": "2026-10-22"}),
        )
        .await
        .unwrap();
    assert!(out["id"].is_string());
    let (item, _) = apps.get("expenses").await.unwrap();
    let logged = store.app_calls(&item.id, 10).await.unwrap();
    assert_eq!(logged[0].actor, Actor::Person);
    let unlisted = apps.screen_call("expenses", "run_ended", json!({})).await;
    match unlisted {
        Err(Error::Invalid(m)) => assert_eq!(m, "the app has no tool run_ended"),
        other => panic!("expected invalid, got {other:?}"),
    }
    let bad = apps
        .screen_call("expenses", "add_expense", json!({"item": "Tea"}))
        .await;
    assert!(matches!(bad, Err(Error::Invalid(_))), "{bad:?}");
    assert_eq!(apps.calls("expenses", 10).await.unwrap().len(), 2);
    assert_eq!(MAX_CALLS_LISTED, 500);
}

#[tokio::test]
async fn an_apps_files_are_its_active_version() {
    let (_dir, _store, apps) = fixture().await;
    let files = apps.files("expenses").await.unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "app.json",
            "logic.js",
            "migrations/0001_expenses.sql",
            "sample.sql",
            "tests/expenses.test.js",
            "ui/app.js",
            "ui/index.html"
        ]
    );
    assert!(files.iter().all(|f| f.text.is_some() && f.base64.is_none()));
    assert!(matches!(
        apps.proposal_files("01NOPE").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn a_proposal_id_is_no_path() {
    let (_dir, _store, apps) = fixture().await;
    let (item, config) = apps.get("expenses").await.unwrap();
    let id = format!("../{}/code/{}", item.id, config.version);
    // The folder is there: only the id check keeps the app's code out of the card.
    assert!(apps.proposed_dir(&id).is_dir());
    assert!(matches!(
        apps.proposal_files(&id).await,
        Err(Error::NotFound(_))
    ));
}

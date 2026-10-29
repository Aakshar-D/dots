//! The data an app's removal kept (app library spec section 10): listed until the person deletes
//! it, and offered to a new app of the same name whose migrations keep the ones that ran on it.

mod common;

use common::apps::{expenses, expenses_with, proposing_run, write_app, MIGRATION};
use dots_core::apps::call::Actor;
use dots_core::apps::propose::{AppProposal, SENT};
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::model::{NewRun, TriggerKind};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Store, Apps) {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let src = dir.path().join("first");
    write_app(&src, &expenses());
    apps.install(read_folder(&src).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let ink = json!({ "item": "Ink", "amount": 3, "on": "2026-10-20" });
    apps.call("expenses", "add_expense", ink, Actor::Person)
        .await
        .unwrap();
    (dir, store, apps)
}

async fn proposed(
    dir: &tempfile::TempDir,
    store: &Store,
    apps: &Apps,
    files: &[(&str, String)],
) -> AppProposal {
    let n = store.list_dots().await.unwrap().len();
    let run = proposing_run(store, &format!("builder{n}")).await;
    let folder = dir.path().join(format!("work-{n}"));
    write_app(&folder, files);
    assert_eq!(
        apps.propose(&run.id, folder, "Again.").await,
        Ok(SENT.to_string())
    );
    store
        .list_app_proposals(Some(&run.dot_id))
        .await
        .unwrap()
        .remove(0)
}

async fn items(apps: &Apps) -> Vec<String> {
    let month = json!({ "month": "2026-10" });
    let rows = apps
        .call("expenses", "list_expenses", month, Actor::Person)
        .await
        .unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .map(|r| r["item"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn removing_an_app_that_keeps_its_data_lists_it_until_it_is_deleted() {
    let (_dir, _store, apps) = fixture().await;
    let (item, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let kept = apps.kept().await.unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].id, item.id);
    assert_eq!(
        (kept[0].name.as_str(), kept[0].title.as_str()),
        ("expenses", "Expenses")
    );
    assert!(kept[0].bytes > 0);
    assert!(apps.data_path(&item.id).exists());
    assert!(!apps.app_dir(&item.id).join("code").exists());

    apps.delete_kept(&item.id).await.unwrap();
    assert!(apps.kept().await.unwrap().is_empty());
    assert!(!apps.app_dir(&item.id).exists());
    let again = apps.delete_kept(&item.id).await;
    assert!(matches!(again, Err(Error::NotFound(_))), "{again:?}");
}

#[tokio::test]
async fn removing_an_app_with_its_data_keeps_nothing() {
    let (_dir, _store, apps) = fixture().await;
    let (item, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, true).await.unwrap();
    assert!(apps.kept().await.unwrap().is_empty());
    assert!(!apps.app_dir(&item.id).exists());
}

#[tokio::test]
async fn a_new_app_that_keeps_the_migrations_is_offered_the_kept_data_and_starts_on_it() {
    let (dir, store, apps) = fixture().await;
    let (old, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let mut files = expenses();
    files.push((
        "migrations/0002_categories.sql",
        "CREATE TABLE categories (name TEXT PRIMARY KEY);\n".to_string(),
    ));
    let p = proposed(&dir, &store, &apps, &files).await;
    let offer = p
        .summary
        .kept_data
        .clone()
        .expect("the kept data is offered");
    assert_eq!(offer.id, old.id);
    assert_eq!(offer.migrations, ["0002_categories.sql"]);

    let item = apps.accept(&p, true).await.unwrap();
    assert_ne!(item.id, old.id);
    assert_eq!(items(&apps).await, ["Ink"], "the kept row is there");
    assert!(apps.kept().await.unwrap().is_empty());
    assert!(!apps.app_dir(&old.id).exists());
    let gone = apps.delete_kept(&old.id).await;
    assert!(matches!(gone, Err(Error::NotFound(_))), "{gone:?}");
}

#[tokio::test]
async fn kept_data_that_does_not_fit_is_not_offered_and_the_app_starts_empty() {
    let (dir, store, apps) = fixture().await;
    apps.remove("expenses", true, false).await.unwrap();
    let changed = MIGRATION.replace("added_by TEXT NOT NULL", "added_by TEXT");
    let files = expenses_with("migrations/0001_expenses.sql", Some(&changed));
    let p = proposed(&dir, &store, &apps, &files).await;
    assert_eq!(p.summary.kept_data, None);
    let refused = apps.accept(&p, true).await;
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    apps.accept(&p, false).await.unwrap();
    assert!(items(&apps).await.is_empty());
    assert_eq!(
        apps.kept().await.unwrap().len(),
        1,
        "the old data is still kept"
    );
}

#[tokio::test]
async fn kept_data_deleted_after_the_card_was_posted_is_not_started_on() {
    let (dir, store, apps) = fixture().await;
    let (old, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let p = proposed(&dir, &store, &apps, &expenses()).await;
    assert!(p.summary.kept_data.is_some());
    apps.delete_kept(&old.id).await.unwrap();
    let refused = apps.accept(&p, true).await;
    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    assert!(apps.list().await.unwrap().is_empty(), "nothing was added");
}

#[tokio::test]
async fn removing_an_app_forgets_its_topics() {
    let (_dir, store, apps) = fixture().await;
    let (item, _) = apps.get("expenses").await.unwrap();
    let dot = store
        .create_dot(&common::spec("helper"), None, "default")
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::App);
    new.payload = Some(json!({
        "message": "Hi", "app": "expenses", "app_title": "Expenses", "tag": "t1"
    }));
    store
        .start_app_run(&item.id, "t1", &new, "2026-10-27T09:00:00Z")
        .await
        .unwrap();
    assert_eq!(store.open_app_topics(&item.id).await.unwrap().len(), 1);
    apps.remove("expenses", true, false).await.unwrap();
    assert!(store.open_app_topics(&item.id).await.unwrap().is_empty());
    assert_eq!(store.prune_app_topics().await.unwrap(), 0);
}

#[tokio::test]
async fn a_failed_migration_on_kept_data_moves_it_back_and_keeps_it_listed() {
    let (dir, store, apps) = fixture().await;
    let (old, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let mut files = expenses();
    files.push((
        "migrations/0002_twice.sql",
        "INSERT INTO expenses SELECT * FROM expenses;\n".to_string(),
    ));
    let p = proposed(&dir, &store, &apps, &files).await;
    assert!(p.summary.kept_data.is_some());
    let failed = apps.accept(&p, true).await;
    assert!(failed.is_err(), "{failed:?}");
    assert!(apps.list().await.unwrap().is_empty(), "nothing was added");
    let kept = apps.kept().await.unwrap();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].id, old.id);
    assert!(kept[0].bytes > 0);

    let again = proposed(&dir, &store, &apps, &expenses()).await;
    let item = apps.accept(&again, true).await.unwrap();
    assert_ne!(item.id, old.id);
    assert_eq!(items(&apps).await, ["Ink"]);
}

#[tokio::test]
async fn the_offered_kept_data_is_started_on_though_newer_kept_data_appeared() {
    let (dir, store, apps) = fixture().await;
    let (old, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();
    let p = proposed(&dir, &store, &apps, &expenses()).await;
    assert_eq!(
        p.summary.kept_data.as_ref().map(|k| k.id.clone()),
        Some(old.id.clone())
    );
    apps.install(
        read_folder(&dir.path().join("first")).unwrap(),
        AppOrigin::Proposed,
    )
    .await
    .unwrap();
    let (newer, _) = apps.get("expenses").await.unwrap();
    apps.remove("expenses", true, false).await.unwrap();

    apps.accept(&p, true).await.unwrap();
    assert_eq!(
        items(&apps).await,
        ["Ink"],
        "the offered data was started on"
    );
    let kept: Vec<String> = apps
        .kept()
        .await
        .unwrap()
        .into_iter()
        .map(|k| k.id)
        .collect();
    assert_eq!(kept, [newer.id], "only the newer kept data is left");
}

#[tokio::test]
async fn kept_data_is_offered_only_to_a_new_app() {
    let (dir, store, apps) = fixture().await;
    let mut files = expenses();
    files.push((
        "migrations/0002_categories.sql",
        "CREATE TABLE categories (name TEXT PRIMARY KEY);\n".to_string(),
    ));
    let p = proposed(&dir, &store, &apps, &files).await;
    assert!(p.base_version.is_some());
    let refused = apps.accept(&p, true).await;
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    apps.accept(&p, false).await.unwrap();
}

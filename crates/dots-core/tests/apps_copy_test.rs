//! `copy_app` (app library spec section 15): a dot copies an installed app's active version, as
//! Dots keeps it, into its workspace, so a change of it keeps the migrations that ran.

mod common;

use common::apps::{expenses, proposing_run, write_app};
use dots_core::apps::propose::SENT;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;

#[tokio::test]
async fn a_copy_is_the_active_version_as_kept_and_a_change_made_from_it_passes() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    let item = apps
        .install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let copy = dir.path().join("work").join("expenses");
    let said = apps
        .copy_to("expenses", copy.clone(), "expenses")
        .await
        .unwrap();
    assert!(
        said.starts_with("Copied version 1 of the app expenses into expenses."),
        "{said}"
    );
    let kept = apps.code_dir(&item.id, 1);
    for entry in [
        "app.json",
        "logic.js",
        "ui/index.html",
        "migrations/0001_expenses.sql",
    ] {
        assert_eq!(
            std::fs::read(copy.join(entry)).unwrap(),
            std::fs::read(kept.join(entry)).unwrap(),
            "{entry}"
        );
    }
    std::fs::write(
        copy.join("migrations/0002_categories.sql"),
        "CREATE TABLE categories (name TEXT PRIMARY KEY);\n",
    )
    .unwrap();
    let run = proposing_run(&store, "fixer").await;
    assert_eq!(
        apps.propose(&run.id, copy, "Categories.").await,
        Ok(SENT.to_string())
    );
}

#[tokio::test]
async fn copying_refuses_an_unknown_app_and_a_folder_in_use() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    assert_eq!(
        apps.copy_to("nothing", dir.path().join("a"), "a").await,
        Err("there is no app nothing".to_string())
    );
    let used = dir.path().join("used");
    std::fs::create_dir_all(&used).unwrap();
    std::fs::write(used.join("notes.md"), "mine").unwrap();
    assert_eq!(
        apps.copy_to("expenses", used.clone(), "used").await,
        Err("the folder used is not empty: copy the app into a new folder".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(used.join("notes.md")).unwrap(),
        "mine"
    );
}

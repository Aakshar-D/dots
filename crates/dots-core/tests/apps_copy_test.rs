//! `copy_app` (app library spec section 15): a dot copies an installed app's active version, as
//! Dots keeps it, into its workspace, so a change of it keeps the migrations that ran.

mod common;

use common::apps::{expenses, proposing_run, write_app};
use dots_core::apps::propose::SENT;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::model::{NewRun, Run, TriggerKind};
use dots_core::policy::{Policy, Preset};
use dots_core::store::Store;

/// A run of a dot on `preset`, its workspace the folder `ws`.
async fn run_in(store: &Store, preset: Preset, ws: &std::path::Path) -> Run {
    std::fs::create_dir_all(ws).unwrap();
    let mut spec = common::spec(&format!("copier-{preset:?}"));
    spec.policy = Policy::preset(preset);
    let dot = store.create_dot(&spec, None).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Manual);
    new.workspace_path = Some(ws.to_string_lossy().into_owned());
    store.create_run(&new).await.unwrap()
}

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
    let ws = dir.path().join("work");
    let run = run_in(&store, Preset::Trusted, &ws).await;
    let copy = ws.join("expenses");
    let said = apps
        .copy_to(&run.id, "expenses", copy.clone(), "expenses")
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
    let ws = dir.path().join("ws");
    let run = run_in(&store, Preset::Trusted, &ws).await;
    assert_eq!(
        apps.copy_to(&run.id, "nothing", ws.join("a"), "a").await,
        Err("there is no app nothing".to_string())
    );
    let used = ws.join("used");
    std::fs::create_dir_all(&used).unwrap();
    std::fs::write(used.join("notes.md"), "mine").unwrap();
    assert_eq!(
        apps.copy_to(&run.id, "expenses", used.clone(), "used")
            .await,
        Err("the folder used is not empty: copy the app into a new folder".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(used.join("notes.md")).unwrap(),
        "mine"
    );
    // A file is named as a file; an empty folder is accepted.
    let file = ws.join("plain.txt");
    std::fs::write(&file, "text").unwrap();
    assert_eq!(
        apps.copy_to(&run.id, "expenses", file.clone(), "plain.txt")
            .await,
        Err("plain.txt is a file: copy the app into a new folder".to_string())
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "text");
    let empty = ws.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    apps.copy_to(&run.id, "expenses", empty.clone(), "empty")
        .await
        .unwrap();
    assert!(empty.join("app.json").is_file());
}

#[tokio::test]
async fn copying_is_refused_where_the_dots_rules_forbid_writes() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();

    let ro = dir.path().join("ro");
    let run = run_in(&store, Preset::ReadOnly, &ro).await;
    assert_eq!(
        apps.copy_to(&run.id, "expenses", ro.join("copy"), "copy")
            .await,
        Err(
            "your rules do not let you write to copy, so the app cannot be copied there"
                .to_string()
        )
    );
    assert!(!ro.join("copy").exists());

    let sb = dir.path().join("sb");
    let run = run_in(&store, Preset::Sandboxed, &sb).await;
    assert_eq!(
        apps.copy_to(&run.id, "expenses", sb.join(".claude/copy"), ".claude/copy")
            .await,
        Err(
            "your rules do not let you write to .claude/copy, so the app cannot be copied there"
                .to_string()
        )
    );
    assert!(!sb.join(".claude").exists());
    apps.copy_to(&run.id, "expenses", sb.join("copy"), "copy")
        .await
        .unwrap();
    assert!(sb.join("copy/app.json").is_file());
}

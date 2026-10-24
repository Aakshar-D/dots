//! The installed apps: install, update, roll back, the switches, remove (app library spec
//! sections 4, 6, 8 and 10).

mod common;

use common::apps::{expenses, write_app};
use dots_core::apps::data;
use dots_core::apps::source::{read_folder, AppSource};
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::{AppConfig, AppOrigin, LibraryConfig, LibraryKind, Uses};
use dots_core::library_service::{LibraryService, APPS_ELSEWHERE};
use dots_core::store::Store;
use dots_core::Error;

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    apps: Apps,
}

async fn fixture() -> Fixture {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(64), dir.path().join("apps"));
    Fixture {
        _dir: dir,
        store,
        apps,
    }
}

fn source(files: &[(&str, String)]) -> AppSource {
    let dir = tempfile::tempdir().unwrap();
    write_app(dir.path(), files);
    read_folder(dir.path()).unwrap()
}

/// Expenses with `title` and, when `more` is set, a second migration.
fn expenses_v2(title: &str, more: bool) -> AppSource {
    let mut files = expenses();
    let manifest = common::apps::MANIFEST.replace(
        "\"title\": \"Expenses\"",
        &format!("\"title\": \"{title}\""),
    );
    files.retain(|(p, _)| *p != "app.json");
    files.push(("app.json", manifest));
    if more {
        files.push((
            "migrations/0002_note.sql",
            "ALTER TABLE expenses ADD COLUMN note TEXT;".into(),
        ));
    }
    source(&files)
}

fn config(item: &dots_core::library::LibraryItem) -> AppConfig {
    match &item.config {
        LibraryConfig::App(c) => c.clone(),
        other => panic!("not an app: {other:?}"),
    }
}

#[tokio::test]
async fn an_app_installs_at_version_1_with_both_switches_off() {
    let f = fixture().await;
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    assert_eq!(item.kind(), LibraryKind::App);
    assert_eq!(item.name, "expenses");
    let c = config(&item);
    assert_eq!(
        c,
        AppConfig {
            title: "Expenses".into(),
            description: "Logs expenses and totals them by month".into(),
            version: 1,
            previous_version: None,
            every_dot: false,
            can_start_dots: false,
            origin: AppOrigin::Proposed,
            changed: false,
        }
    );
    assert!(f.apps.code_dir(&item.id, 1).join("logic.js").is_file());
    let conn = data::open(&f.apps.data_path(&item.id)).unwrap();
    assert_eq!(data::schema_version(&conn).unwrap(), 1);
    let loaded = f.apps.load(&item, &c).await.unwrap();
    assert_eq!(loaded.manifest.name, "expenses");
    assert_eq!(loaded.ui.len(), 2);
}

#[tokio::test]
async fn a_name_the_library_has_is_a_conflict() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let err = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err}");
}

#[tokio::test]
async fn a_failed_install_leaves_nothing() {
    let f = fixture().await;
    let mut files = expenses();
    files.push((
        "migrations/0002_bad.sql",
        "CREATE TABLE expenses (x);".into(),
    ));
    let err = f
        .apps
        .install(source(&files), AppOrigin::Proposed)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid input: migrations/0002_bad.sql: table expenses already exists"
    );
    assert!(f.apps.list().await.unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(f._dir.path().join("apps"))
            .map(|d| d.count())
            .unwrap_or(0),
        0
    );
}

#[tokio::test]
async fn an_update_without_migrations_keeps_the_version_before() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let item = f
        .apps
        .update("expenses", expenses_v2("Spending", false), true)
        .await
        .unwrap();
    let c = config(&item);
    assert_eq!(
        (c.version, c.previous_version, c.title.as_str()),
        (2, Some(1), "Spending")
    );
    assert!(f.apps.code_dir(&item.id, 1).is_dir());
    let item = f.apps.roll_back("expenses").await.unwrap();
    let c = config(&item);
    assert_eq!(
        (c.version, c.previous_version, c.title.as_str()),
        (1, None, "Expenses")
    );
    assert!(!f.apps.code_dir(&item.id, 2).exists());
    let err = f.apps.roll_back("expenses").await.unwrap_err().to_string();
    assert!(
        err.contains("roll back is offered only when the last update changed no data schema"),
        "{err}"
    );
}

#[tokio::test]
async fn an_update_with_migrations_runs_them_and_cannot_be_rolled_back() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let item = f
        .apps
        .update("expenses", expenses_v2("Expenses", true), true)
        .await
        .unwrap();
    let c = config(&item);
    assert_eq!((c.version, c.previous_version), (2, None));
    assert!(!f.apps.code_dir(&item.id, 1).exists());
    let conn = data::open(&f.apps.data_path(&item.id)).unwrap();
    assert_eq!(data::schema_version(&conn).unwrap(), 2);
    assert!(f.apps.roll_back("expenses").await.is_err());
}

#[tokio::test]
async fn a_failed_migration_on_update_changes_nothing() {
    let f = fixture().await;
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let mut files = expenses();
    files.push((
        "migrations/0002_bad.sql",
        "ALTER TABLE nope ADD COLUMN x;".into(),
    ));
    let err = f
        .apps
        .update("expenses", source(&files), true)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("migrations/0002_bad.sql"), "{err}");
    let (_, c) = f.apps.get("expenses").await.unwrap();
    assert_eq!(c.version, 1);
    assert!(!f.apps.code_dir(&item.id, 2).exists());
}

#[tokio::test]
async fn an_update_must_keep_the_name() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let renamed =
        common::apps::MANIFEST.replace("\"name\": \"expenses\"", "\"name\": \"spending\"");
    let src = source(&common::apps::expenses_with("app.json", Some(&renamed)));
    let err = f
        .apps
        .update("expenses", src, true)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "invalid input: the new code names the app spending, not expenses"
    );
}

#[tokio::test]
async fn a_bundled_app_changed_by_a_proposal_becomes_the_persons() {
    let f = fixture().await;
    let origin = AppOrigin::Bundled {
        release: "0.4.0".into(),
    };
    f.apps
        .install(source(&expenses()), origin.clone())
        .await
        .unwrap();
    let item = f
        .apps
        .update("expenses", expenses_v2("Expenses", false), false)
        .await
        .unwrap();
    assert!(!config(&item).changed);
    let item = f
        .apps
        .update("expenses", expenses_v2("Expenses", false), true)
        .await
        .unwrap();
    let c = config(&item);
    assert!(c.changed);
    assert_eq!(c.origin, origin);
}

#[tokio::test]
async fn can_start_dots_needs_the_code_to_ask_for_it() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let err = f
        .apps
        .set_can_start_dots("expenses", true)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "invalid input: the app expenses does not ask to start dots"
    );
    let mut manifest: serde_json::Value = serde_json::from_str(common::apps::MANIFEST).unwrap();
    manifest["permissions"] = serde_json::json!(["start_dots"]);
    let asks = source(&common::apps::expenses_with(
        "app.json",
        Some(&manifest.to_string()),
    ));
    f.apps.update("expenses", asks, true).await.unwrap();
    let item = f.apps.set_can_start_dots("expenses", true).await.unwrap();
    assert!(config(&item).can_start_dots);
    // Code that no longer asks turns the switch off.
    let item = f
        .apps
        .update("expenses", expenses_v2("Expenses", false), true)
        .await
        .unwrap();
    assert!(!config(&item).can_start_dots);
}

#[tokio::test]
async fn every_dot_gives_the_app_to_every_dot() {
    let f = fixture().await;
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let a = f.store.create_dot(&common::spec("a")).await.unwrap();
    let mut spec = common::spec("b");
    spec.uses = Uses {
        apps: vec!["expenses".into()],
        ..Uses::default()
    };
    let b = f.store.create_dot(&spec).await.unwrap();
    assert!(f.store.used_apps(&a.spec.uses).await.unwrap().is_empty());
    assert_eq!(f.store.used_apps(&b.spec.uses).await.unwrap().len(), 1);
    let names = |dots: Vec<dots_core::model::Dot>| {
        dots.into_iter().map(|d| d.spec.name).collect::<Vec<_>>()
    };
    assert_eq!(names(f.store.dots_with_app(&item).await.unwrap()), ["b"]);
    let item = f.apps.set_every_dot("expenses", true).await.unwrap();
    assert_eq!(f.store.used_apps(&a.spec.uses).await.unwrap().len(), 1);
    assert_eq!(
        names(f.store.dots_with_app(&item).await.unwrap()),
        ["a", "b"]
    );
}

#[tokio::test]
async fn a_dot_can_tick_only_an_installed_app() {
    let f = fixture().await;
    let mut spec = common::spec("a");
    spec.uses = Uses {
        apps: vec!["expenses".into()],
        ..Uses::default()
    };
    let err = f.store.create_dot(&spec).await.unwrap_err().to_string();
    assert_eq!(
        err,
        "invalid input: the Library has no app named \"expenses\""
    );
    spec.uses = Uses {
        apps: vec!["Bad Name".into()],
        ..Uses::default()
    };
    assert!(spec.uses.validate().is_err());
}

#[tokio::test]
async fn removing_asks_while_dots_use_it_and_keeps_the_data_unless_told() {
    let f = fixture().await;
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let mut spec = common::spec("b");
    spec.uses = Uses {
        apps: vec!["expenses".into()],
        ..Uses::default()
    };
    let b = f.store.create_dot(&spec).await.unwrap();
    let err = f.apps.remove("expenses", false, false).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "conflict: the app \"expenses\" is used by b; removing it takes it out of them"
    );
    f.apps.remove("expenses", true, false).await.unwrap();
    assert!(f.apps.get("expenses").await.is_err());
    assert!(f
        .store
        .get_dot(&b.id)
        .await
        .unwrap()
        .spec
        .uses
        .apps
        .is_empty());
    assert!(!f.apps.app_dir(&item.id).join("code").exists());
    assert!(f.apps.data_path(&item.id).is_file());
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    f.apps.remove("expenses", true, true).await.unwrap();
    assert!(!f.apps.app_dir(&item.id).exists());
}

#[tokio::test]
async fn the_library_commands_leave_apps_to_the_apps_screen() {
    let f = fixture().await;
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let library = LibraryService::new(f.store.clone(), Bus::new(8));
    let err = library
        .add("x", item.config.clone(), None)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), format!("invalid input: {APPS_ELSEWHERE}"));
    let err = library
        .update(&item.id, item.config.clone())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), format!("invalid input: {APPS_ELSEWHERE}"));
    let err = library.remove(&item.id, true).await.unwrap_err();
    assert_eq!(err.to_string(), format!("invalid input: {APPS_ELSEWHERE}"));
    let listed = library.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].item.name, "expenses");
}

#[cfg(windows)]
#[tokio::test]
async fn an_update_whose_code_cannot_be_written_leaves_the_data_alone() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = fixture().await;
    let item = f
        .apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    // A file in the new version's folder held open with no sharing: the write cannot finish.
    let tmp = f.apps.code_dir(&item.id, 2).with_extension("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let _held = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(tmp.join("app.json"))
        .unwrap();
    let updated = f
        .apps
        .update("expenses", expenses_v2("Expenses", true), true)
        .await;
    assert!(updated.is_err());
    let (item, c) = f.apps.get("expenses").await.unwrap();
    assert_eq!(c.version, 1);
    let conn = data::open(&f.apps.data_path(&item.id)).unwrap();
    assert_eq!(data::schema_version(&conn).unwrap(), 1);
    // The version before still loads.
    f.apps.load(&item, &c).await.unwrap();
}

#[tokio::test]
async fn removing_asks_while_every_dot_gives_the_app_to_dots() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    f.store.create_dot(&common::spec("a")).await.unwrap();
    f.apps.set_every_dot("expenses", true).await.unwrap();
    let err = f.apps.remove("expenses", false, false).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "conflict: the app \"expenses\" is used by a; removing it takes it out of them"
    );
    f.apps.remove("expenses", true, false).await.unwrap();
    assert!(f.apps.get("expenses").await.is_err());
}

#[tokio::test]
async fn changes_at_once_do_not_undo_each_other() {
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let (update, switch) = tokio::join!(
        f.apps
            .update("expenses", expenses_v2("Expenses 2", false), true),
        f.apps.set_every_dot("expenses", true),
    );
    update.unwrap();
    switch.unwrap();
    let (item, c) = f.apps.get("expenses").await.unwrap();
    assert_eq!(c.version, 2);
    assert!(c.every_dot);
    f.apps.load(&item, &c).await.unwrap();
    let (a, b) = tokio::join!(
        f.apps.update("expenses", expenses_v2("A", false), true),
        f.apps.update("expenses", expenses_v2("B", false), true),
    );
    a.unwrap();
    b.unwrap();
    let (item, c) = f.apps.get("expenses").await.unwrap();
    assert_eq!(c.version, 4);
    assert!(c.every_dot);
    f.apps.load(&item, &c).await.unwrap();
}

#[tokio::test]
async fn a_run_refuses_a_tool_that_began_to_write_after_it_started() {
    use dots_core::apps::RunApps;
    use dots_core::policy::{Action, Rule};
    let f = fixture().await;
    f.apps
        .install(source(&expenses()), AppOrigin::Proposed)
        .await
        .unwrap();
    let mut spec = common::spec("reader");
    spec.uses = Uses {
        apps: vec!["expenses".into()],
        ..Uses::default()
    };
    spec.policy
        .rules
        .push(Rule::new("App(expenses.write)", Action::Deny));
    let dot = f.store.create_dot(&spec).await.unwrap();
    let (run, _) = RunApps::open(&f.apps, &dot, "r1").await.unwrap();
    let run = run.unwrap();
    // The deny rule hides add_expense; month_total does not write, so the run offers it.
    assert!(run.resolve("expenses__add_expense").is_none());
    let checked = run
        .policy_input(
            "expenses__month_total",
            &serde_json::json!({"month": "2026-10"}),
        )
        .unwrap();
    assert!(run.call(&checked).await.is_ok());
    // The person accepts a version in which month_total writes.
    let mut files = expenses();
    files.retain(|(p, _)| *p != "app.json");
    files.push((
        "app.json",
        common::apps::MANIFEST.replace(
            "\"description\": \"The total of a month\",",
            "\"description\": \"The total of a month\",\n      \"writes\": true,",
        ),
    ));
    f.apps
        .update("expenses", source(&files), true)
        .await
        .unwrap();
    let err = run.call(&checked).await.unwrap_err();
    assert_eq!(
        err,
        "expenses__month_total: the app expenses changed during this run: month_total now \
         writes, and this run checked the dot's rules for it as a tool that does not; a new run \
         has the new version"
    );
}

#[tokio::test]
async fn a_claude_run_offers_at_most_its_bound_of_app_tools() {
    use dots_core::apps::run::MAX_CLAUDE_APP_TOOLS;
    use dots_core::apps::RunApps;
    let f = fixture().await;
    let big = |name: &str| {
        let tools: Vec<serde_json::Value> = (0..200)
            .map(|i| {
                serde_json::json!({
                    "name": format!("t{i}"), "description": "A tool", "input": {"type": "object"}
                })
            })
            .collect();
        let manifest = serde_json::json!({
            "name": name, "title": name, "description": "Many tools", "tools": tools
        });
        let logic: String = (0..200)
            .map(|i| format!("export function t{i}() {{ return {i}; }}\n"))
            .collect();
        source(&[
            ("app.json", manifest.to_string()),
            ("logic.js", logic),
            (
                "ui/index.html",
                "<!doctype html><title>x</title>".to_string(),
            ),
        ])
    };
    f.apps
        .install(big("alpha"), AppOrigin::Proposed)
        .await
        .unwrap();
    f.apps
        .install(big("beta"), AppOrigin::Proposed)
        .await
        .unwrap();
    let mut spec = common::spec("many");
    spec.engine = dots_core::model::EngineKind::Claude;
    spec.uses = Uses {
        apps: vec!["alpha".into(), "beta".into()],
        ..Uses::default()
    };
    let dot = f.store.create_dot(&spec).await.unwrap();
    let (run, notices) = RunApps::open(&f.apps, &dot, "r1").await.unwrap();
    let run = run.unwrap();
    assert_eq!(run.apps().len(), 1);
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(
        notices[0].ends_with(&format!(
            "left out of this run: a Claude run offers at most {MAX_CLAUDE_APP_TOOLS} app tools"
        )),
        "{notices:?}"
    );
}

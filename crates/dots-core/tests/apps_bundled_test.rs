//! The apps that come with Dots (app library spec section 8): Projects is built into Dots from
//! its folder, passes the checks a dot's proposal gets, and its own tests pass. Dots installs it
//! once, for every dot, updates it with a release while it is the bundled one, and leaves it out
//! once the person removed it; "Install Projects" brings it back, on its kept data when it fits.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use dots_core::apps::bundled::{projects, projects_files, release, PROJECTS};
use dots_core::apps::call::Actor;
use dots_core::apps::data;
use dots_core::apps::source::read_files;
use dots_core::apps::tests::run_tests;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::{AppConfig, AppOrigin};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Store, Apps) {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    (dir, store, apps)
}

async fn config(apps: &Apps) -> AppConfig {
    apps.get(PROJECTS).await.unwrap().1
}

/// Projects as an earlier release shipped it: the same migrations, another README.
async fn install_earlier(apps: &Apps, files: BTreeMap<String, Vec<u8>>) {
    let src = read_files(&files).unwrap();
    let origin = AppOrigin::Bundled {
        release: "0.0.1".into(),
    };
    apps.install(src, origin).await.unwrap();
}

fn earlier_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = projects_files();
    files.insert("README.md".into(), b"# Projects, as it was\n".to_vec());
    files
}

async fn create_project(apps: &Apps, name: &str) {
    apps.call(
        PROJECTS,
        "create_project",
        json!({ "name": name }),
        Actor::Person,
    )
    .await
    .unwrap();
}

async fn project_names(apps: &Apps) -> Vec<String> {
    let listed = apps
        .call(PROJECTS, "list_projects", json!({}), Actor::Person)
        .await
        .unwrap();
    listed
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

/// Every file below `dir`, by its path relative to `root` with `/`.
fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&path).unwrap());
        }
    }
}

#[test]
fn projects_holds_every_file_of_its_folder_and_only_lf_text() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("apps")
        .join(PROJECTS);
    let mut on_disk = BTreeMap::new();
    walk(&root, &root, &mut on_disk);
    let built_in = projects_files();
    assert_eq!(
        built_in.keys().collect::<Vec<_>>(),
        on_disk.keys().collect::<Vec<_>>(),
        "list each file of apps/projects in bundled::PROJECTS_FILES"
    );
    for (path, bytes) in &built_in {
        // A CRLF checkout would change the migrations' bytes, and with them what a change to
        // Projects must keep (.gitattributes keeps the folder LF).
        assert!(!bytes.contains(&b'\r'), "{path} holds a carriage return");
        assert!(std::str::from_utf8(bytes).is_ok(), "{path} is not UTF-8");
    }
}

#[test]
fn projects_passes_the_checks_a_proposal_gets_and_its_own_tests() {
    let src = projects().unwrap();
    assert_eq!(src.manifest.name, PROJECTS);
    assert_eq!(src.manifest.title, "Projects");
    assert_eq!(read_files(&projects_files()).unwrap(), src);
    let report = run_tests(&src);
    assert!(report.passed(), "{}", report.problem().unwrap_or_default());
    assert!(report.results.len() >= 15, "{} tests", report.results.len());
}

#[test]
fn projects_migrations_run_and_its_sample_loads() {
    let src = projects().unwrap();
    let mut conn = data::open_in_memory().unwrap();
    assert_eq!(
        data::migrate(&mut conn, &src.migrations).unwrap(),
        src.migrations.len()
    );
    data::load_sample(&conn, src.sample.as_deref().unwrap()).unwrap();
    let tasks = data::query(&conn, "SELECT COUNT(*) AS n FROM tasks", &[]).unwrap();
    assert_eq!(tasks[0]["n"], 4);
}

#[test]
fn an_app_read_from_memory_is_bounded_like_a_folder() {
    let mut files = projects_files();
    files.insert("ui/big.bin".into(), vec![0; 2 * 1024 * 1024]);
    let err = read_files(&files).unwrap_err().to_string();
    assert!(err.contains("more than 2 MB"), "{err}");
    let mut files = projects_files();
    files.insert("notes.txt".into(), b"hi".to_vec());
    let err = read_files(&files).unwrap_err().to_string();
    assert!(err.contains("notes.txt: an app holds"), "{err}");
}

#[tokio::test]
async fn projects_is_installed_once_for_every_dot_and_removing_it_lasts() {
    let (_dir, _store, apps) = fixture().await;
    apps.sync_bundled().await.unwrap();
    let first = config(&apps).await;
    assert_eq!(first.origin, AppOrigin::Bundled { release: release() });
    assert!(first.every_dot, "Projects is on for every dot");
    assert!(!first.can_start_dots, "starting dots waits for the person");
    assert_eq!(first.version, 1);
    assert!(apps.bundled_offers().await.unwrap().is_empty());

    apps.sync_bundled().await.unwrap();
    assert_eq!(config(&apps).await.version, 1, "nothing new to install");

    apps.remove(PROJECTS, true, true).await.unwrap();
    apps.sync_bundled().await.unwrap();
    assert!(matches!(apps.get(PROJECTS).await, Err(Error::NotFound(_))));
    let offers = apps.bundled_offers().await.unwrap();
    assert_eq!(offers.len(), 1);
    assert_eq!(
        (
            offers[0].name.as_str(),
            offers[0].title.as_str(),
            &offers[0].kept
        ),
        (PROJECTS, "Projects", &None)
    );
}

#[tokio::test]
async fn a_new_release_updates_the_bundled_projects_and_keeps_its_data() {
    let (_dir, _store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    create_project(&apps, "Home").await;
    apps.sync_bundled().await.unwrap();
    let now = config(&apps).await;
    assert_eq!(now.version, 2);
    assert_eq!(now.origin, AppOrigin::Bundled { release: release() });
    assert!(!now.changed);
    let (item, _) = apps.get(PROJECTS).await.unwrap();
    let active = apps.load(&item, &now).await.unwrap();
    assert_eq!(active.files(), projects().unwrap().files());
    assert_eq!(project_names(&apps).await, ["Home"]);
}

#[tokio::test]
async fn a_bundled_release_that_changes_a_migration_that_ran_is_refused() {
    let (_dir, _store, apps) = fixture().await;
    let mut files = earlier_files();
    let sql = String::from_utf8(files["migrations/0001_projects.sql"].clone()).unwrap();
    files.insert(
        "migrations/0001_projects.sql".into(),
        format!("{sql}\nCREATE TABLE old_notes (id TEXT);\n").into_bytes(),
    );
    install_earlier(&apps, files).await;
    let err = apps.sync_bundled().await.unwrap_err().to_string();
    assert!(
        err.contains("the bundled Projects cannot update: migrations/0001_projects.sql differs"),
        "{err}"
    );
    assert_eq!(config(&apps).await.version, 1);
}

#[tokio::test]
async fn projects_the_person_changed_or_rolled_back_is_left_as_it_is() {
    let (_dir, _store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    let mut theirs = earlier_files();
    theirs.insert("README.md".into(), b"# My Projects\n".to_vec());
    apps.update(PROJECTS, read_files(&theirs).unwrap(), true)
        .await
        .unwrap();
    apps.sync_bundled().await.unwrap();
    let now = config(&apps).await;
    assert!(now.changed);
    assert_eq!(now.version, 2, "Dots no longer updates it");

    // A bundled update the person rolled back stays rolled back.
    let (_dir, _store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    apps.sync_bundled().await.unwrap();
    apps.roll_back(PROJECTS).await.unwrap();
    apps.sync_bundled().await.unwrap();
    assert_eq!(config(&apps).await.version, 1);
}

#[tokio::test]
async fn an_app_a_dot_made_named_projects_is_left_alone() {
    let (_dir, store, apps) = fixture().await;
    install_earlier(&apps, earlier_files()).await;
    let (item, mut config) = apps.get(PROJECTS).await.unwrap();
    config.origin = AppOrigin::Proposed;
    store
        .update_library_item(&item.id, &dots_core::library::LibraryConfig::App(config))
        .await
        .unwrap();
    apps.sync_bundled().await.unwrap();
    let now = apps.get(PROJECTS).await.unwrap().1;
    assert_eq!((now.version, now.origin), (1, AppOrigin::Proposed));
}

#[tokio::test]
async fn install_projects_starts_on_the_kept_data_or_fresh() {
    let (_dir, _store, apps) = fixture().await;
    apps.sync_bundled().await.unwrap();
    create_project(&apps, "Home").await;
    apps.remove(PROJECTS, true, false).await.unwrap();

    let offer = apps.bundled_offers().await.unwrap().remove(0);
    let kept = offer.kept.expect("the kept data fits");
    assert!(kept.migrations.is_empty(), "no migration runs on it");
    apps.install_bundled(PROJECTS, Some(&kept.id))
        .await
        .unwrap();
    assert_eq!(project_names(&apps).await, ["Home"]);
    assert_eq!(
        config(&apps).await.origin,
        AppOrigin::Bundled { release: release() }
    );
    assert!(apps.kept().await.unwrap().is_empty());

    // Fresh, when the person unticks the kept data: it stays listed.
    apps.remove(PROJECTS, true, false).await.unwrap();
    apps.install_bundled(PROJECTS, None).await.unwrap();
    assert!(project_names(&apps).await.is_empty());
    assert_eq!(apps.kept().await.unwrap().len(), 1);
    let again = apps.install_bundled(PROJECTS, None).await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
    let other = apps.install_bundled("expenses", None).await;
    assert!(matches!(other, Err(Error::NotFound(_))), "{other:?}");
}

#[tokio::test]
async fn the_prompt_to_start_dots_shows_until_the_person_answers_or_sets_the_switch() {
    let (_dir, _store, apps) = fixture().await;
    apps.sync_bundled().await.unwrap();
    assert!(!config(&apps).await.start_asked);
    apps.answer_start_prompt(PROJECTS, false).await.unwrap();
    let now = config(&apps).await;
    assert!(
        now.start_asked && !now.can_start_dots,
        "Not now leaves it off"
    );

    // A fresh install asks again; "Switch on" turns it on.
    apps.remove(PROJECTS, true, true).await.unwrap();
    apps.install_bundled(PROJECTS, None).await.unwrap();
    assert!(!config(&apps).await.start_asked);
    apps.answer_start_prompt(PROJECTS, true).await.unwrap();
    let now = config(&apps).await;
    assert!(now.start_asked && now.can_start_dots);

    // The switch on the Apps screen is an answer too.
    apps.remove(PROJECTS, true, true).await.unwrap();
    apps.install_bundled(PROJECTS, None).await.unwrap();
    apps.set_can_start_dots(PROJECTS, false).await.unwrap();
    assert!(config(&apps).await.start_asked);
}

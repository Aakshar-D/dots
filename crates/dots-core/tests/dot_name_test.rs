mod common;

use common::{fake_dot, init_repo, spec, temp_store};
use dots_core::model::{folder_name, WorkspaceMode};
use dots_core::workspace::WorkspaceManager;
use dots_core::Error;

const RULE: &str = "Use 1 to 64 letters, digits, spaces";

fn check(name: &str) -> Result<(), Error> {
    spec(name).validate()
}

#[test]
fn names_may_hold_spaces_and_dots() {
    for ok in ["Downloads digest", "v1.2 bot", "a", &"a".repeat(64)] {
        assert!(check(ok).is_ok(), "{ok:?} should be accepted");
    }
}

#[test]
fn names_outside_the_rule_are_refused_with_the_rule() {
    let long = "a".repeat(65);
    for bad in [
        "",
        " lead",
        "trail ",
        "two  spaces",
        "slash/no",
        &long,
        "Résumé",
    ] {
        match check(bad) {
            Err(Error::Invalid(m)) => assert!(m.contains(RULE), "{bad:?}: {m}"),
            other => panic!("{bad:?} should be invalid, got {other:?}"),
        }
    }
}

#[test]
fn folder_name_is_a_safe_slug() {
    assert_eq!(folder_name("Downloads digest"), "downloads-digest");
    assert_eq!(folder_name("  --A__B..c "), "a-b-c");
    assert_eq!(folder_name("Résumé bot"), "r-sum-bot");
    assert_eq!(folder_name("日本"), "dot");
    let long = folder_name(&vec!["a"; 32].join(" "));
    assert!(long.len() <= 40, "{long}");
    assert!(!long.ends_with('-'), "{long}");
}

#[tokio::test]
async fn a_name_that_differs_only_in_case_is_a_clash() {
    let (_dir, store) = temp_store().await;
    store
        .create_dot(&spec("Foo"), None, "default")
        .await
        .unwrap();
    let err = store
        .create_dot(&spec("foo"), None, "default")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn existing_case_twins_can_still_be_edited_but_not_renamed_into_a_clash() {
    let (dir, store) = temp_store().await;
    store
        .create_dot(&spec("Bar"), None, "default")
        .await
        .unwrap();
    let twin = store
        .create_dot(&spec("Baz"), None, "default")
        .await
        .unwrap();
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.path().join("dots.db"))
        .foreign_keys(true);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
    sqlx::query("UPDATE dots SET name = 'bar' WHERE id = ?")
        .bind(&twin.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let mut edit = store.get_dot(&twin.id).await.unwrap().spec;
    assert_eq!(edit.name, "bar");
    edit.instructions = "Changed.".into();
    let saved = store.update_dot(&twin.id, &edit).await.unwrap();
    assert_eq!(saved.spec.instructions, "Changed.");

    edit.name = "BAR".into();
    let err = store.update_dot(&twin.id, &edit).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn a_worktree_branch_uses_the_folder_name() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("My Bot", &repo, WorkspaceMode::Worktree);
    let p = WorkspaceManager::new(tmp.path().join("wt"))
        .prepare(&dot, "R1")
        .await
        .unwrap();
    assert_eq!(p.branch.as_deref(), Some("dots/my-bot/R1"));
}

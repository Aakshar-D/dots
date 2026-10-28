//! The review card's Code tab (app library spec section 6.2): a change shows as the diff against
//! the app's active version, a new app as every file added.

mod common;

use std::collections::BTreeMap;

use common::apps::{expenses, expenses_with, proposing_run, write_app, LOGIC};
use dots_core::apps::propose::SENT;
use dots_core::apps::screen::diff_files;
use dots_core::apps::source::read_folder;
use dots_core::apps::Apps;
use dots_core::events::Bus;
use dots_core::library::AppOrigin;
use dots_core::model::FileStatus;
use dots_core::Error;

fn files(list: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    list.iter()
        .map(|(path, bytes)| (path.to_string(), bytes.to_vec()))
        .collect()
}

#[test]
fn a_diff_lists_what_was_added_removed_and_changed_and_skips_the_rest() {
    let old = files(&[
        ("a.txt", b"one\ntwo\nthree\n"),
        ("gone.txt", b"bye\n"),
        ("same.txt", b"same\n"),
        ("ui/logo.png", &[0x89, 0x50, 0xff, 0x00]),
    ]);
    let new = files(&[
        ("a.txt", b"one\n2\nthree\n"),
        ("new.txt", b"hello\n"),
        ("same.txt", b"same\n"),
        ("ui/logo.png", &[0x89, 0x50, 0xfe, 0x00]),
    ]);
    let diff = diff_files(&old, &new);
    let listed: Vec<(&str, FileStatus, bool)> = diff
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status, f.binary))
        .collect();
    assert_eq!(
        listed,
        [
            ("a.txt", FileStatus::Modified, false),
            ("gone.txt", FileStatus::Deleted, false),
            ("new.txt", FileStatus::Added, false),
            ("ui/logo.png", FileStatus::Modified, true),
        ]
    );
    let a = &diff.files[0].patch;
    assert!(a.contains("@@ -1,3 +1,3 @@"), "{a}");
    assert!(a.contains("-two\n+2\n"), "{a}");
    assert!(diff.files[1].patch.contains("-bye\n"));
    assert!(diff.files[2].patch.contains("+hello\n"));
    assert_eq!(diff.files[3].patch, "");
    assert!(!diff.truncated);
}

#[tokio::test]
async fn a_changes_code_tab_shows_its_diff_and_a_new_apps_shows_every_file() {
    let (dir, store) = common::temp_store().await;
    let apps = Apps::new(store.clone(), Bus::new(16), dir.path().join("apps"));
    let run = proposing_run(&store, "builder").await;
    let first = dir.path().join("first");
    write_app(&first, &expenses());
    assert_eq!(
        apps.propose(&run.id, first.clone(), "New.").await,
        Ok(SENT.into())
    );
    let new_app = store.list_app_proposals(None).await.unwrap().remove(0);
    let diff = apps.proposal_diff(&new_app.id).await.unwrap();
    assert_eq!(diff.files.len(), expenses().len(), "{diff:?}");
    assert!(diff.files.iter().all(|f| f.status == FileStatus::Added));
    apps.drop_proposed(&new_app.id).await;
    let decided = apps.proposal_diff(&new_app.id).await;
    assert!(matches!(decided, Err(Error::NotFound(_))), "{decided:?}");

    apps.install(read_folder(&first).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let logic = LOGIC.replace("ORDER BY on_day, id", "ORDER BY on_day DESC, id");
    let next = dir.path().join("next");
    write_app(&next, &expenses_with("logic.js", Some(&logic)));
    let run = proposing_run(&store, "fixer").await;
    assert_eq!(
        apps.propose(&run.id, next, "Newest first.").await,
        Ok(SENT.into())
    );
    let change = store
        .list_app_proposals(Some(&run.dot_id))
        .await
        .unwrap()
        .remove(0);
    let diff = apps.proposal_diff(&change.id).await.unwrap();
    assert_eq!(diff.files.len(), 1, "{diff:?}");
    assert_eq!(diff.files[0].path, "logic.js");
    assert_eq!(diff.files[0].status, FileStatus::Modified);
    assert!(
        diff.files[0].patch.contains("+  return ctx.db.query"),
        "{}",
        diff.files[0].patch
    );
}

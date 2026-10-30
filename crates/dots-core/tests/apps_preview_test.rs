//! The review card's preview and the frame's files (app library spec sections 5 and 6.2): a
//! proposal runs for real on a throwaway database, as the person, starting no dot; a frame gets
//! only the `ui/` files of the version it opened.

mod common;

use common::apps::{expenses, expenses_with, proposing_run, write_app, MANIFEST, UI, UI_SCRIPT};
use dots_core::apps::preview::MAX_PREVIEWS;
use dots_core::apps::propose::{AppProposal, SENT};
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
    (dir, store, apps)
}

/// `files` proposed by a new dot `name`.
async fn proposed(
    dir: &tempfile::TempDir,
    store: &Store,
    apps: &Apps,
    name: &str,
    files: &[(&str, String)],
) -> AppProposal {
    let run = proposing_run(store, name).await;
    let folder = dir.path().join(format!("work-{name}"));
    write_app(&folder, files);
    assert_eq!(
        apps.propose(&run.id, folder, "Built it.").await,
        Ok(SENT.to_string())
    );
    store
        .list_app_proposals(Some(&run.dot_id))
        .await
        .unwrap()
        .remove(0)
}

fn count(rows: &serde_json::Value) -> usize {
    rows.as_array().unwrap().len()
}

#[tokio::test]
async fn a_preview_runs_on_the_sample_data_and_its_changes_vanish_when_it_closes() {
    let (dir, store, apps) = fixture().await;
    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let month = json!({ "month": "2026-10" });
    let preview = apps.preview_open(&p.id).await.unwrap();
    let rows = apps
        .preview_call(&preview, "list_expenses", month.clone())
        .await
        .unwrap();
    assert_eq!(count(&rows), 2, "the sample's two rows: {rows}");
    let added = json!({ "item": "Lunch", "amount": 9.5, "on": "2026-10-23" });
    apps.preview_call(&preview, "add_expense", added)
        .await
        .unwrap();
    let rows = apps
        .preview_call(&preview, "list_expenses", month.clone())
        .await
        .unwrap();
    assert_eq!(count(&rows), 3);
    // Nothing reached the Library.
    assert!(apps.list().await.unwrap().is_empty());

    apps.preview_close(&preview);
    let closed = apps
        .preview_call(&preview, "list_expenses", month.clone())
        .await;
    assert!(matches!(closed, Err(Error::NotFound(_))), "{closed:?}");
    let again = apps.preview_open(&p.id).await.unwrap();
    let rows = apps
        .preview_call(&again, "list_expenses", month)
        .await
        .unwrap();
    assert_eq!(
        count(&rows),
        2,
        "a new preview starts from the sample again"
    );
}

#[tokio::test]
async fn a_preview_calls_as_the_person_only_listed_tools_and_cannot_start_dots() {
    let (dir, store, apps) = fixture().await;
    let manifest = MANIFEST.replacen(
        "\"tools\": [",
        "\"permissions\": [\"start_dots\"],\n  \"tools\": [\n    { \"name\": \"who\", \
         \"description\": \"Who calls\", \"input\": { \"type\": \"object\" }, \"writes\": true },",
        1,
    );
    let logic = format!(
        "{}\nexport function who(input, ctx) {{ return {{ actor: ctx.actor.kind, \
         start: typeof ctx.dots.start }}; }}\n",
        common::apps::LOGIC
    );
    let mut files = expenses_with("app.json", Some(&manifest));
    files.retain(|(path, _)| *path != "logic.js");
    files.push(("logic.js", logic));
    let p = proposed(&dir, &store, &apps, "builder", &files).await;
    let preview = apps.preview_open(&p.id).await.unwrap();
    let who = apps.preview_call(&preview, "who", json!({})).await.unwrap();
    assert_eq!(who, json!({ "actor": "person", "start": "undefined" }));
    let unlisted = apps.preview_call(&preview, "on_run_end", json!({})).await;
    assert!(matches!(unlisted, Err(Error::Invalid(_))), "{unlisted:?}");
    let bad = apps
        .preview_call(&preview, "add_expense", json!({ "item": "Lunch" }))
        .await;
    match bad {
        Err(Error::Invalid(m)) => assert!(m.contains("amount"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_frame_gets_only_the_ui_files_of_its_version_or_its_preview() {
    let (dir, store, apps) = fixture().await;
    let src = tempfile::tempdir().unwrap();
    write_app(src.path(), &expenses());
    apps.install(read_folder(src.path()).unwrap(), AppOrigin::Proposed)
        .await
        .unwrap();
    let index = apps.ui_file("expenses", 1, "index.html").await.unwrap();
    assert_eq!(String::from_utf8(index).unwrap(), UI);
    let script = apps.ui_file("expenses", 1, "app.js").await.unwrap();
    assert_eq!(String::from_utf8(script).unwrap(), UI_SCRIPT);
    for (version, path) in [
        (2, "index.html"),
        (1, "../logic.js"),
        (1, "logic.js"),
        (1, ""),
    ] {
        let refused = apps.ui_file("expenses", version, path).await;
        assert!(
            matches!(refused, Err(Error::NotFound(_))),
            "{path}: {refused:?}"
        );
    }
    let missing = apps.ui_file("nothing", 1, "index.html").await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");

    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let preview = apps.preview_open(&p.id).await.unwrap();
    let index = apps.preview_file(&preview, "index.html").unwrap();
    assert_eq!(String::from_utf8(index).unwrap(), UI);
    assert!(matches!(
        apps.preview_file(&preview, "app.json"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        apps.preview_file("no-such-preview", "index.html"),
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn previews_are_bounded_and_a_decided_proposal_closes_its_own() {
    let (dir, store, apps) = fixture().await;
    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let mut open = Vec::new();
    for _ in 0..=MAX_PREVIEWS {
        open.push(apps.preview_open(&p.id).await.unwrap());
    }
    assert!(matches!(
        apps.preview_file(&open[0], "index.html"),
        Err(Error::NotFound(_))
    ));
    assert!(apps.preview_file(&open[1], "index.html").is_ok());
    apps.drop_proposed(&p.id).await;
    for id in &open {
        assert!(apps.preview_file(id, "index.html").is_err(), "{id}");
    }
    let gone = apps.preview_open(&p.id).await;
    assert!(gone.is_err(), "{gone:?}");
}

#[tokio::test]
async fn opening_one_preview_too_many_closes_the_one_used_least_recently() {
    let (dir, store, apps) = fixture().await;
    let p = proposed(&dir, &store, &apps, "builder", &expenses()).await;
    let mut open = Vec::new();
    for _ in 0..MAX_PREVIEWS {
        open.push(apps.preview_open(&p.id).await.unwrap());
    }
    // The first card is still in use: a call and a file keep its preview.
    apps.preview_call(&open[0], "list_expenses", json!({ "month": "2026-10" }))
        .await
        .unwrap();
    apps.preview_file(&open[1], "index.html").unwrap();
    let newest = apps.preview_open(&p.id).await.unwrap();
    assert!(apps.preview_file(&open[0], "index.html").is_ok());
    assert!(apps.preview_file(&open[1], "index.html").is_ok());
    assert!(matches!(
        apps.preview_file(&open[2], "index.html"),
        Err(Error::NotFound(_))
    ));
    assert!(apps.preview_file(&newest, "index.html").is_ok());
}

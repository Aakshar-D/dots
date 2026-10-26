//! The watchers of a dot (watchers spec sections 3 and 4): the rows, their order, a change that
//! clears the cursor, the pause by Dots and the person's resume, the sliding hour of fires, the
//! checks before a save with the spec's sentences, and the JSON the UI reads.

mod common;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    EmailConfig, FolderConfig, FolderEvent, GithubConfig, GithubEvent, NewRun, PausedReason,
    PausedWatcher, TriggerKind, WatcherConfig, WatcherKind,
};
use dots_core::util::timestamp;
use dots_core::vault::{EntryKind, Vault};
use dots_core::watchers::{
    bad_pattern, bad_repo, check_config, check_name, no_entry, no_folder, plain_entry,
    ADDRESS_RULE, EVENTS_RULE, FOLDER_EVENTS_RULE, FROM_RULE, NAME_RULE, NOT_ABSOLUTE, REPOS_RULE,
    SECRET_RULE,
};
use dots_core::Error;
use serde_json::json;

fn message(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-23T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// Times are stored to the millisecond: rows written one after another need a tick between.
async fn tick() {
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
}

fn folder(path: &str) -> WatcherConfig {
    WatcherConfig::Folder(FolderConfig::new(path))
}

fn email() -> WatcherConfig {
    WatcherConfig::Email(EmailConfig::new("me@gmail.com", "GMAIL_APP_PASSWORD"))
}

#[tokio::test]
async fn watchers_are_added_listed_changed_and_deleted_and_go_with_their_dot() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("invoices")).await.unwrap();
    let other = store.create_dot(&spec("other")).await.unwrap();
    let a = store
        .add_watcher(&dot.id, "Invoices folder", &folder("C:/Users/me/Invoices"))
        .await
        .unwrap();
    tick().await;
    let b = store.add_watcher(&dot.id, "Mail", &email()).await.unwrap();
    store
        .add_watcher(
            &other.id,
            "Repo",
            &WatcherConfig::Github(GithubConfig::new(&["owner/repo"], "GITHUB_TOKEN")),
        )
        .await
        .unwrap();
    assert_eq!(a.kind(), WatcherKind::Folder);
    assert!(a.enabled);
    assert_eq!((a.paused_reason, a.last_error.as_deref()), (None, None));
    assert_eq!(a.last_event_at, None);
    assert_eq!(a.created_at, a.updated_at);
    assert_eq!(
        store.list_watchers(&dot.id).await.unwrap(),
        vec![a.clone(), b.clone()]
    );
    assert_eq!(store.get_watcher(&a.id).await.unwrap(), a);
    let unknown = store.add_watcher("NODOT", "x", &email()).await.unwrap_err();
    assert!(matches!(unknown, Error::NotFound(_)), "{unknown:?}");

    // A changed config starts the watcher again from now: its cursor goes.
    let cursor = json!({"uid_validity": 7, "last_uid": 41});
    store
        .set_watcher_cursor(&b.id, Some(&cursor))
        .await
        .unwrap();
    let mut labelled = EmailConfig::new("me@gmail.com", "GMAIL_APP_PASSWORD");
    labelled.label = "Dots".into();
    let labelled = WatcherConfig::Email(labelled);
    tick().await;
    let b2 = store
        .update_watcher(&b.id, "Dots mail", &labelled)
        .await
        .unwrap();
    assert_eq!((b2.name.as_str(), &b2.config), ("Dots mail", &labelled));
    assert!(b2.updated_at > b.updated_at);
    assert_eq!(store.get_watcher_cursor(&b.id).await.unwrap(), None);
    // A new name alone keeps it.
    store
        .set_watcher_cursor(&b.id, Some(&cursor))
        .await
        .unwrap();
    store
        .update_watcher(&b.id, "Renamed", &labelled)
        .await
        .unwrap();
    assert_eq!(store.get_watcher_cursor(&b.id).await.unwrap(), Some(cursor));

    assert!(
        !store
            .set_watcher_enabled(&a.id, false)
            .await
            .unwrap()
            .enabled
    );
    assert_eq!(store.delete_watcher(&a.id).await.unwrap().id, a.id);
    let gone = store.get_watcher(&a.id).await.unwrap_err();
    assert!(matches!(gone, Error::NotFound(_)), "{gone:?}");

    store.delete_dot(&dot.id).await.unwrap();
    let gone = store.get_watcher(&b.id).await.unwrap_err();
    assert!(matches!(gone, Error::NotFound(_)), "{gone:?}");
    assert!(store.list_watchers(&dot.id).await.unwrap().is_empty());
    assert_eq!(store.all_watchers().await.unwrap().len(), 1);
}

#[tokio::test]
async fn dots_pauses_a_watcher_once_and_the_person_resumes_it() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("invoices")).await.unwrap();
    let w = store
        .add_watcher(&dot.id, "Invoices folder", &folder("C:/in"))
        .await
        .unwrap();
    let at = timestamp(t0());
    let paused = store
        .pause_watcher(&w.id, PausedReason::Flood, None, &at)
        .await
        .unwrap()
        .expect("it was not paused");
    assert_eq!(paused.paused_reason, Some(PausedReason::Flood));
    // Paused already: nothing changes, so nothing is announced twice.
    assert_eq!(
        store
            .pause_watcher(&w.id, PausedReason::Error, Some("x"), &at)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.paused_watchers().await.unwrap(),
        vec![PausedWatcher {
            watcher_id: w.id.clone(),
            dot_id: dot.id.clone(),
            dot_name: "invoices".into(),
            kind: WatcherKind::Folder,
            name: "Invoices folder".into(),
            reason: PausedReason::Flood,
            error: None,
            at: at.clone(),
        }]
    );
    store
        .set_watcher_error(&w.id, Some("network down"))
        .await
        .unwrap();
    assert_eq!(
        store
            .get_watcher(&w.id)
            .await
            .unwrap()
            .last_error
            .as_deref(),
        Some("network down")
    );
    let resumed = store.resume_watcher(&w.id).await.unwrap();
    assert_eq!((resumed.paused_reason, resumed.last_error), (None, None));
    assert!(store.paused_watchers().await.unwrap().is_empty());

    let error = "The folder C:/in no longer exists.";
    store
        .pause_watcher(&w.id, PausedReason::Error, Some(error), &at)
        .await
        .unwrap();
    let listed = store.paused_watchers().await.unwrap();
    assert_eq!(listed[0].reason, PausedReason::Error);
    assert_eq!(listed[0].error.as_deref(), Some(error));

    store.touch_watcher_event(&w.id, &at).await.unwrap();
    assert_eq!(
        store.get_watcher(&w.id).await.unwrap().last_event_at,
        Some(at)
    );
}

#[tokio::test]
async fn a_watcher_takes_at_most_20_fires_in_a_sliding_hour() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("busy")).await.unwrap();
    let w = store
        .add_watcher(&dot.id, "Busy", &folder("C:/in"))
        .await
        .unwrap();
    let hour = TimeDelta::hours(1);
    for i in 0..20 {
        let at = t0() + TimeDelta::minutes(i);
        assert!(
            store.take_fire(&w.id, at, hour, 20).await.unwrap(),
            "fire {i}"
        );
    }
    assert!(!store
        .take_fire(&w.id, t0() + TimeDelta::minutes(20), hour, 20)
        .await
        .unwrap());
    // An hour and a second after the first fire, it has left the window: 19 remain.
    let later = t0() + hour + TimeDelta::seconds(1);
    assert!(store.take_fire(&w.id, later, hour, 20).await.unwrap());
    assert!(!store.take_fire(&w.id, later, hour, 20).await.unwrap());
    // The person's resume clears the count.
    store.clear_fires(&w.id).await.unwrap();
    for _ in 0..20 {
        assert!(store.take_fire(&w.id, later, hour, 20).await.unwrap());
    }
}

#[tokio::test]
async fn a_watcher_is_checked_before_it_is_saved() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    vault
        .add(
            "GMAIL_APP_PASSWORD",
            EntryKind::Secret,
            "app-pass-1234567890",
        )
        .await
        .unwrap();
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    let refused = |r: dots_core::Result<WatcherConfig>| message(r.unwrap_err());

    assert_eq!(check_name("  Invoices folder ").unwrap(), "Invoices folder");
    for bad in ["", "   ", &"x".repeat(81)] {
        assert_eq!(message(check_name(bad).unwrap_err()), NAME_RULE);
    }

    let here = dir.path().to_string_lossy().to_string();
    let missing = dir.path().join("nothing").to_string_lossy().to_string();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").unwrap();
    let file = file.to_string_lossy().to_string();
    assert_eq!(
        refused(check_config(&store, &folder("Invoices")).await),
        NOT_ABSOLUTE
    );
    assert_eq!(
        refused(check_config(&store, &folder(&missing)).await),
        no_folder(&missing)
    );
    assert_eq!(
        refused(check_config(&store, &folder(&file)).await),
        no_folder(&file)
    );
    let mut none = FolderConfig::new(&here);
    none.events.clear();
    assert_eq!(
        refused(check_config(&store, &WatcherConfig::Folder(none)).await),
        FOLDER_EVENTS_RULE
    );
    let mut broken = FolderConfig::new(&here);
    broken.pattern = "[".into();
    assert_eq!(
        refused(check_config(&store, &WatcherConfig::Folder(broken)).await),
        bad_pattern("[")
    );
    let mut ok = FolderConfig::new(&here);
    ok.pattern = " *.pdf ".into();
    let WatcherConfig::Folder(saved) = check_config(&store, &WatcherConfig::Folder(ok))
        .await
        .unwrap()
    else {
        panic!("a folder config")
    };
    assert_eq!(saved.pattern, "*.pdf");

    let mail = |f: &dyn Fn(&mut EmailConfig)| {
        let mut c = EmailConfig::new("me@gmail.com", "GMAIL_APP_PASSWORD");
        f(&mut c);
        WatcherConfig::Email(c)
    };
    assert_eq!(
        refused(check_config(&store, &mail(&|c| c.address = "me".into())).await),
        ADDRESS_RULE
    );
    assert_eq!(
        refused(check_config(&store, &mail(&|c| c.secret = " ".into())).await),
        SECRET_RULE
    );
    assert_eq!(
        refused(check_config(&store, &mail(&|c| c.secret = "NOPE".into())).await),
        no_entry("NOPE")
    );
    assert_eq!(
        refused(check_config(&store, &mail(&|c| c.secret = "LOG_LEVEL".into())).await),
        plain_entry("LOG_LEVEL")
    );
    assert_eq!(
        refused(check_config(&store, &mail(&|c| c.from = Some("acme.com".into()))).await),
        FROM_RULE
    );
    let WatcherConfig::Email(saved) = check_config(
        &store,
        &mail(&|c| {
            c.label = "  ".into();
            c.from = Some(" @acme.com ".into());
            c.subject = Some("".into());
        }),
    )
    .await
    .unwrap() else {
        panic!("an email config")
    };
    assert_eq!(saved.label, "INBOX");
    assert_eq!(saved.from.as_deref(), Some("@acme.com"));
    assert_eq!(saved.subject, None);

    vault
        .add(
            "GITHUB_TOKEN",
            EntryKind::Secret,
            "github_pat_0123456789abcdef",
        )
        .await
        .unwrap();
    let hub = |repos: &[&str], f: &dyn Fn(&mut GithubConfig)| {
        let mut c = GithubConfig::new(repos, "GITHUB_TOKEN");
        f(&mut c);
        WatcherConfig::Github(c)
    };
    assert_eq!(
        refused(check_config(&store, &hub(&[], &|_| {})).await),
        REPOS_RULE
    );
    assert_eq!(
        refused(check_config(&store, &hub(&["owner"], &|_| {})).await),
        bad_repo("owner")
    );
    assert_eq!(
        refused(check_config(&store, &hub(&["owner/repo"], &|c| c.events.clear())).await),
        EVENTS_RULE
    );
    let WatcherConfig::Github(saved) = check_config(
        &store,
        &hub(&[" owner/repo "], &|c| {
            c.authors = vec![" alice ".into(), "".into()];
            c.branches = vec!["main".into(), " ".into()];
        }),
    )
    .await
    .unwrap() else {
        panic!("a GitHub config")
    };
    assert_eq!(saved.repos, ["owner/repo"]);
    assert_eq!(saved.authors, ["alice"]);
    assert_eq!(saved.branches, ["main"]);
}

#[test]
fn a_watchers_config_is_tagged_by_its_kind_and_the_events_say_which_watcher() {
    assert_eq!(
        serde_json::to_value(folder("C:/in")).unwrap(),
        json!({"kind": "folder", "path": "C:/in", "pattern": "",
               "events": ["created", "changed", "deleted"], "recursive": true})
    );
    assert_eq!(
        serde_json::to_value(email()).unwrap(),
        json!({"kind": "email", "address": "me@gmail.com", "secret": "GMAIL_APP_PASSWORD",
               "label": "INBOX", "from": null, "subject": null, "has_attachment": null})
    );
    assert_eq!(
        serde_json::to_value(WatcherConfig::Github(GithubConfig::new(
            &["owner/repo"],
            "GITHUB_TOKEN"
        )))
        .unwrap(),
        json!({"kind": "github", "repos": ["owner/repo"], "events": ["issue", "pull_request"],
               "secret": "GITHUB_TOKEN", "label": null, "authors": [], "skip_bots": true,
               "branches": []})
    );
    // A config saved with only what it needs reads with the defaults.
    let read: WatcherConfig =
        serde_json::from_value(json!({"kind": "folder", "path": "C:/in"})).unwrap();
    assert_eq!(read, folder("C:/in"));
    assert_eq!(FolderEvent::ALL.len(), 3);
    assert_eq!(GithubEvent::ALL.len(), 5);
    assert_eq!(TriggerKind::Watcher.as_str(), "watcher");
    assert_eq!(TriggerKind::parse("watcher").unwrap(), TriggerKind::Watcher);
    assert_eq!(
        serde_json::to_value(RuntimeEvent::WatcherUpdated {
            dot_id: "D1".into(),
            watcher_id: "W1".into()
        })
        .unwrap(),
        json!({"type": "watcher_updated", "dot_id": "D1", "watcher_id": "W1"})
    );
    assert_eq!(
        serde_json::to_value(RuntimeEvent::WatcherPaused {
            dot_id: "D1".into(),
            watcher_id: "W1".into(),
            name: "Invoices folder".into(),
            reason: PausedReason::Flood,
            error: None,
        })
        .unwrap(),
        json!({"type": "watcher_paused", "dot_id": "D1", "watcher_id": "W1",
               "name": "Invoices folder", "reason": "flood", "error": null})
    );
}

#[tokio::test]
async fn a_run_can_be_queued_under_an_id_made_beforehand() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("ids")).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Watcher);
    new.id = Some("01JWATCHRUN000000000000000".into());
    new.payload = Some(json!({"source": "folder"}));
    let run = store.create_run(&new).await.unwrap();
    assert_eq!(run.id, "01JWATCHRUN000000000000000");
    assert_eq!(run.root_run_id, run.id);
    assert_eq!(run.trigger, TriggerKind::Watcher);
    // Without an id, the store makes one.
    let other = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(other.id.len(), 26);
}

/// Self-QA: an empty folder field asks for a folder, rather than for an absolute path.
#[tokio::test]
async fn an_empty_folder_field_asks_for_a_folder() {
    let (_dir, store) = temp_store().await;
    let refused = message(check_config(&store, &folder("  ")).await.unwrap_err());
    assert_eq!(refused, dots_core::watchers::FOLDER_RULE);
    assert_eq!(dots_core::watchers::FOLDER_RULE, "Choose a folder.");
}

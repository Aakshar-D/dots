//! The watchers of a dot (watchers spec sections 3 and 4): the rows, their order, a change that
//! clears the cursor, the pause by Dots and the person's resume, the sliding hour of fires, the
//! checks before a save with the spec's sentences, and the JSON the UI reads.

mod common;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    EmailConfig, FolderConfig, FolderEvent, GithubConfig, GithubEvent, NewRun, PausedReason,
    PausedWatcher, TriggerKind, WatchHours, WatcherConfig, WatcherKind,
};
use dots_core::store::Store;
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
    let dot = store
        .create_dot(&spec("invoices"), None, "default")
        .await
        .unwrap();
    let other = store
        .create_dot(&spec("other"), None, "default")
        .await
        .unwrap();
    let a = store
        .add_watcher(
            &dot.id,
            "Invoices folder",
            &folder("C:/Users/me/Invoices"),
            None,
        )
        .await
        .unwrap();
    tick().await;
    let b = store
        .add_watcher(&dot.id, "Mail", &email(), None)
        .await
        .unwrap();
    store
        .add_watcher(
            &other.id,
            "Repo",
            &WatcherConfig::Github(GithubConfig::new(&["owner/repo"], "GITHUB_TOKEN")),
            None,
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
    let unknown = store
        .add_watcher("NODOT", "x", &email(), None)
        .await
        .unwrap_err();
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
        .update_watcher(&b.id, "Dots mail", &labelled, None)
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
        .update_watcher(&b.id, "Renamed", &labelled, None)
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
    let dot = store
        .create_dot(&spec("invoices"), None, "default")
        .await
        .unwrap();
    let w = store
        .add_watcher(&dot.id, "Invoices folder", &folder("C:/in"), None)
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
    let dot = store
        .create_dot(&spec("busy"), None, "default")
        .await
        .unwrap();
    let w = store
        .add_watcher(&dot.id, "Busy", &folder("C:/in"), None)
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
    let dot = store
        .create_dot(&spec("ids"), None, "default")
        .await
        .unwrap();
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

fn hours() -> WatchHours {
    serde_json::from_value(json!({"days": ["mon", "tue"], "start": "09:00", "end": "18:00"}))
        .unwrap()
}

#[tokio::test]
async fn a_watchers_hours_are_kept_on_add_and_update_and_none_means_any_time() {
    let (_dir, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("hrs"), None, "default")
        .await
        .unwrap();
    let a = store
        .add_watcher(&dot.id, "Mail", &email(), Some(&hours()))
        .await
        .unwrap();
    assert_eq!(a.hours, Some(hours()));
    assert_eq!((a.held_count, a.next_opening.clone()), (0, None));
    assert_eq!(store.get_watcher(&a.id).await.unwrap().hours, Some(hours()));
    let b = store
        .add_watcher(&dot.id, "Folder", &folder("C:/in"), None)
        .await
        .unwrap();
    assert_eq!(b.hours, None);

    // An update that keeps the config keeps the cursor and changes the hours.
    let cursor = json!({"uid_validity": 7, "last_uid": 41});
    store
        .set_watcher_cursor(&a.id, Some(&cursor))
        .await
        .unwrap();
    let mut other = hours();
    other.end = chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap();
    let a2 = store
        .update_watcher(&a.id, "Mail", &email(), Some(&other))
        .await
        .unwrap();
    assert_eq!(a2.hours, Some(other));
    assert_eq!(store.get_watcher_cursor(&a.id).await.unwrap(), Some(cursor));
    let a3 = store
        .update_watcher(&a.id, "Mail", &email(), None)
        .await
        .unwrap();
    assert_eq!(a3.hours, None);
}

#[tokio::test]
async fn a_watcher_stored_before_migration_0028_reads_hours_none() {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version <= 27)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode,          webhook_token, policy, max_turns, timeout_secs, approval_wait_secs,          created_at, updated_at) VALUES ('D1', 'old', 'Do it.', 'claude', 'sonnet',          'C:/tmp/repo', 'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z',          '2026-10-01T00:00:00.000Z')",
    )
    .bind(
        serde_json::to_string(&dots_core::policy::Policy::preset(
            dots_core::policy::Preset::Sandboxed,
        ))
        .unwrap(),
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO watchers (id, dot_id, kind, name, config, enabled, created_at, updated_at)          VALUES ('W1', 'D1', 'folder', 'old', ?, 1, '2026-10-01T00:00:00.000Z',          '2026-10-01T00:00:00.000Z')",
    )
    .bind(serde_json::to_string(&folder("C:/in")).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let w = store.get_watcher("W1").await.unwrap();
    assert_eq!((w.hours, w.held_count), (None, 0));
}

#[tokio::test]
async fn held_events_are_counted_listed_oldest_first_taken_dropped_and_cascade() {
    let (_dir, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("held"), None, "default")
        .await
        .unwrap();
    let w = store
        .add_watcher(&dot.id, "Mail", &email(), Some(&hours()))
        .await
        .unwrap();
    let w2 = store
        .add_watcher(&dot.id, "Mail 2", &email(), None)
        .await
        .unwrap();
    // Held out of order: the listing goes by held_at.
    let late = store
        .hold_event(&w.id, &json!({"n": 3}), None, t0() + TimeDelta::seconds(2))
        .await
        .unwrap();
    let files = json!([{"name": "a.pdf", "path": "C:/held/a.pdf"}]);
    let first = store
        .hold_event(&w.id, &json!({"n": 1}), Some(&files), t0())
        .await
        .unwrap();
    let mid = store
        .hold_event(&w.id, &json!({"n": 2}), None, t0() + TimeDelta::seconds(1))
        .await
        .unwrap();
    store
        .hold_event(&w2.id, &json!({"n": 9}), None, t0())
        .await
        .unwrap();
    assert_eq!(first.held_at, timestamp(t0()));
    assert_eq!(store.held_count(&w.id).await.unwrap(), 3);
    assert_eq!(store.get_watcher(&w.id).await.unwrap().held_count, 3);
    let listed = store.list_watchers(&dot.id).await.unwrap();
    assert_eq!(
        listed.iter().map(|w| w.held_count).collect::<Vec<_>>(),
        vec![3, 1]
    );
    assert_eq!(store.all_watchers().await.unwrap()[0].held_count, 3);
    let held = store.held_events(&w.id).await.unwrap();
    assert_eq!(held, vec![first.clone(), mid.clone(), late.clone()]);
    assert_eq!(held[0].files, Some(files));

    let taken = store.take_held(&w.id).await.unwrap();
    assert_eq!(taken, vec![first, mid, late]);
    assert_eq!(store.held_count(&w.id).await.unwrap(), 0);
    assert!(store.take_held(&w.id).await.unwrap().is_empty());
    assert_eq!(store.held_count(&w2.id).await.unwrap(), 1);

    for n in 0..2 {
        store
            .hold_event(&w.id, &json!({"n": n}), None, t0() + TimeDelta::seconds(n))
            .await
            .unwrap();
    }
    let dropped = store.drop_held(&w.id).await.unwrap();
    assert_eq!(
        dropped
            .iter()
            .map(|h| h.payload["n"].clone())
            .collect::<Vec<_>>(),
        vec![json!(0), json!(1)]
    );
    assert_eq!(store.held_count(&w.id).await.unwrap(), 0);

    store
        .hold_event(
            &w2.id,
            &json!({"n": 10}),
            None,
            t0() + TimeDelta::seconds(1),
        )
        .await
        .unwrap();
    store.delete_watcher(&w2.id).await.unwrap();
    assert!(store.held_events(&w2.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_bad_stored_hours_value_reads_as_none_and_hides_no_watcher() {
    let (dir, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("bad"), None, "default")
        .await
        .unwrap();
    let good = store
        .add_watcher(&dot.id, "Good", &email(), Some(&hours()))
        .await
        .unwrap();
    tick().await;
    let bad = store
        .add_watcher(&dot.id, "Bad", &folder("C:/in"), Some(&hours()))
        .await
        .unwrap();
    let raw = sqlx::sqlite::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("dots.db")),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE watchers SET hours = 'garbage' WHERE id = ?")
        .bind(&bad.id)
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let listed = store.list_watchers(&dot.id).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].hours, Some(hours()));
    assert_eq!(
        (listed[1].id.as_str(), &listed[1].hours),
        (bad.id.as_str(), &None)
    );
    assert_eq!(store.all_watchers().await.unwrap().len(), 2);
    assert_eq!(
        store.get_watcher(&good.id).await.unwrap().hours,
        Some(hours())
    );
}

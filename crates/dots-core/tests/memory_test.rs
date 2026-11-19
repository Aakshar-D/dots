//! What a dot remembers (dot memory spec sections 2, 3 and 6): the store's order, limit and
//! duplicate check, the rules of a memory's text, the secret patterns, the event every change
//! publishes, and the memories going with their dot.

mod common;

use common::{spec, temp_store};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::memory::{
    check_text, looks_secret, same_key, Memories, DUPLICATE, FULL, LENGTH_RULE, MAX_PER_DOT, SECRET,
};
use dots_core::model::{Dot, MemorySource};
use dots_core::store::{Added, Store};
use dots_core::Error;
use tempfile::TempDir;

struct Setup {
    _dir: TempDir,
    store: Store,
    memories: Memories,
    events: tokio::sync::broadcast::Receiver<RuntimeEvent>,
}

async fn setup() -> Setup {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let events = bus.subscribe();
    let memories = Memories::new(store.clone(), bus);
    Setup {
        _dir: dir,
        store,
        memories,
        events,
    }
}

impl Setup {
    async fn dot(&self, name: &str) -> Dot {
        self.store
            .create_dot(&spec(name), None, "default")
            .await
            .unwrap()
    }

    /// The dots whose memories changed since the last call, in order.
    fn changed(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if let RuntimeEvent::MemoriesUpdated { dot_id } = event {
                out.push(dot_id);
            }
        }
        out
    }
}

fn message(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

/// Times are stored to the millisecond: memories saved one after another need a tick between.
async fn tick() {
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
}

#[tokio::test]
async fn memories_are_listed_pinned_first_then_newest_and_can_be_changed_and_deleted() {
    let mut t = setup().await;
    let dot = t.dot("invoices").await;
    let other = t.dot("other").await;
    let a = t
        .memories
        .add(&dot.id, "  Prefers invoices as PDF.  ")
        .await
        .unwrap();
    tick().await;
    let b = t
        .memories
        .save(
            &dot.id,
            "The staging server is staging.example.com.",
            MemorySource::Dot,
            Some("RUN1"),
        )
        .await
        .unwrap();
    tick().await;
    let c = t.memories.add(&dot.id, "Sends on Fridays.").await.unwrap();
    t.memories
        .add(&other.id, "Another sol's note.")
        .await
        .unwrap();
    assert_eq!(a.text, "Prefers invoices as PDF.");
    assert_eq!(
        (a.source, a.run_id.as_deref()),
        (MemorySource::Person, None)
    );
    assert_eq!(
        (b.source, b.run_id.as_deref()),
        (MemorySource::Dot, Some("RUN1"))
    );
    assert!(!a.pinned);
    assert_eq!(a.created_at, a.updated_at);

    let ids =
        |list: Vec<dots_core::model::Memory>| list.into_iter().map(|m| m.id).collect::<Vec<_>>();
    assert_eq!(
        ids(t.memories.list(&dot.id).await.unwrap()),
        [c.id.clone(), b.id.clone(), a.id.clone()]
    );
    let pinned = t.memories.update(&a.id, None, Some(true)).await.unwrap();
    assert!(pinned.pinned);
    assert_eq!(pinned.text, a.text);
    assert_eq!(
        ids(t.memories.list(&dot.id).await.unwrap()),
        [a.id.clone(), c.id.clone(), b.id.clone()]
    );

    // The person may change any memory, the dot's too.
    tick().await;
    let edited = t
        .memories
        .update(&b.id, Some(" Staging is staging2.example.com. "), None)
        .await
        .unwrap();
    assert_eq!(edited.text, "Staging is staging2.example.com.");
    assert_eq!(edited.source, MemorySource::Dot);
    assert!(edited.updated_at > b.updated_at);
    t.memories.delete(&c.id).await.unwrap();
    assert_eq!(
        ids(t.memories.list(&dot.id).await.unwrap()),
        [a.id.clone(), b.id.clone()]
    );
    assert_eq!(
        t.changed(),
        vec![
            dot.id.clone(),
            dot.id.clone(),
            dot.id.clone(),
            other.id.clone(),
            dot.id.clone(),
            dot.id.clone(),
            dot.id.clone()
        ]
    );

    let gone = t.memories.delete(&c.id).await.unwrap_err();
    assert!(matches!(gone, Error::NotFound(_)), "{gone:?}");
    let unknown = t.memories.list("NODOT").await.unwrap_err();
    assert!(matches!(unknown, Error::NotFound(_)), "{unknown:?}");
    let unknown = t.memories.add("NODOT", "x").await.unwrap_err();
    assert!(matches!(unknown, Error::NotFound(_)), "{unknown:?}");
    assert!(t.changed().is_empty());
}

#[tokio::test]
async fn a_memory_is_one_to_500_characters_and_never_a_duplicate() {
    let mut t = setup().await;
    let dot = t.dot("rules").await;
    for blank in ["", "   ", "\n\t"] {
        assert_eq!(
            message(t.memories.add(&dot.id, blank).await.unwrap_err()),
            LENGTH_RULE
        );
    }
    let long = "é".repeat(501);
    assert_eq!(
        message(t.memories.add(&dot.id, &long).await.unwrap_err()),
        LENGTH_RULE
    );
    // 500 characters of two bytes each is a memory.
    let most = t.memories.add(&dot.id, &"é".repeat(500)).await.unwrap();
    assert_eq!(most.text.chars().count(), 500);

    t.memories
        .add(&dot.id, "Prefers invoices as PDF.")
        .await
        .unwrap();
    for same in [
        "prefers invoices as pdf.",
        "  PREFERS  invoices\nas PDF. ",
        "Prefersinvoicesas PDF.",
    ] {
        let e = t
            .memories
            .save(&dot.id, same, MemorySource::Dot, Some("RUN1"))
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Conflict(_)), "{e:?}");
        assert_eq!(message(e), DUPLICATE);
    }
    // Another dot may remember the same.
    let other = t.dot("other").await;
    t.memories
        .add(&other.id, "Prefers invoices as PDF.")
        .await
        .unwrap();
    // An edit to another memory's text is a duplicate too; to its own text it is not.
    let note = t.memories.add(&dot.id, "Sends on Fridays.").await.unwrap();
    let e = t
        .memories
        .update(&note.id, Some("PREFERS invoices as PDF."), None)
        .await
        .unwrap_err();
    assert_eq!(message(e), DUPLICATE);
    let e = t
        .memories
        .update(&note.id, Some(" "), None)
        .await
        .unwrap_err();
    assert_eq!(message(e), LENGTH_RULE);
    t.memories
        .update(&note.id, Some("sends on FRIDAYS."), None)
        .await
        .unwrap();
    assert_eq!(
        t.store.get_memory(&note.id).await.unwrap().text,
        "sends on FRIDAYS."
    );
    t.changed();
    let e = t
        .memories
        .update("NOMEMORY", Some("x"), None)
        .await
        .unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
    assert!(t.changed().is_empty());
}

#[tokio::test]
async fn a_dot_keeps_at_most_200_memories_checked_with_the_insert() {
    let t = setup().await;
    let dot = t.dot("full").await;
    assert_eq!(MAX_PER_DOT, 200);
    for i in 0..MAX_PER_DOT {
        t.memories
            .add(&dot.id, &format!("Fact {i}."))
            .await
            .unwrap();
    }
    let e = t.memories.add(&dot.id, "One more.").await.unwrap_err();
    assert_eq!(message(e), FULL);
    // A duplicate says so even when the dot is full.
    let e = t.memories.add(&dot.id, "fact 7.").await.unwrap_err();
    assert_eq!(message(e), DUPLICATE);
    assert_eq!(t.memories.list(&dot.id).await.unwrap().len(), MAX_PER_DOT);

    // Twenty saves at once of a dot with 190 leave exactly 200.
    let other = t.dot("racing").await;
    for i in 0..190 {
        t.memories
            .add(&other.id, &format!("Fact {i}."))
            .await
            .unwrap();
    }
    let saves = (0..20).map(|i| {
        let memories = t.memories.clone();
        let id = other.id.clone();
        tokio::spawn(async move { memories.add(&id, &format!("Late {i}.")).await })
    });
    let mut saved = 0;
    for save in saves {
        match save.await.unwrap() {
            Ok(_) => saved += 1,
            Err(e) => assert_eq!(message(e), FULL),
        }
    }
    assert_eq!(saved, 10);
    assert_eq!(t.memories.list(&other.id).await.unwrap().len(), MAX_PER_DOT);
    // The store says which limit stopped it.
    let key = same_key("Late 0.");
    let again = t
        .store
        .add_memory(
            &other.id,
            "Late 0.",
            &key,
            MemorySource::Person,
            None,
            MAX_PER_DOT,
        )
        .await
        .unwrap();
    assert!(matches!(again, Added::Duplicate | Added::Full), "{again:?}");
}

#[tokio::test]
async fn a_deleted_dot_takes_its_memories_with_it() {
    let t = setup().await;
    let dot = t.dot("gone").await;
    let memory = t.memories.add(&dot.id, "Remember me.").await.unwrap();
    t.store.delete_dot(&dot.id).await.unwrap();
    let e = t.store.get_memory(&memory.id).await.unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

#[test]
fn text_that_looks_like_a_secret_is_refused() {
    let secrets = [
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "The key is sk-proj-abcdefghijklmnop1234",
        "sk_live_ABCDEFGHIJKLMNOP",
        "token ghp_abcdefghijklmnopqrstuvwxyz0123",
        "gho_0123456789abcdefABCD",
        "github_pat_11ABCDEFG0123456789_xyz",
        "slack xoxb-1234567890-abcdefghij",
        "xoxp-1234567890123456",
        "AWS AKIAIOSFODNN7EXAMPLE",
        "Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
        "password=hunter2hunter2",
        "Password: correct-horse",
        "db passwd : s3cr3tpass",
        "client_secret=abcdefgh",
        "API token: 1234567890",
    ];
    for text in secrets {
        assert!(looks_secret(text), "{text}");
        assert_eq!(check_text(text), Err(SECRET), "{text}");
    }
    let plain = [
        "Prefers invoices as PDF.",
        "The task-management board is at board.example.com.",
        "Ask for the token before Friday.",
        "password: short",
        "sk-short",
        "Bearer of bad news",
        "Use the secret santa list: names.txt",
        "AKIA prefix keys are AWS keys",
    ];
    for text in plain {
        assert!(!looks_secret(text), "{text}");
        assert_eq!(check_text(text).as_deref(), Ok(text), "{text}");
    }
}

#[tokio::test]
async fn a_secret_is_not_stored_by_the_person_or_the_dot() {
    let mut t = setup().await;
    let dot = t.dot("secrets").await;
    let e = t
        .memories
        .add(&dot.id, "password: hunter2hunter2")
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
    assert_eq!(message(e), SECRET);
    let e = t
        .memories
        .save(
            &dot.id,
            "key sk-abcdefghijklmnopqrstuv",
            MemorySource::Dot,
            Some("RUN1"),
        )
        .await
        .unwrap_err();
    assert_eq!(message(e), SECRET);
    let note = t.memories.add(&dot.id, "A plain note.").await.unwrap();
    let e = t
        .memories
        .update(&note.id, Some("Bearer abcdefghijklmnopqrstuvwxyz"), None)
        .await
        .unwrap_err();
    assert_eq!(message(e), SECRET);
    assert_eq!(t.memories.list(&dot.id).await.unwrap(), vec![note]);
    assert_eq!(t.changed(), vec![dot.id.clone()]);
}

#[test]
fn the_event_is_memories_updated_with_the_dot() {
    let event = RuntimeEvent::MemoriesUpdated {
        dot_id: "D1".into(),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({"type": "memories_updated", "dot_id": "D1"})
    );
    let event = RuntimeEvent::ProjectMemoriesUpdated {
        root: "C:/code/shop".into(),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({"type": "project_memories_updated", "root": "C:/code/shop"})
    );
}

/// Memory spec section 2: a dot's memory has a kind and may name files; a core memory is
/// short, since every prompt carries it.
#[tokio::test]
async fn a_dots_memory_has_a_kind_and_may_name_files() {
    use dots_core::memory::{NewFact, CORE_RULE};
    use dots_core::model::{MemoryKind, MemoryScope};
    let t = setup().await;
    let dot = t.dot("kinds").await;
    let plain = t.memories.add(&dot.id, "A plain note.").await.unwrap();
    assert_eq!(
        (plain.kind, plain.scope, plain.files.len()),
        (MemoryKind::Semantic, MemoryScope::Dot, 0)
    );
    let files = vec!["src\\main.rs".to_string(), "src/main.rs".to_string()];
    let how = t
        .memories
        .save_as(
            &dot.id,
            NewFact {
                text: "Build with cargo build --release.",
                kind: MemoryKind::Procedural,
                files: &files,
                source: MemorySource::Dot,
                run_id: Some("RUN1"),
            },
        )
        .await
        .unwrap();
    assert_eq!(how.kind, MemoryKind::Procedural);
    assert_eq!(how.files, ["src/main.rs"]);
    assert_eq!(t.store.get_memory(&how.id).await.unwrap(), how);

    // The kind changes by itself; the text and the pin stay.
    let core = t
        .memories
        .update_as(&plain.id, None, None, Some(MemoryKind::Core))
        .await
        .unwrap();
    assert_eq!(
        (core.kind, core.text.as_str()),
        (MemoryKind::Core, "A plain note.")
    );
    // A core memory is at most 160 characters, whichever of the two changes.
    let long = "x".repeat(161);
    let e = t
        .memories
        .update_as(&plain.id, Some(&long), None, None)
        .await
        .unwrap_err();
    assert_eq!(message(e), CORE_RULE);
    let e = t
        .memories
        .update_as(&how.id, Some(&long), None, Some(MemoryKind::Core))
        .await
        .unwrap_err();
    assert_eq!(message(e), CORE_RULE);
    let new = NewFact {
        kind: MemoryKind::Core,
        ..NewFact::plain(&long, MemorySource::Person, None)
    };
    assert_eq!(
        message(t.memories.save_as(&dot.id, new).await.unwrap_err()),
        CORE_RULE
    );
    t.memories
        .update_as(&plain.id, Some(&long), None, Some(MemoryKind::Semantic))
        .await
        .unwrap();
    // A file outside the project is refused.
    let outside = vec!["../secrets.txt".to_string()];
    let new = NewFact {
        files: &outside,
        ..NewFact::plain("A fact.", MemorySource::Dot, None)
    };
    assert_eq!(
        message(t.memories.save_as(&dot.id, new).await.unwrap_err()),
        dots_core::memory::files::FILES_RULE
    );
    assert_eq!(t.memories.list(&dot.id).await.unwrap().len(), 2);
}

/// Memory spec section 12: what was remembered before kinds is `semantic`; a pinned memory
/// becomes `core` while the dot's pinned ones fit what a prompt gives core memories.
#[tokio::test]
async fn memories_stored_before_kinds_keep_everything_and_get_a_kind() {
    use dots_core::model::MemoryKind;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let wide = "é".repeat(150);
    let too_long = "x".repeat(161);
    {
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let all = sqlx::migrate!("./migrations");
        let before = sqlx::migrate::Migrator {
            migrations: all
                .migrations
                .iter()
                .filter(|m| m.version < 59)
                .cloned()
                .collect::<Vec<_>>()
                .into(),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        before.run(&pool).await.unwrap();
        let ts = "2026-11-01T09:00:00.000Z";
        for dot in ["d1", "d2"] {
            sqlx::query(
                "INSERT INTO dots (id, name, instructions, engine, model, workdir, \
                 workspace_mode, webhook_token, policy, max_turns, timeout_secs, \
                 approval_wait_secs, created_at, updated_at) VALUES (?, ?, 'x', 'claude', 'm', \
                 'C:/tmp', 'folder', ?, '{}', 1, 1, 1, ?, ?)",
            )
            .bind(dot)
            .bind(dot)
            .bind(dot)
            .bind(ts)
            .bind(ts)
            .execute(&pool)
            .await
            .unwrap();
        }
        // (id, dot, text, pinned, minute): the newest has the highest minute. `plain` is the
        // one a run saved.
        let rows: Vec<(&str, &str, &str, i64, u32)> = vec![
            ("p-new", "d1", "Prefers invoices as PDF.", 1, 50),
            ("p-long", "d1", &too_long, 1, 40),
            ("p-wide-1", "d1", &wide, 1, 30),
            ("p-wide-2", "d1", &wide, 1, 20),
            ("p-old", "d1", "An old pinned note.", 1, 10),
            ("plain", "d1", "Not pinned.", 0, 45),
            ("other", "d2", "Another sol's pinned note.", 1, 5),
        ];
        for (id, dot, text, pinned, minute) in &rows {
            let run = (*id == "plain").then_some("r2");
            let source = if run.is_some() { "dot" } else { "person" };
            let at = format!("2026-11-01T09:{minute:02}:00.000Z");
            sqlx::query(
                "INSERT INTO memories (id, dot_id, text, pinned, source, run_id, created_at, \
                 updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(dot)
            .bind(text)
            .bind(pinned)
            .bind(source)
            .bind(run)
            .bind(&at)
            .bind(&at)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool.close().await;
    }
    let store = Store::open(&path).await.unwrap();
    let list = store.list_memories("d1").await.unwrap();
    assert_eq!(list.len(), 6);
    let get = |id: &str| list.iter().find(|m| m.id == id).unwrap();
    // Nothing but the kind changed.
    let newest = get("p-new");
    assert_eq!(newest.text, "Prefers invoices as PDF.");
    assert!(newest.pinned && newest.files.is_empty());
    assert_eq!(newest.source, MemorySource::Person);
    assert_eq!(newest.created_at, "2026-11-01T09:50:00.000Z");
    assert_eq!(get("plain").run_id.as_deref(), Some("r2"));
    // Newest first: the short one's line is 65 bytes, the first wide one's 341 (406 so far),
    // and the second wide one's would pass 600. The sum runs on from there, so the older one
    // behind it stays as it was too. The one of 161 characters is too long to be core.
    let kinds: Vec<(&str, MemoryKind)> =
        ["p-new", "p-long", "p-wide-1", "p-wide-2", "p-old", "plain"]
            .into_iter()
            .map(|id| (id, get(id).kind))
            .collect();
    assert_eq!(
        kinds,
        [
            ("p-new", MemoryKind::Core),
            ("p-long", MemoryKind::Semantic),
            ("p-wide-1", MemoryKind::Core),
            ("p-wide-2", MemoryKind::Semantic),
            ("p-old", MemoryKind::Semantic),
            ("plain", MemoryKind::Semantic),
        ]
    );
    // Every pin is kept, core or not.
    assert_eq!(list.iter().filter(|m| m.pinned).count(), 5);
    // Each dot's pinned memories are counted on their own.
    assert_eq!(
        store.list_memories("d2").await.unwrap()[0].kind,
        MemoryKind::Core
    );
    // What became core fits in a prompt's core lines.
    let section = dots_core::memory::prompt_section(&list);
    assert!(
        section.contains("- [p-new] Prefers invoices as PDF. (pinned)\n"),
        "{section}"
    );
    assert!(
        section.contains(&format!("- [p-wide-1] {wide} (pinned)\n")),
        "{section}"
    );
    store.close().await;
}

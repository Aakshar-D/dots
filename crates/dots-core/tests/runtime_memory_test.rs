//! The person's side of a dot's memory (dot memory spec section 4): the runtime lists, adds,
//! changes and deletes memories with the core's sentences, publishes `memories_updated`, and a
//! deleted dot's memories go with it.

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::memory::{DUPLICATE, LENGTH_RULE, SECRET};
use dots_core::model::MemorySource;
use dots_core::{Config, Error, Runtime};
use tempfile::TempDir;

struct Rt {
    _dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { _dir: dir, rt }
}

fn refusal(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

#[tokio::test]
async fn the_person_lists_adds_changes_and_deletes_a_dots_memories() {
    let t = start().await;
    let dot = t.rt.create_dot(spec("invoices")).await.unwrap();
    let mut events = t.rt.subscribe();

    let added =
        t.rt.memory_add(&dot.id, " Prefers invoices as PDF. ")
            .await
            .unwrap();
    assert_eq!(added.text, "Prefers invoices as PDF.");
    assert_eq!(added.source, MemorySource::Person);
    assert_eq!(added.run_id, None);
    let pinned =
        t.rt.memory_update(&added.id, None, Some(true))
            .await
            .unwrap();
    assert!(pinned.pinned);
    let edited =
        t.rt.memory_update(
            &added.id,
            Some("Prefers invoices as PDF, in A4."),
            Some(false),
        )
        .await
        .unwrap();
    assert_eq!(edited.text, "Prefers invoices as PDF, in A4.");
    assert!(!edited.pinned);
    assert_eq!(
        t.rt.memories_list(&dot.id).await.unwrap(),
        vec![edited.clone()]
    );

    for (text, sentence) in [
        ("", LENGTH_RULE),
        ("prefers invoices as pdf, in a4.", DUPLICATE),
        ("token: abcdefgh12345", SECRET),
    ] {
        assert_eq!(
            refusal(t.rt.memory_add(&dot.id, text).await.unwrap_err()),
            sentence
        );
    }
    t.rt.memory_delete(&added.id).await.unwrap();
    assert!(t.rt.memories_list(&dot.id).await.unwrap().is_empty());
    let gone = t.rt.memory_delete(&added.id).await.unwrap_err();
    assert!(matches!(gone, Error::NotFound(_)), "{gone:?}");
    let unknown = t.rt.memories_list("NODOT").await.unwrap_err();
    assert!(matches!(unknown, Error::NotFound(_)), "{unknown:?}");

    let mut changed = 0;
    while let Ok(event) = events.try_recv() {
        if let RuntimeEvent::MemoriesUpdated { dot_id } = event {
            assert_eq!(dot_id, dot.id);
            changed += 1;
        }
    }
    // Added, pinned, edited, deleted.
    assert_eq!(changed, 4);
}

#[tokio::test]
async fn a_deleted_dot_takes_its_memories_with_it() {
    let t = start().await;
    let dot = t.rt.create_dot(spec("gone")).await.unwrap();
    let memory = t.rt.memory_add(&dot.id, "Remember me.").await.unwrap();
    t.rt.delete_dot(&dot.id).await.unwrap();
    let e =
        t.rt.memory_update(&memory.id, None, Some(true))
            .await
            .unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

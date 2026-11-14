//! Search (search spec 2026-11-06-search-design.md): the index follows what the person and
//! the dots said, memory, goals, topic titles and chat file names as they are written,
//! changed and deleted; what the person types is never read as query syntax; hits come best
//! first with the matched words marked, within one account's workspace.

mod common;

use common::{spec, temp_store};
use dots_core::intents::IntentFields;
use dots_core::model::{Dot, MemorySource, NewRun, Run, RunStatus, TriggerKind};
use dots_core::search::{
    fts_query, SearchHit, SearchKind, SearchQuery, SearchTarget, HIT_END, HIT_START, MAX_WORDS,
};
use dots_core::store::Store;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const WS: &str = "default";

async fn dot(store: &Store, name: &str) -> Dot {
    store.create_dot(&spec(name), None, WS).await.unwrap()
}

/// A chat run of `dot` saying `message`, continuing `parent` (none starts a topic).
async fn chat(store: &Store, dot: &Dot, parent: Option<&Run>, message: &str) -> Run {
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({ "message": message, "typed": true }));
    if let Some(p) = parent {
        new.parent_run_id = Some(p.id.clone());
        new.root_run_id = Some(p.root_run_id.clone());
    }
    store.create_run(&new).await.unwrap()
}

async fn find(store: &Store, text: &str) -> Vec<SearchHit> {
    search(
        store,
        SearchQuery {
            text: text.into(),
            ..Default::default()
        },
    )
    .await
}

async fn search(store: &Store, query: SearchQuery) -> Vec<SearchHit> {
    store.search(None, WS, &query).await.unwrap()
}

fn kinds(hits: &[SearchHit]) -> Vec<SearchKind> {
    hits.iter().map(|h| h.kind).collect()
}

fn goal(text: &str) -> IntentFields {
    IntentFields {
        text: text.into(),
        due_date: None,
        urgent: false,
        dot_id: None,
    }
}

#[test]
fn what_the_person_types_becomes_quoted_words_and_the_last_one_a_prefix() {
    assert_eq!(
        fts_query("lisbon trip").as_deref(),
        Some("\"lisbon\" \"trip\"*")
    );
    // A space or a mark after the last word says it is whole.
    assert_eq!(
        fts_query("lisbon trip ").as_deref(),
        Some("\"lisbon\" \"trip\"")
    );
    assert_eq!(fts_query("trip?").as_deref(), Some("\"trip\""));
    // Query syntax is only separators between words.
    assert_eq!(
        fts_query("\"a OR b\" -c* NEAR(d) body:e (f").as_deref(),
        Some("\"a\" \"OR\" \"b\" \"c\" \"NEAR\" \"d\" \"body\" \"e\" \"f\"*")
    );
    assert_eq!(
        fts_query("café über").as_deref(),
        Some("\"café\" \"über\"*")
    );
    assert_eq!(fts_query(""), None);
    assert_eq!(fts_query("  \"*- () :^ "), None);
    let many = (0..MAX_WORDS + 3)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let query = fts_query(&many).unwrap();
    assert_eq!(query.matches('"').count(), MAX_WORDS * 2);
    assert!(
        !query.ends_with('*'),
        "a cut-off word is not a prefix: {query}"
    );
}

#[tokio::test]
async fn hostile_text_searches_without_an_error() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "a").await;
    chat(&store, &d, None, "the OR operator and NEAR words").await;
    for text in [
        "\"", "\"\"", "-", "*", "-foo", "foo*", "a OR", "OR", "AND NOT", "NEAR(", "body:x", "(",
        ")", "^", "'", "%", "_", "\\", "near/2", "{a b}", "a\u{0}b",
    ] {
        store
            .search(
                None,
                WS,
                &SearchQuery {
                    text: text.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{text:?} failed: {e}"));
    }
    // Words that are operators elsewhere are plain words here.
    assert_eq!(
        kinds(&find(&store, "OR operator").await),
        vec![SearchKind::Chat]
    );
    assert_eq!(kinds(&find(&store, "near").await), vec![SearchKind::Chat]);
}

#[tokio::test]
async fn a_chat_message_and_the_dots_reply_are_found_in_their_topic() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "planner").await;
    let root = chat(&store, &d, None, "Plan the Lisbon trip").await;
    let next = chat(&store, &d, Some(&root), "And the hotel?").await;
    store
        .append_event(
            &next.id,
            "assistant_text",
            &json!({ "kind": "assistant_text", "text": "Looking at Lisbon hotels now." }),
        )
        .await
        .unwrap();
    store
        .append_event(&next.id, "tool_call", &json!({ "kind": "tool_call", "id": "1", "tool": "Bash", "input": { "command": "echo lisbon" } }))
        .await
        .unwrap();
    store
        .finish_run(
            &next.id,
            RunStatus::Succeeded,
            Some("Booked a Lisbon hotel by the river."),
            None,
        )
        .await
        .unwrap();

    let hits = find(&store, "lisbon").await;
    // The reply and the result of one run are one hit; a tool call is not indexed.
    assert_eq!(hits.len(), 2, "{hits:#?}");
    let said = hits.iter().find(|h| h.kind == SearchKind::Chat).unwrap();
    assert_eq!(said.dot_name.as_deref(), Some("planner"));
    assert_eq!(
        said.target,
        SearchTarget::Topic {
            dot_id: d.id.clone(),
            root_run_id: root.id.clone(),
            run_id: Some(root.id.clone())
        }
    );
    let reply = hits.iter().find(|h| h.kind == SearchKind::Reply).unwrap();
    assert_eq!(
        reply.target,
        SearchTarget::Topic {
            dot_id: d.id.clone(),
            root_run_id: root.id.clone(),
            run_id: Some(next.id.clone())
        }
    );
    assert!(
        reply
            .snippet
            .contains(&format!("{HIT_START}Lisbon{HIT_END}")),
        "{}",
        reply.snippet
    );

    // The last word matches words it starts.
    assert_eq!(find(&store, "hot").await.len(), 2);
    assert_eq!(kinds(&find(&store, "river").await), vec![SearchKind::Reply]);
    assert!(find(&store, "echo").await.is_empty());
}

#[tokio::test]
async fn a_run_that_did_not_start_in_a_chat_opens_its_own_page() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "nightly").await;
    let run = store
        .create_run(&NewRun::new(&d.id, TriggerKind::Schedule))
        .await
        .unwrap();
    store
        .finish_run(
            &run.id,
            RunStatus::Succeeded,
            Some("Three invoices were overdue."),
            None,
        )
        .await
        .unwrap();
    let hits = find(&store, "invoices").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].target,
        SearchTarget::Run {
            run_id: run.id.clone()
        }
    );
}

#[tokio::test]
async fn memory_goals_titles_notes_and_files_are_found_and_follow_changes() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "helper").await;
    let root = chat(&store, &d, None, "hello").await;

    let added = store
        .add_memory(
            &d.id,
            "Prefers aisle seats",
            "prefers aisle seats",
            MemorySource::Person,
            None,
            100,
        )
        .await
        .unwrap();
    let memory = match added {
        dots_core::store::Added::Saved(m) => m,
        _ => panic!("the memory was not saved"),
    };
    let intent = store
        .add_intent(None, WS, &goal("Renew the passport"))
        .await
        .unwrap();
    store
        .set_topic_title(&root.id, Some("Summer travel"))
        .await
        .unwrap();
    store
        .add_question(&d.id, &root.id, "Which airline do you like?", None)
        .await
        .unwrap();
    store
        .add_note(&d.id, None, "Sage restarted the scheduler")
        .await
        .unwrap();
    store
        .insert_staged_attachment("f1", &d.id, "itinerary.pdf", 10)
        .await
        .unwrap();
    store
        .insert_staged_attachment("f2", &d.id, "draft-notes.txt", 10)
        .await
        .unwrap();
    store
        .create_chat_run(
            &{
                let mut n = NewRun::new(&d.id, TriggerKind::Chat);
                n.payload = Some(json!({ "message": "here it is" }));
                n.parent_run_id = Some(root.id.clone());
                n.root_run_id = Some(root.id.clone());
                n
            },
            &[("f1".into(), "itinerary.pdf".into())],
            None,
            None,
        )
        .await
        .unwrap();

    let one = |hits: Vec<SearchHit>| {
        assert_eq!(hits.len(), 1, "{hits:#?}");
        hits.into_iter().next().unwrap()
    };
    let m = one(find(&store, "aisle").await);
    assert_eq!(
        m.target,
        SearchTarget::Memory {
            dot_id: d.id.clone(),
            memory_id: memory.id.clone()
        }
    );
    let g = one(find(&store, "passport").await);
    assert_eq!(
        g.target,
        SearchTarget::Intent {
            intent_id: intent.id.clone()
        }
    );
    assert_eq!(g.dot_id, None);
    let t = one(find(&store, "summer").await);
    assert_eq!(t.kind, SearchKind::Topic);
    assert_eq!(
        t.target,
        SearchTarget::Topic {
            dot_id: d.id.clone(),
            root_run_id: root.id.clone(),
            run_id: None
        }
    );
    let q = one(find(&store, "airline").await);
    assert_eq!(q.kind, SearchKind::Note);
    assert!(
        matches!(q.target, SearchTarget::Topic { ref run_id, .. } if run_id.as_deref() == Some(root.id.as_str()))
    );
    let n = one(find(&store, "scheduler").await);
    assert_eq!(
        n.target,
        SearchTarget::Dot {
            dot_id: d.id.clone()
        }
    );
    let f = one(find(&store, "itinerary").await);
    assert_eq!(f.kind, SearchKind::File);
    // A staged file is still the person's draft.
    assert!(find(&store, "draft").await.is_empty());

    // A change replaces the old words.
    store
        .update_memory(
            &memory.id,
            Some(("Prefers window seats", "prefers window seats")),
            None,
        )
        .await
        .unwrap();
    assert!(find(&store, "aisle").await.is_empty());
    assert_eq!(one(find(&store, "window").await).kind, SearchKind::Memory);
    store
        .set_intent_fields(&intent.id, &goal("Renew the driving licence"))
        .await
        .unwrap();
    assert!(find(&store, "passport").await.is_empty());
    assert_eq!(one(find(&store, "licence").await).kind, SearchKind::Intent);
    store.set_topic_title(&root.id, None).await.unwrap();
    assert!(find(&store, "summer").await.is_empty());

    // A delete takes the words away.
    store.delete_memory(&memory.id).await.unwrap();
    assert!(find(&store, "window").await.is_empty());
    store.delete_intent(&intent.id).await.unwrap();
    assert!(find(&store, "licence").await.is_empty());
}

#[tokio::test]
async fn a_deleted_dot_leaves_nothing_to_find() {
    let (_d, store) = temp_store().await;
    let d = dot(&store, "gone").await;
    let root = chat(&store, &d, None, "zanzibar").await;
    store
        .append_event(
            &root.id,
            "assistant_text",
            &json!({ "kind": "assistant_text", "text": "zanzibar again" }),
        )
        .await
        .unwrap();
    store
        .finish_run(&root.id, RunStatus::Succeeded, Some("zanzibar done"), None)
        .await
        .unwrap();
    store
        .set_topic_title(&root.id, Some("zanzibar"))
        .await
        .unwrap();
    store
        .add_memory(
            &d.id,
            "zanzibar fact",
            "zanzibar fact",
            MemorySource::Dot,
            None,
            100,
        )
        .await
        .unwrap();
    assert_eq!(find(&store, "zanzibar").await.len(), 4);

    store.delete_dot(&d.id).await.unwrap();
    assert!(find(&store, "zanzibar").await.is_empty());
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_docs")
        .fetch_one(&pool_of(&_d).await)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

async fn pool_of(dir: &tempfile::TempDir) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(dir.path().join("dots.db")))
        .await
        .unwrap()
}

#[tokio::test]
async fn the_closer_match_comes_first_and_filters_narrow_the_hits() {
    let (_d, store) = temp_store().await;
    let a = dot(&store, "alpha").await;
    let b = dot(&store, "beta").await;
    chat(&store, &a, None, "budget").await;
    chat(&store, &b, None, "a long message that mentions the budget once among many other words about lunch and travel").await;
    store
        .add_memory(
            &b.id,
            "The budget is tight",
            "the budget is tight",
            MemorySource::Person,
            None,
            100,
        )
        .await
        .unwrap();

    let hits = find(&store, "budget").await;
    assert_eq!(hits.len(), 3);
    assert_eq!(
        hits[0].dot_name.as_deref(),
        Some("alpha"),
        "the short text ranks first: {hits:#?}"
    );

    let only_b = search(
        &store,
        SearchQuery {
            text: "budget".into(),
            dot_id: Some(b.id.clone()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(only_b.len(), 2);
    assert!(only_b
        .iter()
        .all(|h| h.dot_id.as_deref() == Some(b.id.as_str())));
    let memory = search(
        &store,
        SearchQuery {
            text: "budget".into(),
            kinds: vec![SearchKind::Memory],
            ..Default::default()
        },
    )
    .await;
    assert_eq!(kinds(&memory), vec![SearchKind::Memory]);
    let one = search(
        &store,
        SearchQuery {
            text: "budget".into(),
            limit: Some(1),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(one.len(), 1);
}

#[tokio::test]
async fn another_workspace_or_account_is_not_searched() {
    let (_d, store) = temp_store().await;
    let here = dot(&store, "here").await;
    let there = store.create_dot(&spec("there"), None, "w2").await.unwrap();
    let org = store
        .create_dot(&spec("org"), Some("org-1"), WS)
        .await
        .unwrap();
    for d in [&here, &there, &org] {
        chat(&store, d, None, "quarterly numbers").await;
    }
    store
        .add_intent(None, "w2", &goal("quarterly review"))
        .await
        .unwrap();
    store
        .add_intent(Some("org-1"), WS, &goal("quarterly review"))
        .await
        .unwrap();

    let hits = find(&store, "quarterly").await;
    assert_eq!(hits.len(), 1, "{hits:#?}");
    assert_eq!(hits[0].dot_id.as_deref(), Some(here.id.as_str()));
    let org_hits = store
        .search(
            Some("org-1"),
            WS,
            &SearchQuery {
                text: "quarterly".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(org_hits.len(), 2);
}

#[tokio::test]
async fn what_was_stored_before_the_index_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
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
                .filter(|m| m.version < 37)
                .cloned()
                .collect::<Vec<_>>()
                .into(),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        before.run(&pool).await.unwrap();
        let ts = "2026-11-01T09:00:00.000Z";
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, created_at, \
             updated_at) VALUES ('d1', 'old', 'x', 'claude', 'm', 'C:/tmp', 'folder', 't', '{}', \
             1, 1, 1, ?, ?)",
        )
        .bind(ts)
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO runs (id, dot_id, root_run_id, trigger_kind, payload, status, summary, \
             queued_at) VALUES ('r1', 'd1', 'r1', 'chat', '{\"message\":\"find the walrus\"}', \
             'succeeded', 'The walrus is in Svalbard.', ?)",
        )
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO run_events (run_id, seq, ts, kind, data) VALUES ('r1', 1, ?, \
             'assistant_text', '{\"kind\":\"assistant_text\",\"text\":\"Searching for the narwhal\"}')",
        )
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memories (id, dot_id, text, source, created_at, updated_at) \
             VALUES ('m1', 'd1', 'Likes penguins', 'person', ?, ?)",
        )
        .bind(ts)
        .bind(ts)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO topics (root_run_id, dot_id, title) VALUES ('r1', 'd1', 'Arctic animals')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }
    let store = Store::open(&path).await.unwrap();
    assert_eq!(kinds(&find(&store, "walrus").await).len(), 2);
    assert_eq!(
        kinds(&find(&store, "narwhal").await),
        vec![SearchKind::Reply]
    );
    assert_eq!(
        kinds(&find(&store, "penguins").await),
        vec![SearchKind::Memory]
    );
    assert_eq!(
        kinds(&find(&store, "arctic").await),
        vec![SearchKind::Topic]
    );
}

#[tokio::test]
async fn no_trigger_writes_to_the_index_and_its_queue_empties_on_search() {
    let (d, store) = temp_store().await;
    let pool = pool_of(&d).await;
    // A statement whose triggers reach the index would read its settings before taking the
    // write lock, and fail at once when another connection wrote meanwhile.
    let reaching: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'trigger' AND sql LIKE '%search_fts%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(reaching.is_empty(), "{reaching:?}");

    let a = dot(&store, "a").await;
    let run = chat(&store, &a, None, "ephemeral words").await;
    store
        .set_topic_title(&run.id, Some("ephemeral"))
        .await
        .unwrap();
    store.set_topic_title(&run.id, None).await.unwrap();
    let queued = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM search_queue")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    // The title that came and went never reached the queue's inserts.
    assert_eq!(queued().await, 1);
    assert_eq!(find(&store, "ephemeral").await.len(), 1);
    assert_eq!(queued().await, 0);
}

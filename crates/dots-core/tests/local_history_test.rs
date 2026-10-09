mod common;

use common::{spec, temp_store};
use dots_core::engine::local::history::{self, INTERRUPTED};
use dots_core::model::{EngineKind, NewRun, TriggerKind};
use serde_json::{json, Value};

fn user(t: &str) -> Value {
    json!({"role": "user", "content": t})
}

fn assistant(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn calling(ids: &[&str]) -> Value {
    let calls: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({"id": id, "type": "function",
                   "function": {"name": "list_dir", "arguments": "{}"}})
        })
        .collect();
    json!({"role": "assistant", "content": "", "tool_calls": calls})
}

fn tool(id: &str, out: &str) -> Value {
    json!({"role": "tool", "tool_call_id": id, "content": out})
}

#[test]
fn repair_keeps_a_valid_conversation_unchanged() {
    let conv = vec![
        json!({"role": "system", "content": "s"}),
        user("u"),
        calling(&["a", "b"]),
        tool("a", "1"),
        tool("b", "2"),
        assistant("done"),
    ];
    assert_eq!(history::repair(conv.clone()), conv);
}

#[test]
fn repair_answers_unanswered_tool_calls() {
    let conv = vec![
        user("u"),
        calling(&["a", "b"]),
        tool("a", "1"),
        user("next"),
    ];
    assert_eq!(
        history::repair(conv),
        vec![
            user("u"),
            calling(&["a", "b"]),
            tool("a", "1"),
            tool("b", INTERRUPTED),
            user("next"),
        ]
    );
    let trailing = vec![user("u"), calling(&["a"])];
    assert_eq!(
        history::repair(trailing),
        vec![user("u"), calling(&["a"]), tool("a", INTERRUPTED)]
    );
}

#[test]
fn repair_drops_orphan_results_and_merges_user_messages() {
    let conv = vec![
        user("first"),
        tool("ghost", "x"),
        user("second"),
        assistant("ok"),
    ];
    assert_eq!(
        history::repair(conv),
        vec![user("first\n\nsecond"), assistant("ok")]
    );
}

#[tokio::test]
async fn load_follows_the_parent_chain_in_order() {
    let (_d, store) = temp_store().await;
    let mut s = spec("hist");
    s.engine = EngineKind::Local;
    s.endpoint_url = Some("http://127.0.0.1:1".into());
    let dot = store.create_dot(&s).await.unwrap();
    let root = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let child_of = |parent: &str| {
        let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
        new.parent_run_id = Some(parent.to_string());
        new.root_run_id = Some(root.id.clone());
        new
    };
    let child = store.create_run(&child_of(&root.id)).await.unwrap();
    let grandchild = store.create_run(&child_of(&child.id)).await.unwrap();
    // A sibling branch of the same root must not leak into the chain.
    let sibling = store.create_run(&child_of(&root.id)).await.unwrap();

    let add = |run: String, m: Value| {
        let store = store.clone();
        async move {
            store
                .append_event(&run, "message", &json!({"kind": "message", "message": m}))
                .await
                .unwrap();
        }
    };
    add(root.id.clone(), user("one")).await;
    store
        .append_event(
            &root.id,
            "assistant_text",
            &json!({"kind": "assistant_text", "text": "x"}),
        )
        .await
        .unwrap();
    add(root.id.clone(), calling(&["a"])).await;
    add(child.id.clone(), user("two")).await;
    add(child.id.clone(), assistant("ok")).await;
    add(sibling.id.clone(), user("other")).await;
    add(grandchild.id.clone(), user("three")).await;

    assert_eq!(
        history::load(&store, &grandchild.id).await.unwrap(),
        vec![
            user("one"),
            calling(&["a"]),
            tool("a", INTERRUPTED),
            user("two"),
            assistant("ok"),
        ]
    );
    assert!(history::load(&store, &root.id).await.unwrap().is_empty());
}

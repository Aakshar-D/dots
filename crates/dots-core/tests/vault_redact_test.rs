//! Scrubbing secret values from what Dots keeps (env vault spec §10).

mod common;

use std::io::Write;

use common::{spec, temp_store};
use dots_core::model::{NewRun, RunStatus, TriggerKind};
use dots_core::vault::redact::{global, spellings, RedactWriter, Redactor};
use dots_core::vault::{EntryKind, Vault};
use serde_json::json;

#[test]
fn every_common_spelling_of_a_secret_is_scrubbed() {
    let r = Redactor::default();
    r.add("API_KEY", "p@ss word/+=\"x");
    let text = format!(
        "raw {} url {} plus {} json {} b64 {}",
        "p@ss word/+=\"x",
        "p%40ss%20word%2F%2B%3D%22x",
        "p%40ss+word%2F%2B%3D%22x",
        "p@ss word/+=\\\"x",
        "cEBzcyB3b3JkLys9Ing=",
    );
    let out = r.scrub(&text);
    assert!(!out.contains("p@ss"), "{out}");
    assert!(!out.contains("p%40ss"), "{out}");
    assert!(!out.contains("cEBzcyB3b3JkLys9Ing"), "{out}");
    assert_eq!(out.matches("[secret:API_KEY]").count(), 5, "{out}");
    assert!(spellings("abc").contains(&"YWJj".to_string()));
}

#[test]
fn a_short_value_is_left_alone_and_a_longer_secret_wins() {
    let r = Redactor::default();
    r.add("SHORT", "1234567");
    r.add("OUTER", "token-abcdefgh-1234");
    r.add("INNER", "abcdefgh");
    assert_eq!(r.scrub("1234567").as_ref(), "1234567");
    assert_eq!(
        r.scrub("x token-abcdefgh-1234 y").as_ref(),
        "x [secret:OUTER] y"
    );
    assert_eq!(r.scrub("abcdefgh").as_ref(), "[secret:INNER]");
    assert_eq!(
        r.scrub_json(&json!({"a": ["abcdefgh", 1], "abcdefgh": null})),
        json!({"a": ["[secret:INNER]", 1], "[secret:INNER]": null})
    );
}

#[tokio::test]
async fn what_the_store_keeps_of_a_run_holds_no_secret_it_has_read() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    vault
        .add(
            "DOTS_REDACT_TOKEN",
            EntryKind::Secret,
            "ghp_redact_test_value_42",
        )
        .await
        .unwrap();
    let secret = "ghp_redact_test_value_42";
    let dot = store.create_dot(&spec("redact")).await.unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Webhook);
    new.payload = Some(json!({"echo": format!("token={secret}")}));
    let run = store.create_run(&new).await.unwrap();
    assert_eq!(
        run.payload,
        Some(json!({"echo": "token=[secret:DOTS_REDACT_TOKEN]"}))
    );
    let ev = store
        .append_event(
            &run.id,
            "tool_result",
            &json!({"output": format!("Bearer {secret}")}),
        )
        .await
        .unwrap();
    assert_eq!(ev.data["output"], "Bearer [secret:DOTS_REDACT_TOKEN]");
    let stored = store.list_events(&run.id, 0).await.unwrap();
    assert!(!serde_json::to_string(&stored).unwrap().contains(secret));
    let approval = store
        .create_approval(
            &run.id,
            "Bash",
            &json!({"command": format!("curl -H 'x: {secret}'")}),
        )
        .await
        .unwrap();
    assert!(!approval.input.to_string().contains(secret));
    let finished = store
        .finish_run(
            &run.id,
            RunStatus::Failed,
            Some(secret),
            Some(&format!("failed: {secret}")),
        )
        .await
        .unwrap();
    assert!(!format!("{:?} {:?}", finished.summary, finished.error).contains(secret));
    let note = store
        .add_note(&dot.id, Some(&run.id), secret)
        .await
        .unwrap();
    assert_eq!(note.text, "[secret:DOTS_REDACT_TOKEN]");
}

#[test]
fn the_log_writer_scrubs_each_line() {
    global().add("DOTS_REDACT_LOG", "log-secret-value-77");
    let mut out = Vec::new();
    {
        let mut w = RedactWriter::new(&mut out);
        w.write_all(b"INFO MCP server stderr: auth log-secret-value-77 ok\n")
            .unwrap();
    }
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "INFO MCP server stderr: auth [secret:DOTS_REDACT_LOG] ok\n"
    );
}

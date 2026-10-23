//! Scrubbing secret values from what Dots keeps (env vault spec §10).

mod common;

use std::io::Write;
use std::sync::Arc;

use common::{spec, temp_store};
use dots_core::approvals::grant_key;
use dots_core::engine::local::tools::{cap_head, cap_tail, MAX_OUTPUT};
use dots_core::model::{NewRun, RunStatus, TriggerKind};
use dots_core::proc::read_scrubbed_tail;
use dots_core::vault::redact::{global, spellings, RedactWriter, Redactor};
use dots_core::vault::{EntryKind, MemorySecrets, SecretStore, Vault};
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

/// Whether `text` holds any 8 bytes of `secret` in a row.
fn holds_piece(text: &str, secret: &str) -> bool {
    secret
        .as_bytes()
        .windows(8)
        .any(|w| text.contains(std::str::from_utf8(w).unwrap()))
}

#[test]
fn base64_is_scrubbed_wherever_the_secret_sits_in_the_encoded_text() {
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    use base64::Engine as _;
    let secret = "ghp_b64_position_value_7788";
    let r = Redactor::default();
    r.add("B64_TOKEN", secret);
    // HTTP Basic: the token follows `user:`, so it is at offset 5, which is 2 mod 3.
    for prefix in ["", "u:", "user:", "usr:"] {
        for suffix in ["", "\n", "\r\n", ":x"] {
            let plain = format!("{prefix}{secret}{suffix}");
            let engines = [&STANDARD_NO_PAD, &URL_SAFE_NO_PAD];
            for engine in engines {
                let text = format!("Authorization: Basic {}", engine.encode(&plain));
                let out = r.scrub(&text);
                assert!(out.contains("[secret:B64_TOKEN]"), "{out}");
                // The characters wholly inside the secret are gone: no 8 of them in a row.
                let offset = prefix.len() % 3;
                let mut bytes = vec![0u8; offset];
                bytes.extend_from_slice(secret.as_bytes());
                let encoded = engine.encode(&bytes);
                let inner = &encoded[(8 * offset).div_ceil(6)..8 * (offset + secret.len()) / 6];
                for w in inner.as_bytes().windows(8) {
                    let w = std::str::from_utf8(w).unwrap();
                    assert!(!out.contains(w), "{prefix:?} {suffix:?} {w}: {out}");
                }
            }
        }
    }
}

#[tokio::test]
async fn a_parked_approval_that_holds_a_secret_still_matches_its_grant() {
    let (_dir, store) = temp_store().await;
    let secret = "ghp_grant_key_value_9911";
    global().add("DOTS_GRANT_TOKEN", secret);
    let dot = store.create_dot(&spec("grant")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();
    let raw = json!({"command": format!("curl -H 'x: {secret}' host"), "description": "d"});
    let approval = store.create_approval(&run.id, "Bash", &raw).await.unwrap();
    assert!(!approval.input.to_string().contains(secret));
    // The resume path keys the grant by the stored input; the retry presents the raw one.
    store
        .create_grant(
            &run.root_run_id,
            &approval.tool,
            &grant_key(&approval.tool, &approval.input),
        )
        .await
        .unwrap();
    assert!(store
        .take_grant(&run.root_run_id, "Bash", &grant_key("Bash", &raw))
        .await
        .unwrap());
}

#[tokio::test]
async fn notes_of_approvals_and_follow_ups_are_scrubbed() {
    let (_dir, store) = temp_store().await;
    let secret = "ghp_note_scrub_value_5566";
    global().add("DOTS_NOTE_TOKEN", secret);
    let dot = store.create_dot(&spec("notes")).await.unwrap();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Webhook))
        .await
        .unwrap();
    let approval = store
        .create_approval(&run.id, "Bash", &json!({"command": "ls"}))
        .await
        .unwrap();
    let decided = store
        .decide_approval(&approval.id, false, Some(&format!("no, {secret}")))
        .await
        .unwrap();
    assert_eq!(
        decided.note.as_deref(),
        Some("no, [secret:DOTS_NOTE_TOKEN]")
    );
    let now = chrono::Utc::now();
    let follow_up = store
        .add_follow_up(&dot.id, &run.id, now, &format!("check {secret}"), 5, now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(follow_up.note, "check [secret:DOTS_NOTE_TOKEN]");
}

#[tokio::test]
async fn a_value_the_vault_reads_is_scrubbed_from_then_on() {
    let (_dir, store) = temp_store().await;
    let secrets = Arc::new(MemorySecrets::new());
    let vault = Vault::new(store, secrets.clone());
    vault
        .add(
            "DOTS_READ_ONLY_TOKEN",
            EntryKind::Secret,
            "first-value-12345",
        )
        .await
        .unwrap();
    // The value changes behind the vault's back, so only a read can tell the redactor.
    let later = "changed-behind-vault-67890";
    secrets.put("personal/DOTS_READ_ONLY_TOKEN", later).unwrap();
    assert_eq!(global().scrub(later).as_ref(), later);
    let read = vault
        .personal_values(&["DOTS_READ_ONLY_TOKEN".to_string()])
        .await
        .unwrap();
    assert!(read.contains("DOTS_READ_ONLY_TOKEN"));
    assert_eq!(
        global().scrub(later).as_ref(),
        "[secret:DOTS_READ_ONLY_TOKEN]"
    );
}

#[test]
fn a_head_or_tail_cut_never_leaves_a_piece_of_a_secret() {
    let secret = "ghp_cut_straddle_value_0123456789";
    global().add("DOTS_CUT_TOKEN", secret);
    // The secret straddles the cut: 12 bytes of it lie on the kept side.
    let head = format!(
        "{}{secret}{}",
        "h".repeat(MAX_OUTPUT - 12),
        "t".repeat(MAX_OUTPUT)
    );
    let out = cap_head(head);
    assert!(!holds_piece(&out, secret), "head: {out}");
    let tail = format!(
        "{}{secret}{}",
        "h".repeat(MAX_OUTPUT),
        "t".repeat(MAX_OUTPUT - secret.len() + 12)
    );
    let out = cap_tail(tail);
    assert!(!holds_piece(&out, secret), "tail: {out}");
}

#[tokio::test]
async fn a_streamed_tail_never_leaves_a_piece_of_a_secret() {
    let secret = "ghp_stream_tail_value_0123456789";
    global().add("DOTS_TAIL_TOKEN", secret);
    let max = 100;
    // 12 bytes of the secret lie in the last `max` bytes read.
    let data = format!(
        "{}{secret}{}",
        "h".repeat(500),
        "t".repeat(max - secret.len() + 12)
    );
    let out = read_scrubbed_tail(data.as_bytes(), max).await;
    assert!(!holds_piece(&out, secret), "{out}");
    assert!(out.len() <= max);
    // Without a secret near the cut the tail is the last `max` bytes.
    let plain = format!("{}{}", "p".repeat(500), "q".repeat(max));
    assert_eq!(
        read_scrubbed_tail(plain.as_bytes(), max).await,
        "q".repeat(max)
    );
}

#[tokio::test]
async fn a_vault_opened_over_kept_secrets_scrubs_them_before_any_read() {
    let (_dir, store) = temp_store().await;
    // Secrets kept from an earlier start: their rows and values exist, and nothing read them.
    store
        .add_env_entry("P1_KEPT_TOKEN", EntryKind::Secret, None)
        .await
        .unwrap();
    store
        .add_env_entry("P1_BROKEN_TOKEN", EntryKind::Secret, None)
        .await
        .unwrap();
    store
        .add_env_entry(
            "P1_PLAIN",
            EntryKind::Plain,
            Some("plain-is-not-a-secret-p1"),
        )
        .await
        .unwrap();
    let secrets = common::secrets::FailingSecrets::new("P1_BROKEN");
    secrets.preload("personal/P1_KEPT_TOKEN", "kept-secret-value-p1");
    // A read error for one name is logged and skipped: the others still load.
    let _vault = Vault::open(store, Arc::new(secrets)).await.unwrap();
    assert_eq!(
        global().scrub("log: kept-secret-value-p1").as_ref(),
        "log: [secret:P1_KEPT_TOKEN]"
    );
    assert_eq!(
        global().scrub("plain-is-not-a-secret-p1").as_ref(),
        "plain-is-not-a-secret-p1"
    );
}

#[tokio::test]
async fn the_runtime_loads_personal_secrets_into_the_redactor_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    {
        let store = dots_core::store::Store::open(&data.join("dots.db"))
            .await
            .unwrap();
        store
            .add_env_entry("P1_START_TOKEN", EntryKind::Secret, None)
            .await
            .unwrap();
    }
    let secrets = MemorySecrets::new();
    secrets
        .put("personal/P1_START_TOKEN", "start-secret-value-p1")
        .unwrap();
    let mut cfg = dots_core::Config::new(&data);
    cfg.port = 0;
    cfg.secrets = Some(Arc::new(secrets));
    let rt = dots_core::Runtime::start(cfg).await.unwrap();
    assert_eq!(
        global().scrub("start-secret-value-p1").as_ref(),
        "[secret:P1_START_TOKEN]"
    );
    rt.shutdown();
}

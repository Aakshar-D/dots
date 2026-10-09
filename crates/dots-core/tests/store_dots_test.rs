mod common;

use chrono::{Local, TimeZone};
use common::{spec, temp_store};
use dots_core::model::EngineKind;
use dots_core::policy::Preset;
use dots_core::scheduler::{next_fire, parse_cron};
use dots_core::Error;
use serde_json::json;

#[tokio::test]
async fn create_and_get_round_trip() {
    let (_d, store) = temp_store().await;
    let mut s = spec("triage");
    s.schedule = Some("0 0 8 * * Mon-Fri".into());
    s.mcp_servers = Some(json!({"sf": {"type": "stdio", "command": "sf-mcp"}}));
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec, s);
    assert_eq!(dot.id.len(), 26);
    assert_eq!(dot.webhook_token.len(), 43);
    assert_eq!(dot.created_at, dot.updated_at);
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);
    assert_eq!(dot.spec.policy.preset, Preset::Sandboxed);
}

#[tokio::test]
async fn duplicate_name_conflicts() {
    let (_d, store) = temp_store().await;
    store.create_dot(&spec("a")).await.unwrap();
    let err = store.create_dot(&spec("a")).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
}

#[tokio::test]
async fn invalid_specs_are_rejected() {
    let (_d, store) = temp_store().await;
    let bad_name = spec("has space");
    let mut local_no_endpoint = spec("loc");
    local_no_endpoint.engine = EngineKind::Local;
    let mut five_field_cron = spec("cron5");
    five_field_cron.schedule = Some("0 8 * * *".into());
    let mut reserved_mcp = spec("mcp");
    reserved_mcp.mcp_servers = Some(json!({"dots": {}}));
    let mut zero_turns = spec("zero");
    zero_turns.max_turns = 0;
    let mut local_https = spec("https");
    local_https.engine = EngineKind::Local;
    local_https.endpoint_url = Some("https://example.com".into());
    let mut local_no_host = spec("nohost");
    local_no_host.engine = EngineKind::Local;
    local_no_host.endpoint_url = Some("http:///v1".into());
    for s in [
        bad_name,
        local_no_endpoint,
        local_https,
        local_no_host,
        five_field_cron,
        reserved_mcp,
        zero_turns,
    ] {
        let err = store.create_dot(&s).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{}: {err:?}", s.name);
    }
    assert!(store.list_dots().await.unwrap().is_empty());
}

#[tokio::test]
async fn update_changes_fields_and_timestamp() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let mut s = dot.spec.clone();
    s.instructions = "New instructions".into();
    s.max_turns = 7;
    let updated = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(updated.spec.instructions, "New instructions");
    assert_eq!(updated.spec.max_turns, 7);
    assert_eq!(updated.webhook_token, dot.webhook_token);
    assert!(updated.updated_at > dot.updated_at);
    assert!(matches!(
        store.update_dot("missing", &s).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn list_is_sorted_by_name() {
    let (_d, store) = temp_store().await;
    for n in ["c", "a", "b"] {
        store.create_dot(&spec(n)).await.unwrap();
    }
    let names: Vec<String> = store
        .list_dots()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.spec.name)
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
}

#[tokio::test]
async fn delete_enable_and_token_regeneration() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("a")).await.unwrap();
    let disabled = store.set_dot_enabled(&dot.id, false).await.unwrap();
    assert!(!disabled.spec.enabled);
    let token = store.regenerate_webhook_token(&dot.id).await.unwrap();
    assert_ne!(token, dot.webhook_token);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().webhook_token, token);
    store.delete_dot(&dot.id).await.unwrap();
    assert!(matches!(
        store.get_dot(&dot.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.delete_dot(&dot.id).await,
        Err(Error::NotFound(_))
    ));
}

#[test]
fn cron_requires_seconds_field() {
    assert!(parse_cron("0 0 8 * * Mon-Fri").is_ok());
    assert!(parse_cron("0 0 8 * * * 2027").is_ok());
    assert!(parse_cron("0 8 * * *").is_err());
    assert!(parse_cron("not cron at all x").is_err());
}

#[test]
fn next_fire_is_strictly_after() {
    let at = Local.with_ymd_and_hms(2026, 10, 8, 7, 0, 0).unwrap();
    let next = next_fire("0 0 8 * * *", at).unwrap().unwrap();
    assert_eq!(next, Local.with_ymd_and_hms(2026, 10, 8, 8, 0, 0).unwrap());
    let again = next_fire("0 0 8 * * *", next).unwrap().unwrap();
    assert_eq!(again, Local.with_ymd_and_hms(2026, 10, 9, 8, 0, 0).unwrap());
}

#[test]
fn timeout_and_approval_wait_are_capped() {
    let mut s = spec("caps");
    s.timeout_secs = 604_800;
    s.approval_wait_secs = 280;
    s.validate().unwrap();
    s.timeout_secs = 604_801;
    assert!(matches!(s.validate(), Err(Error::Invalid(_))));
    s.timeout_secs = 1800;
    s.approval_wait_secs = 281;
    assert!(matches!(s.validate(), Err(Error::Invalid(_))));
}

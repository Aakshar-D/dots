use dots_core::model::{RunStatus, TriggerKind};
use dots_core::util::{ct_eq, json_hash, new_id, new_token, now};
use serde_json::{json, Value};

#[test]
fn tokens_are_43_url_safe_chars_and_unique() {
    let a = new_token();
    let b = new_token();
    assert_eq!(a.len(), 43);
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert_ne!(a, b);
}

#[test]
fn ids_are_26_char_ulids() {
    assert_eq!(new_id().len(), 26);
}

#[test]
fn now_is_rfc3339_utc_millis() {
    let t = now();
    assert!(t.ends_with('Z'), "{t}");
    assert!(chrono::DateTime::parse_from_rfc3339(&t).is_ok());
}

#[test]
fn json_hash_ignores_key_order() {
    let a = json!({"a": 1, "b": {"x": 1, "y": [1, 2]}});
    let b: Value = serde_json::from_str(r#"{"b":{"y":[1,2],"x":1},"a":1}"#).unwrap();
    assert_eq!(json_hash(&a), json_hash(&b));
    assert_ne!(json_hash(&a), json_hash(&json!({"a": 2})));
    assert_eq!(json_hash(&a).len(), 64);
}

#[test]
fn ct_eq_compares_exactly() {
    assert!(ct_eq("abc", "abc"));
    assert!(!ct_eq("abc", "abd"));
    assert!(!ct_eq("abc", "abcd"));
    assert!(!ct_eq("", "a"));
}

#[test]
fn str_enum_round_trips() {
    assert_eq!(RunStatus::AwaitingApproval.as_str(), "awaiting_approval");
    assert_eq!(RunStatus::parse("awaiting_approval").unwrap(), RunStatus::AwaitingApproval);
    assert!(RunStatus::parse("nope").is_err());
    assert_eq!(serde_json::to_string(&TriggerKind::Webhook).unwrap(), "\"webhook\"");
    let t: TriggerKind = serde_json::from_str("\"resume\"").unwrap();
    assert_eq!(t, TriggerKind::Resume);
}

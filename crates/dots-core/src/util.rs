use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Sortable unique id (ULID, 26 chars).
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

/// 32 random bytes, base64url without padding (43 chars).
pub fn new_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Current time as RFC 3339 UTC with milliseconds, e.g. `2026-10-08T18:24:30.129Z`.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// SHA-256 hex of the JSON value with object keys sorted recursively.
pub fn json_hash(v: &Value) -> String {
    let s = serde_json::to_string(&canonical(v)).expect("serde_json::Value always serializes");
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonical(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// Constant-time string comparison for secrets.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

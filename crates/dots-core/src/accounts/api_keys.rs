//! Making API keys (accounts spec §10). A key is `dots_sk_` and 43 characters from `[0-9A-Za-z]`;
//! the person sees it once, and only its prefix and hash are kept.

use serde::Serialize;

use crate::store::ApiKeyRow;

/// What every API key starts with.
pub const API_KEY_PREFIX: &str = "dots_sk_";

const SECRET_CHARS: usize = 43;
const PREFIX_CHARS: usize = 12;
const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// 62 * 4: bytes at or above this are dropped, so `b % 62` has no bias.
const REJECT_FROM: u8 = 248;

/// A new key: each character is drawn from OS random bytes by rejection sampling (bytes of 248
/// or more are dropped), so it carries about 256 bits and no modulo bias.
pub fn new_api_key() -> String {
    let mut key = String::from(API_KEY_PREFIX);
    let mut chars = 0;
    while chars < SECRET_CHARS {
        let mut buf = [0u8; 64];
        getrandom::fill(&mut buf).expect("OS random number generator unavailable");
        for b in buf.into_iter().filter(|b| *b < REJECT_FROM) {
            if chars == SECRET_CHARS {
                break;
            }
            key.push(ALPHABET[usize::from(b % 62)] as char);
            chars += 1;
        }
    }
    key
}

/// The part of a key kept in the clear to recognise it by: its first 12 characters.
pub fn key_prefix(key: &str) -> String {
    key.chars().take(PREFIX_CHARS).collect()
}

/// Whether `text` has the shape of a key: the prefix and exactly 43 letters or digits.
pub fn looks_like_key(text: &str) -> bool {
    text.strip_prefix(API_KEY_PREFIX).is_some_and(|rest| {
        rest.len() == SECRET_CHARS && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// One of the keys in the API keys tab. Never carries the key text: only its prefix.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ApiKeyView {
    pub id: String,
    /// `personal` or the id of the org the key's workspace belongs to.
    pub scope: String,
    pub workspace_id: String,
    /// The workspace's name (a key whose workspace is not visible is not listed at all).
    pub workspace_name: String,
    pub name: String,
    pub prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    /// The core's words for why this PC may not create keys in that workspace; `None` is
    /// allowed. Revoking is always allowed.
    pub refusal: Option<String>,
}

impl ApiKeyView {
    pub(crate) fn new(row: ApiKeyRow, workspace_name: String, refusal: Option<String>) -> Self {
        ApiKeyView {
            id: row.id,
            scope: row.scope,
            workspace_id: row.workspace_id,
            workspace_name,
            name: row.name,
            prefix: row.prefix,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
            revoked_at: row.revoked_at,
            refusal,
        }
    }
}

/// The answer to making a key: its view, and its text, which is shown this once and never kept.
#[derive(Serialize, Clone, Debug)]
pub struct CreatedApiKey {
    pub key: ApiKeyView,
    pub secret: String,
}

//! Making API keys (accounts spec §10). A key is `dots_sk_` and 43 characters from `[0-9A-Za-z]`;
//! the person sees it once, and only its prefix and hash are kept.

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

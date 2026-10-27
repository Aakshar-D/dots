//! Publishing Dots from inside Dots (updates UI spec section 5): a GitHub client, the release
//! signing key, versions and notes, and the flows of the Releases page. No token or private key
//! ever leaves this module in an error or a log: GitHub's text passes through `scrub`.

pub mod flow;
pub mod github;
pub mod keys;
pub mod versions;

/// `text` with each non-empty secret replaced by `[hidden]`.
pub fn scrub(text: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|s| !s.is_empty())
        .fold(text.to_string(), |text, secret| {
            text.replace(secret, "[hidden]")
        })
}

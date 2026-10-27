//! Publishing Dots from inside Dots (updates UI spec section 5): a GitHub client, the release
//! signing key, versions and notes, and the flows of the Releases page. No token or private key
//! ever leaves this module in an error or a log: GitHub's text passes through `scrub`.

pub mod flow;
pub mod github;
pub mod keys;
pub mod versions;

/// `text` with each non-empty secret, and anything shaped like a GitHub token, replaced by
/// `[hidden]`.
pub fn scrub(text: &str, secrets: &[&str]) -> String {
    static TOKEN_LIKE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}").unwrap()
    });
    let text = secrets
        .iter()
        .filter(|s| !s.is_empty())
        .fold(text.to_string(), |text, secret| {
            text.replace(secret, "[hidden]")
        });
    TOKEN_LIKE.replace_all(&text, "[hidden]").into_owned()
}

//! Scrubbing secret values from what Dots keeps (env vault spec §10): run events, approvals, a
//! run's payload, summary and error, the messages of dots, and the log. Every secret Dots reads
//! is added here; a text that holds one, in any common spelling, gets `[secret:NAME]` in its
//! place. A backstop: resolution keeps values out of places that never need them.

use std::borrow::Cow;
use std::io::Write;
use std::sync::{LazyLock, RwLock};

use base64::Engine as _;
use serde_json::Value;

/// Values shorter than this are not scrubbed: too likely to match ordinary text.
pub const MIN_REDACT_BYTES: usize = 8;

/// The secret values to scrub, each spelling with the name it stands for, longest first.
#[derive(Default)]
pub struct Redactor {
    spellings: RwLock<Vec<(String, String)>>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.spellings.read().map(|s| s.len()).unwrap_or(0);
        f.debug_struct("Redactor")
            .field("spellings", &count)
            .finish()
    }
}

static GLOBAL: LazyLock<Redactor> = LazyLock::new(Redactor::default);

/// The process's redactor: the vault adds every secret it reads, and the store and the log
/// scrub with it.
pub fn global() -> &'static Redactor {
    &GLOBAL
}

/// `value` percent-encoded, every byte but `A-Za-z0-9-._~`; `plus` writes a space as `+`.
fn url_encoded(value: &str, plus: bool, lower: bool) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b' ' if plus => out.push('+'),
            _ if lower => out.push_str(&format!("%{b:02x}")),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The spellings of `value` that text may carry: as it is, URL-encoded (`%20` and `+` for a
/// space, upper and lower case hex), inside a JSON string, and in standard and URL-safe base64.
pub fn spellings(value: &str) -> Vec<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut out = vec![
        value.to_string(),
        url_encoded(value, false, false),
        url_encoded(value, true, false),
        url_encoded(value, false, true),
        json[1..json.len().saturating_sub(1).max(1)].to_string(),
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
    ];
    out.retain(|s| !s.is_empty());
    out.sort();
    out.dedup();
    out
}

impl Redactor {
    /// Scrubs `value` as the secret `name` from now on. A value shorter than
    /// `MIN_REDACT_BYTES` is left alone.
    pub fn add(&self, name: &str, value: &str) {
        if value.len() < MIN_REDACT_BYTES {
            return;
        }
        let mut all = self.spellings.write().unwrap();
        for spelling in spellings(value) {
            if !all.iter().any(|(s, _)| *s == spelling) {
                all.push((spelling, name.to_string()));
            }
        }
        all.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    }

    /// `text` with every spelling of every secret replaced by `[secret:NAME]`, longest first.
    pub fn scrub<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let all = self.spellings.read().unwrap();
        if !all.iter().any(|(s, _)| text.contains(s.as_str())) {
            return Cow::Borrowed(text);
        }
        let mut out = text.to_string();
        for (spelling, name) in all.iter() {
            if out.contains(spelling.as_str()) {
                out = out.replace(spelling.as_str(), &format!("[secret:{name}]"));
            }
        }
        Cow::Owned(out)
    }

    /// `value` with every string in it scrubbed, object keys included.
    pub fn scrub_json(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.scrub(s).into_owned()),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.scrub_json(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (self.scrub(k).into_owned(), self.scrub_json(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// A writer that scrubs each write with the process's redactor: the log's. Each write is one
/// line of the logger, so a value is never split across two.
pub struct RedactWriter<W> {
    inner: W,
}

impl<W> RedactWriter<W> {
    pub fn new(inner: W) -> RedactWriter<W> {
        RedactWriter { inner }
    }
}

impl<W: Write> Write for RedactWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match std::str::from_utf8(buf) {
            Ok(text) => self.inner.write_all(global().scrub(text).as_bytes())?,
            Err(_) => self
                .inner
                .write_all(global().scrub(&String::from_utf8_lossy(buf)).as_bytes())?,
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

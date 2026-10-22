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
/// What `try_scrub` gives in place of a text it cannot scrub without waiting.
pub const WITHHELD: &str = "(message withheld: redactor busy)";

/// The secret values to scrub, each spelling with the name it stands for, longest first.
#[derive(Default)]
pub struct Redactor {
    spellings: RwLock<Vec<(String, String)>>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.spellings.read().map_or(0, |s| s.len());
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

/// The base64 text that stays the same wherever `value` sits in a longer encoded text: a value
/// at offset 1 or 2 of a three-byte group (`base64("user:" + token)`), or followed by more
/// data (`echo $T | base64` adds a newline), encodes differently at its edges. For each of the
/// three offsets, the characters that lie wholly inside the value, in both alphabets. A core
/// shorter than `MIN_REDACT_BYTES` characters is left out.
fn base64_cores(value: &str) -> Vec<String> {
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    let n = value.len();
    let mut out = Vec::new();
    for offset in 0..3usize {
        let mut bytes = vec![0u8; offset];
        bytes.extend_from_slice(value.as_bytes());
        // Character j holds bits 6j..6j+6; the value holds bits 8*offset..8*(offset+n).
        let first = (8 * offset).div_ceil(6);
        let end = 8 * (offset + n) / 6;
        for engine in [&STANDARD_NO_PAD, &URL_SAFE_NO_PAD] {
            let encoded = engine.encode(&bytes);
            if end > first && end - first >= MIN_REDACT_BYTES {
                out.push(encoded[first..end].to_string());
            }
        }
    }
    out
}

/// The spellings of `value` that text may carry: as it is, URL-encoded (`%20` and `+` for a
/// space, upper and lower case hex), inside a JSON string, and in standard and URL-safe base64.
pub fn spellings(value: &str) -> Vec<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut out = base64_cores(value);
    out.extend([
        value.to_string(),
        url_encoded(value, false, false),
        url_encoded(value, true, false),
        url_encoded(value, false, true),
        json[1..json.len().saturating_sub(1).max(1)].to_string(),
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
    ]);
    out.retain(|s| !s.is_empty());
    out.sort();
    out.dedup();
    out
}

impl Redactor {
    /// The lock, whether or not a panic poisoned it: the list is only ever appended to whole.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Vec<(String, String)>> {
        self.spellings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Vec<(String, String)>> {
        self.spellings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The length of the longest spelling registered: how many bytes of a secret a cut can
    /// leave behind. A text is scrubbed before it is cut, and a buffer that keeps only a tail
    /// keeps this much more, so that a secret across the cut is whole when it is scrubbed.
    pub fn longest(&self) -> usize {
        self.read().first().map_or(0, |(s, _)| s.len())
    }

    /// Like `scrub`, for a caller that must not block (a panic hook). When the list is being
    /// written, the text is withheld: `WITHHELD` takes its place.
    pub fn try_scrub<'a>(&self, text: &'a str) -> Cow<'a, str> {
        match self.spellings.try_read() {
            Ok(all) => Self::scrub_with(&all, text),
            Err(std::sync::TryLockError::Poisoned(p)) => Self::scrub_with(&p.into_inner(), text),
            Err(std::sync::TryLockError::WouldBlock) => Cow::Borrowed(WITHHELD),
        }
    }

    /// Scrubs `text`, the end of a stream whose head was cut off, and drops the first bytes
    /// where a secret cut in two may have left its end.
    pub fn scrub_clipped_head(&self, text: &str) -> String {
        let scrubbed = self.scrub(text);
        let mut start = self.longest().saturating_sub(1).min(scrubbed.len());
        while !scrubbed.is_char_boundary(start) {
            start += 1;
        }
        scrubbed[start..].to_string()
    }

    /// Scrubs `text`, the start of a stream cut off after it, and drops the last bytes where a
    /// secret cut in two may have left its start.
    pub fn scrub_clipped_tail(&self, text: &str) -> String {
        let scrubbed = self.scrub(text);
        let mut end = scrubbed
            .len()
            .saturating_sub(self.longest().saturating_sub(1));
        while !scrubbed.is_char_boundary(end) {
            end -= 1;
        }
        scrubbed[..end].to_string()
    }

    /// Scrubs `value` as the secret `name` from now on. A value shorter than
    /// `MIN_REDACT_BYTES` is left alone.
    pub fn add(&self, name: &str, value: &str) {
        if value.len() < MIN_REDACT_BYTES {
            return;
        }
        let mut all = self.write();
        for spelling in spellings(value) {
            if !all.iter().any(|(s, _)| *s == spelling) {
                all.push((spelling, name.to_string()));
            }
        }
        all.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    }

    /// `text` with every spelling of every secret replaced by `[secret:NAME]`, longest first.
    pub fn scrub<'a>(&self, text: &'a str) -> Cow<'a, str> {
        Self::scrub_with(&self.read(), text)
    }

    fn scrub_with<'a>(all: &[(String, String)], text: &'a str) -> Cow<'a, str> {
        if !all.iter().any(|(s, _)| text.contains(s.as_str())) {
            return Cow::Borrowed(text);
        }
        let mut out = text.to_string();
        for (spelling, name) in all {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a_poisoned_lock_does_not_stop_the_scrub() {
        let r = Arc::new(Redactor::default());
        r.add("POISON", "poison-value-12345");
        let r2 = r.clone();
        let _ = std::thread::spawn(move || {
            let _held = r2.spellings.write().unwrap();
            panic!("poisons the lock");
        })
        .join();
        r.add("AFTER", "after-poison-value-1");
        assert_eq!(r.scrub("poison-value-12345").as_ref(), "[secret:POISON]");
        assert_eq!(
            r.try_scrub("after-poison-value-1").as_ref(),
            "[secret:AFTER]"
        );
        assert!(r.longest() > 0);
    }

    #[test]
    fn try_scrub_never_waits_for_a_writer_and_never_shows_the_text() {
        let r = Redactor::default();
        r.add("HELD", "held-value-12345");
        let _writer = r.spellings.write().unwrap();
        assert_eq!(
            r.try_scrub("held-value-12345").as_ref(),
            "(message withheld: redactor busy)"
        );
    }
}

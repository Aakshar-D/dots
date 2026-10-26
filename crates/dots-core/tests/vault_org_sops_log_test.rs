//! What a SOPS fetch logs (env vault spec §6.2): a skipped name is logged by its length only,
//! since a value pasted into a name's place must not reach the logs. Alone in its binary: the
//! logs are read through a subscriber set for this thread, and a test on another thread that
//! reached the same log line first could leave it switched off for this one.

use std::path::Path;
use std::sync::{Arc, Mutex};

use dots_core::vault::org::{http_client, Sops, SopsSettings, Source, SourceCreds};
use zeroize::Zeroizing;

/// What the code under test logs, as text, while the guard lives on this thread.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn capture(&self) -> tracing::subscriber::DefaultGuard {
        let writer = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn fixture_text(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sops")
        .join(name);
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// The test identity, which the fixtures are encrypted to.
fn mine() -> SourceCreds {
    let text = fixture_text("test-identity.txt");
    let line = text
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-1"))
        .expect("an identity line");
    SourceCreds::Age {
        identity: Zeroizing::new(line.to_string()),
    }
}

#[tokio::test]
async fn an_empty_value_is_logged_by_its_names_length_only() {
    // An empty text is never encrypted, so a file may carry one that its MAC does not cover.
    let text = fixture_text("mac-only.enc.json").replacen(
        "\"env\": {\n",
        "\"env\": {\n\t\t\"EVIL_unencrypted\": \"\",\n",
        1,
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("org.sops.json");
    std::fs::write(&path, text).unwrap();
    let source = Sops::new(
        SopsSettings {
            location: path.display().to_string(),
        },
        http_client(),
    );

    let logs = Logs::default();
    let guard = logs.capture();
    let fetched = source.fetch(&mine()).await.unwrap();
    drop(guard);

    assert!(!fetched.entries.keys().any(|name| name.contains("EVIL")));
    let logged = logs.text();
    assert!(
        logged.contains("an empty value under a name 16 characters long, skipped"),
        "{logged}"
    );
    assert!(!logged.contains("EVIL"), "{logged}");
}

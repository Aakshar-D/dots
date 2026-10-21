//! The test stand-in for Microsoft Edge, `fake-browser` (`src/bin/fake-browser.rs`): its path,
//! for `Settings.edge_path` or `EdgeHost::open`; what each start on a dot's profile recorded;
//! and its controls. Every fake running on a profile under `root` is told to quit when this is
//! dropped, so a failed test leaves no Edge behind.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

/// The program `fake-browser`.
pub fn exe() -> String {
    env!("CARGO_BIN_EXE_fake-browser").to_string()
}

/// The fakes on the profiles under `root` (`<data_dir>/browser`).
pub struct FakeEdges {
    pub root: PathBuf,
}

impl FakeEdges {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn profile(&self, dot_id: &str) -> PathBuf {
        self.root.join(dot_id).join("profile")
    }

    /// Every start on the dot's profile, oldest first: `{"args": [...], "port": n, "pid": n}`.
    pub fn starts(&self, dot_id: &str) -> Vec<Value> {
        match std::fs::read_to_string(self.profile(dot_id).join("fake-browser.jsonl")) {
            Ok(text) => text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The port of the newest start.
    pub fn port(&self, dot_id: &str) -> Option<u16> {
        self.starts(dot_id)
            .last()
            .and_then(|start| start["port"].as_u64())
            .map(|port| port as u16)
    }

    /// Whether a fake holds the dot's profile.
    pub fn running(&self, dot_id: &str) -> bool {
        dots_core::browser::host::profile_in_use(&self.profile(dot_id))
    }

    /// The pages the next start opens with, most recent first.
    pub fn first_pages(&self, dot_id: &str, urls: &[&str]) {
        std::fs::create_dir_all(self.profile(dot_id)).unwrap();
        std::fs::write(
            self.profile(dot_id).join("fake-pages.json"),
            serde_json::to_string(urls).unwrap(),
        )
        .unwrap();
    }

    /// Makes `urls` the pages of the running fake, most recent first.
    pub fn set_pages(&self, dot_id: &str, urls: &[&str]) {
        let port = self.port(dot_id).expect("a started fake");
        assert!(
            post(port, "/fake/pages", &serde_json::to_string(urls).unwrap()),
            "the fake did not take its pages"
        );
    }

    /// Ends the running fake at once, as when the person closes the window.
    pub fn quit(&self, dot_id: &str) {
        if let Some(port) = self.port(dot_id) {
            post(port, "/fake/quit", "");
        }
    }

    /// Whether no fake holds the dot's profile within `limit`.
    pub async fn gone_within(&self, dot_id: &str, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while self.running(dot_id) {
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    }
}

impl Drop for FakeEdges {
    fn drop(&mut self) {
        let Ok(dots) = std::fs::read_dir(&self.root) else {
            return;
        };
        for dot in dots.flatten() {
            let id = dot.file_name().to_string_lossy().into_owned();
            if self.running(&id) {
                self.quit(&id);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && std::fs::read_dir(&self.root)
                .map(|dots| {
                    dots.flatten()
                        .any(|d| self.running(&d.file_name().to_string_lossy()))
                })
                .unwrap_or(false)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A plain HTTP POST to the fake on `port`. True when it answered 200.
fn post(port: u16, path: &str, body: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer.starts_with("HTTP/1.1 200")
}

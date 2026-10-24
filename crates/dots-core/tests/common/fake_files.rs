//! A fake web server for files, such as an org's SOPS file published over HTTPS: it answers
//! `GET <path>` with the file put there, a 404 for any other path, or the status a test sets.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tokio::task::JoinHandle;

#[derive(Default)]
struct Fake {
    files: HashMap<String, Bytes>,
    /// A status to answer with, and how many more requests get it.
    failing: Option<(u16, usize)>,
    /// The paths asked for, with their query, in order.
    requests: Vec<String>,
}

type Shared = Arc<Mutex<Fake>>;

/// The fake. It stops when dropped.
pub struct FakeFiles {
    address: String,
    state: Shared,
    task: JoinHandle<()>,
}

impl Drop for FakeFiles {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeFiles {
    /// Starts an empty fake on `127.0.0.1` and a free port.
    pub async fn start() -> FakeFiles {
        let state: Shared = Arc::default();
        let app = Router::new().fallback(handle).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        FakeFiles {
            address,
            state,
            task,
        }
    }

    /// The URL of `path`, which starts with `/`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.address)
    }

    /// Serves `body` at `path`.
    pub fn put(&self, path: &str, body: impl Into<Bytes>) {
        self.fake().files.insert(path.to_string(), body.into());
    }

    /// Answers the next `count` requests with `status` and a short text body.
    pub fn fail_next(&self, status: u16, count: usize) {
        self.fake().failing = Some((status, count));
    }

    /// The paths asked for, with their query, in order.
    pub fn requests(&self) -> Vec<String> {
        self.fake().requests.clone()
    }

    fn fake(&self) -> std::sync::MutexGuard<'_, Fake> {
        self.state.lock().unwrap()
    }
}

async fn handle(State(state): State<Shared>, uri: Uri) -> Response {
    let mut fake = state.lock().unwrap();
    let asked = uri
        .path_and_query()
        .map_or(uri.path().to_string(), |p| p.to_string());
    fake.requests.push(asked);
    if let Some((status, count)) = fake.failing {
        fake.failing = (count > 1).then_some((status, count - 1));
        let status = StatusCode::from_u16(status).unwrap();
        return (status, "the fake is failing on purpose").into_response();
    }
    match fake.files.get(uri.path()) {
        Some(body) => body.clone().into_response(),
        None => (StatusCode::NOT_FOUND, "404 page not found").into_response(),
    }
}

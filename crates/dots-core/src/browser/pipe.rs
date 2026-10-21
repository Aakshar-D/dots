//! A dot's Edge over its debugging pipe (browser spec section 14). Edge is started with
//! `--remote-debugging-pipe` and opens no port: Dots holds its only connection. Messages are
//! JSON, each ended by a NUL byte. Dots' own requests (the pages, closing) and the requests of
//! the run's Playwright, relayed from its door (`door.rs`), share the pipe: every request gets
//! an id of the pipe's own, and each answer goes back to its sender under the id the sender
//! gave. Events go to Playwright.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::host::{page_urls, HostError};

/// The longest message either side may send: a full-page screenshot is a few MB.
pub const MAX_MESSAGE: usize = 256 * 1024 * 1024;
/// How long Dots waits for Edge to answer one of its own requests.
pub const ASK_WAIT: Duration = Duration::from_secs(5);

/// NUL-ended messages, put together from the pieces a pipe delivers.
#[derive(Debug)]
pub struct Frames {
    buf: Vec<u8>,
    max: usize,
    /// The message being read is longer than `max`: it is dropped up to its NUL.
    skipping: bool,
}

impl Default for Frames {
    fn default() -> Self {
        Frames::with_max(MAX_MESSAGE)
    }
}

impl Frames {
    /// Frames whose messages may be `max` bytes long at most.
    pub fn with_max(max: usize) -> Frames {
        Frames {
            buf: Vec::new(),
            max,
            skipping: false,
        }
    }

    /// Takes the next bytes read; returns every message they complete, in order. An empty
    /// message is skipped, and one longer than the most allowed is dropped.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut done = Vec::new();
        for part in bytes.split_inclusive(|b| *b == 0) {
            let ended = part.last() == Some(&0);
            let body = if ended { &part[..part.len() - 1] } else { part };
            if !self.skipping {
                if self.buf.len() + body.len() > self.max {
                    tracing::warn!("dropped a browser message longer than {} bytes", self.max);
                    self.buf = Vec::new();
                    self.skipping = true;
                } else {
                    self.buf.extend_from_slice(body);
                }
            }
            if ended {
                if !self.skipping && !self.buf.is_empty() {
                    done.push(std::mem::take(&mut self.buf));
                }
                self.skipping = false;
            }
        }
        done
    }
}

/// The run's Playwright, connected through a door.
struct Client {
    serial: u64,
    to: mpsc::UnboundedSender<String>,
    /// The pipe's id of each of its requests in flight: the id it gave, and the method.
    ids: HashMap<u64, (Value, String)>,
    /// The sessions Edge attached for it at the top level (`Target.attachedToTarget`).
    sessions: BTreeSet<String>,
    /// The browser contexts it made (`Target.createBrowserContext`).
    contexts: BTreeSet<String>,
}

#[derive(Default)]
struct Waiting {
    /// The pipe ended: nothing more is answered.
    ended: bool,
    own: HashMap<u64, oneshot::Sender<Value>>,
    client: Option<Client>,
    /// The requests in flight of Playwrights that went, by the pipe's id: their methods.
    orphans: HashMap<u64, String>,
}

/// The connection to one Edge.
pub struct Pipe {
    to_edge: mpsc::UnboundedSender<Vec<u8>>,
    next: AtomicU64,
    waiting: Mutex<Waiting>,
    gone: CancellationToken,
}

impl std::fmt::Debug for Pipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipe")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl Pipe {
    /// The pipe read from `from_edge` and written to `to_edge` (the helper's stdout and
    /// stdin). It ends when either does; Edge's input closes once the last `Arc` of the pipe
    /// is dropped.
    pub fn new<R, W>(from_edge: R, to_edge: W) -> Arc<Pipe>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let pipe = Arc::new(Pipe {
            to_edge: tx,
            next: AtomicU64::new(1),
            waiting: Mutex::new(Waiting::default()),
            gone: CancellationToken::new(),
        });
        tokio::spawn(write(rx, to_edge, Arc::downgrade(&pipe)));
        tokio::spawn(read(from_edge, Arc::downgrade(&pipe)));
        pipe
    }

    /// Whether Edge's end of the pipe has closed (Edge ended, or the helper did).
    pub fn is_closed(&self) -> bool {
        self.gone.is_cancelled()
    }

    /// Waits until the pipe has closed.
    pub async fn closed(&self) {
        self.gone.cancelled().await
    }

    fn id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn send(&self, message: &Value) {
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        let _ = self.to_edge.send(bytes);
    }

    /// Sends Edge the request `method` of Dots' own and returns its `result`, waiting `ASK_WAIT`.
    pub async fn ask(&self, method: &str, params: Value) -> Result<Value, HostError> {
        self.ask_within(method, params, ASK_WAIT).await
    }

    /// `ask`, waiting up to `wait`.
    pub async fn ask_within(
        &self,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> Result<Value, HostError> {
        let id = self.id();
        let (tx, rx) = oneshot::channel();
        {
            let mut waiting = self.waiting.lock().unwrap();
            if waiting.ended {
                return Err(HostError::Unreachable("Edge has closed".to_string()));
            }
            waiting.own.insert(id, tx);
            self.send(&json!({ "id": id, "method": method, "params": params }));
        }
        let answer = tokio::time::timeout(wait, rx).await;
        self.waiting.lock().unwrap().own.remove(&id);
        match answer {
            Ok(Ok(answer)) => match answer.get("error") {
                Some(error) => Err(HostError::Unreachable(format!("{method}: {error}"))),
                None => Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
            },
            Ok(Err(_)) => Err(HostError::Unreachable("Edge has closed".to_string())),
            Err(_) => Err(HostError::Unreachable(format!(
                "no answer to {method} within {} s",
                wait.as_secs()
            ))),
        }
    }

    /// The address of every page of this Edge (`Target.getTargets`, type `page`).
    pub async fn pages(&self) -> Result<Vec<String>, HostError> {
        let result = self.ask("Target.getTargets", json!({})).await?;
        let targets = result
            .get("targetInfos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(page_urls(&targets))
    }

    /// Connects Playwright: what Edge sends it comes out of the receiver, under the ids it
    /// gave. A Playwright connected before is let go first (`disconnect`). `None` once the
    /// pipe has ended.
    pub fn connect(&self) -> Option<(u64, mpsc::UnboundedReceiver<String>)> {
        let mut waiting = self.waiting.lock().unwrap();
        if waiting.ended {
            return None;
        }
        if let Some(old) = waiting.client.take() {
            self.forget(old, &mut waiting);
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let serial = self.id();
        waiting.client = Some(Client {
            serial,
            to: tx,
            ids: HashMap::new(),
            sessions: BTreeSet::new(),
            contexts: BTreeSet::new(),
        });
        Some((serial, rx))
    }

    /// A message from the Playwright connected as `serial`: sent to Edge under an id of the
    /// pipe's own. A message that is not a JSON object with a number `id`, or from a
    /// Playwright no longer connected, is dropped: false.
    pub fn relay(&self, serial: u64, text: &str) -> bool {
        let mut message = match serde_json::from_str::<Value>(text) {
            Ok(message) => message,
            Err(e) => {
                tracing::warn!("dropped a browser message from Playwright that is not JSON: {e}");
                return false;
            }
        };
        let Some(theirs) = message.get("id").filter(|id| id.is_number()).cloned() else {
            tracing::warn!("dropped a browser message from Playwright without an id");
            return false;
        };
        let mut waiting = self.waiting.lock().unwrap();
        let Some(client) = waiting.client.as_mut().filter(|c| c.serial == serial) else {
            return false;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = self.id();
        client.ids.insert(id, (theirs, method));
        message["id"] = json!(id);
        self.send(&message);
        true
    }

    /// The Playwright connected as `serial` went: the sessions Edge attached for it are
    /// detached and Edge stops attaching new ones, so that the next Playwright finds every
    /// page again.
    pub fn disconnect(&self, serial: u64) {
        let mut waiting = self.waiting.lock().unwrap();
        if waiting.client.as_ref().is_some_and(|c| c.serial == serial) {
            let old = waiting.client.take().expect("a client");
            self.forget(old, &mut waiting);
        }
    }

    /// Sends Edge a request of Dots' own whose answer is not waited for.
    fn tell(&self, method: &str, params: Value) {
        self.send(&json!({ "id": self.id(), "method": method, "params": params }));
    }

    /// Undoes what a Playwright set up on the pipe: over a WebSocket, Edge would have undone it
    /// when the connection ended, but the pipe is one connection for Edge's whole life. Its
    /// requests still in flight become orphans, whose answers are undone as they come
    /// (`undo_answer`). The answers are not waited for.
    fn forget(&self, client: Client, waiting: &mut Waiting) {
        for session in client.sessions {
            self.tell("Target.detachFromTarget", json!({ "sessionId": session }));
        }
        for context in client.contexts {
            self.tell(
                "Target.disposeBrowserContext",
                json!({ "browserContextId": context }),
            );
        }
        self.tell(
            "Target.setAutoAttach",
            json!({ "autoAttach": false, "waitForDebuggerOnStart": false }),
        );
        self.tell("Target.setDiscoverTargets", json!({ "discover": false }));
        // Downloads go where the profile's preferences say, not to the server's own folder.
        self.tell(
            "Browser.setDownloadBehavior",
            json!({ "behavior": "default" }),
        );
        waiting
            .orphans
            .extend(client.ids.into_iter().map(|(id, (_, method))| (id, method)));
    }

    /// The answer to a request of a Playwright that went: a session it attached is detached,
    /// and a context it made is disposed.
    fn undo_answer(&self, method: &str, answer: &Value) {
        if let Some(session) = answer.pointer("/result/sessionId").and_then(Value::as_str) {
            self.tell("Target.detachFromTarget", json!({ "sessionId": session }));
        }
        if method == "Target.createBrowserContext" {
            if let Some(context) = answer
                .pointer("/result/browserContextId")
                .and_then(Value::as_str)
            {
                self.tell(
                    "Target.disposeBrowserContext",
                    json!({ "browserContextId": context }),
                );
            }
        }
    }

    /// One message from Edge: an answer to Dots or to Playwright, or an event for Playwright.
    fn dispatch(&self, frame: &[u8]) {
        let mut message = match serde_json::from_slice::<Value>(frame) {
            Ok(message @ Value::Object(_)) => message,
            _ => {
                tracing::warn!("dropped a browser message from Edge that is not a JSON object");
                return;
            }
        };
        let mut waiting = self.waiting.lock().unwrap();
        if let Some(id) = message.get("id") {
            let id = id.as_u64();
            if let Some(own) = id.and_then(|id| waiting.own.remove(&id)) {
                let _ = own.send(message);
                return;
            }
            if let Some(method) = id.and_then(|id| waiting.orphans.remove(&id)) {
                self.undo_answer(&method, &message);
                return;
            }
            let Some(client) = waiting.client.as_mut() else {
                return;
            };
            let Some((theirs, method)) = id.and_then(|id| client.ids.remove(&id)) else {
                return;
            };
            if method == "Target.createBrowserContext" {
                if let Some(context) = message
                    .pointer("/result/browserContextId")
                    .and_then(Value::as_str)
                {
                    client.contexts.insert(context.to_string());
                }
            }
            message["id"] = theirs;
            let _ = client.to.send(message.to_string());
            return;
        }
        let Some(client) = waiting.client.as_mut() else {
            // Nobody is connected: a page Edge attached anyway (auto-attach was still on, and
            // the page may be paused waiting for a debugger) is detached at once.
            let top_level = message.get("sessionId").is_none();
            let method = message.get("method").and_then(Value::as_str);
            if let (true, Some("Target.attachedToTarget"), Some(session)) = (
                top_level,
                method,
                message.pointer("/params/sessionId").and_then(Value::as_str),
            ) {
                self.tell("Target.detachFromTarget", json!({ "sessionId": session }));
            }
            return;
        };
        if message.get("sessionId").is_none() {
            let method = message.get("method").and_then(Value::as_str);
            let session = message
                .pointer("/params/sessionId")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (method, session) {
                (Some("Target.attachedToTarget"), Some(session)) => {
                    client.sessions.insert(session);
                }
                (Some("Target.detachedFromTarget"), Some(session)) => {
                    client.sessions.remove(&session);
                }
                _ => {}
            }
        }
        let _ = client.to.send(String::from_utf8_lossy(frame).into_owned());
    }

    /// The pipe ended: every request waiting fails, and Playwright's connection ends.
    fn end(&self) {
        self.gone.cancel();
        let mut waiting = self.waiting.lock().unwrap();
        waiting.ended = true;
        waiting.own.clear();
        waiting.client = None;
        waiting.orphans.clear();
    }
}

async fn read<R: AsyncRead + Unpin>(mut from_edge: R, pipe: Weak<Pipe>) {
    let mut frames = Frames::default();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match from_edge.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let Some(pipe) = pipe.upgrade() else {
            return;
        };
        for frame in frames.push(&buf[..n]) {
            pipe.dispatch(&frame);
        }
    }
    if let Some(pipe) = pipe.upgrade() {
        pipe.end();
    }
}

async fn write<W: AsyncWrite + Unpin>(
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut to_edge: W,
    pipe: Weak<Pipe>,
) {
    while let Some(bytes) = rx.recv().await {
        if to_edge.write_all(&bytes).await.is_err() || to_edge.flush().await.is_err() {
            if let Some(pipe) = pipe.upgrade() {
                pipe.end();
            }
            return;
        }
    }
    // Every sender is gone: Edge's input closes, and the helper closes Edge.
    let _ = to_edge.shutdown().await;
}

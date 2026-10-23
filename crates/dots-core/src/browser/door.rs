//! The door through which a run's Playwright reaches its dot's Edge (browser spec section 14):
//! `ws://127.0.0.1:<dots port>/cdp/<secret>` on Dots' own server. Each Playwright server gets a
//! fresh secret when it starts. The first connection with it is relayed to the Edge's pipe
//! (`pipe.rs`); every other one (a second connection, a wrong or a spent secret) is refused
//! with 404 and logged.

use std::sync::{Arc, Mutex};

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;

use super::pipe::{Pipe, MAX_MESSAGE};
use crate::util::ct_eq;

/// A new secret: 128 random bits, base64url without padding (22 characters).
pub fn new_secret() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

enum Door {
    /// No one has come in yet.
    Waiting(Arc<Pipe>),
    /// Playwright is connected.
    Open,
    /// The connection was made and has ended, or the upgrade failed.
    Spent,
}

/// The open doors, by secret.
#[derive(Default)]
pub struct Doors {
    doors: Mutex<Vec<(String, Door)>>,
}

impl Doors {
    /// A new door to `pipe`; returns its secret.
    pub fn open(&self, pipe: Arc<Pipe>) -> String {
        let secret = new_secret();
        self.doors
            .lock()
            .unwrap()
            .push((secret.clone(), Door::Waiting(pipe)));
        secret
    }

    /// Whether the door of `secret` was used and its connection has ended: the Playwright
    /// server behind it cannot come in again.
    pub fn spent(&self, secret: &str) -> bool {
        self.doors
            .lock()
            .unwrap()
            .iter()
            .any(|(s, door)| ct_eq(s, secret) && matches!(door, Door::Spent))
    }

    /// Forgets the door of `secret` (its Playwright server stopped).
    pub fn shut(&self, secret: &str) {
        self.doors
            .lock()
            .unwrap()
            .retain(|(s, _)| !ct_eq(s, secret));
    }

    /// The pipe behind the door of `secret`, once: the door is open from then on.
    fn enter(&self, secret: &str) -> Option<Arc<Pipe>> {
        let mut doors = self.doors.lock().unwrap();
        let (_, door) = doors.iter_mut().find(|(s, _)| ct_eq(s, secret))?;
        match std::mem::replace(door, Door::Open) {
            Door::Waiting(pipe) => Some(pipe),
            other => {
                *door = other;
                None
            }
        }
    }

    /// The connection through the door of `secret` ended.
    fn leave(&self, secret: &str) {
        if let Some((_, door)) = self
            .doors
            .lock()
            .unwrap()
            .iter_mut()
            .find(|(s, _)| ct_eq(s, secret))
        {
            *door = Door::Spent;
        }
    }
}

/// `GET /cdp/<secret>`: Playwright's WebSocket to the Edge behind the door of `secret`, which
/// is spent by it. Anything else is 404.
pub async fn connect(
    doors: Arc<Doors>,
    secret: String,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some(pipe) = doors.enter(&secret) else {
        tracing::warn!("refused a connection to a dot's browser: its secret is wrong or spent");
        return StatusCode::NOT_FOUND.into_response();
    };
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => {
            tracing::warn!("refused a connection to a dot's browser: {rejection}");
            doors.leave(&secret);
            return rejection.into_response();
        }
    };
    let failed = (doors.clone(), secret.clone());
    upgrade
        .max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .on_failed_upgrade(move |e| {
            tracing::warn!("a connection to a dot's browser failed: {e}");
            failed.0.leave(&failed.1);
        })
        .on_upgrade(move |socket| async move {
            relay(&pipe, socket).await;
            doors.leave(&secret);
        })
}

/// Relays between Playwright's WebSocket and the Edge's pipe until either ends.
async fn relay(pipe: &Pipe, mut socket: WebSocket) {
    let Some((serial, mut from_edge)) = pipe.connect() else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    pipe.relay(serial, text.as_str());
                }
                Some(Ok(Message::Binary(bytes))) => match std::str::from_utf8(&bytes) {
                    Ok(text) => {
                        pipe.relay(serial, text);
                    }
                    Err(_) => tracing::warn!("dropped a browser message from Playwright that is not UTF-8"),
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
            },
            outgoing = from_edge.recv() => match outgoing {
                Some(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // Edge closed, or another Playwright took the pipe.
                None => {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
            },
        }
    }
    pipe.disconnect(serial);
}

//! An Edge played by the test over an in-memory pipe (`browser::pipe::Pipe`): each message Dots
//! sent, in order, and whatever the test sends back, as messages or as raw bytes.

use std::sync::Arc;
use std::time::Duration;

use dots_core::browser::pipe::{Frames, Pipe};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, WriteHalf};
use tokio::sync::mpsc;

pub struct TestEdge {
    pub pipe: Arc<Pipe>,
    from_dots: mpsc::UnboundedReceiver<Value>,
    to_dots: WriteHalf<DuplexStream>,
}

impl TestEdge {
    pub fn start() -> TestEdge {
        let (dots_side, edge_side) = tokio::io::duplex(64 * 1024);
        let (from_edge, to_edge) = tokio::io::split(dots_side);
        let pipe = Pipe::new(from_edge, to_edge);
        let (mut edge_in, to_dots) = tokio::io::split(edge_side);
        let (tx, from_dots) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut frames = Frames::default();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match edge_in.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for frame in frames.push(&buf[..n]) {
                    let _ = tx.send(serde_json::from_slice(&frame).expect("Dots sends JSON"));
                }
            }
        });
        TestEdge {
            pipe,
            from_dots,
            to_dots,
        }
    }

    /// The next message Dots sent; fails after 5 s.
    pub async fn next(&mut self) -> Value {
        self.next_within(Duration::from_secs(5))
            .await
            .expect("a message from Dots within 5 s")
    }

    /// The next message Dots sends within `wait`, if any.
    pub async fn next_within(&mut self, wait: Duration) -> Option<Value> {
        tokio::time::timeout(wait, self.from_dots.recv())
            .await
            .ok()
            .flatten()
    }

    /// Bytes as Edge writes them: a message may come in pieces, or several at once.
    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.to_dots.write_all(bytes).await.unwrap();
        self.to_dots.flush().await.unwrap();
    }

    /// One message, ended by its NUL byte.
    pub async fn send(&mut self, message: &Value) {
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        self.send_raw(&bytes).await;
    }

    /// Edge ends: its end of the pipe closes.
    pub async fn close(&mut self) {
        let _ = self.to_dots.shutdown().await;
    }
}

use std::future::Future;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::approvals::ApprovalHub;
use crate::runner::Runner;
use crate::store::Store;
use crate::Result;

mod browser;
pub mod gateway;
pub mod mcp;
pub mod webhook;

pub const MAX_WEBHOOK_BODY: usize = 256 * 1024;
pub const MAX_MCP_BODY: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct ServerState {
    pub store: Store,
    pub runner: Arc<Runner>,
    pub hub: Arc<ApprovalHub>,
}

pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/dots/{id}/trigger",
            post(webhook::trigger).layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY)),
        )
        // Tool inputs (e.g. a Write of a large file) pass through the permission gate.
        .route(
            "/mcp",
            post(mcp::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
        )
        // The gateway to a run's MCP server: Dots checks each call (MCP/skills spec §5).
        .route(
            "/mcp/s/{server}",
            post(gateway::handle)
                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                .layer(DefaultBodyLimit::max(MAX_MCP_BODY)),
        )
        .layer(middleware::from_fn(loopback_only))
        .with_state(state)
}

async fn loopback_only(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if !addr.ip().is_loopback() {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

pub async fn bind(port: u16) -> Result<TcpListener> {
    Ok(TcpListener::bind(("127.0.0.1", port)).await?)
}

/// How long a start keeps trying a port that is in use, unless `Config::bind_wait` says
/// otherwise.
pub const BIND_WAIT: Duration = Duration::from_secs(2);
const BIND_RETRY_EVERY: Duration = Duration::from_millis(50);

/// `bind`, tried again every 50 ms for up to `wait` while the port is in use. "Restart Dots"
/// starts the new process while the old one may still hold the socket for a moment; without
/// the wait that start would come up without its server. A port that is free costs one attempt.
pub async fn bind_retrying(port: u16, wait: Duration) -> std::io::Result<TcpListener> {
    retry_while_in_use(wait, || TcpListener::bind(("127.0.0.1", port))).await
}

/// Runs `attempt` until it answers anything but "address in use", or until `wait` has passed
/// since the first attempt: then the last answer is returned. Every other error is returned at
/// once, since waiting does not change it.
pub async fn retry_while_in_use<T, F, Fut>(wait: Duration, mut attempt: F) -> std::io::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let give_up = Instant::now() + wait;
    loop {
        match attempt().await {
            Err(e) if e.kind() == ErrorKind::AddrInUse && Instant::now() < give_up => {
                tokio::time::sleep(BIND_RETRY_EVERY).await;
            }
            answer => return answer,
        }
    }
}

/// The error of a Claude run while the local server is down (spec §8): such a run needs the
/// server's MCP endpoint. Says "in use" only when that is what the bind answered.
pub fn no_server_text(port: u16, bind_error: &std::io::Error) -> String {
    if bind_error.kind() == ErrorKind::AddrInUse {
        format!(
            "local server is not running: port {port} is in use. Change the port in Settings \
             and restart Dots."
        )
    } else {
        format!(
            "local server is not running: could not listen on port {port}: {bind_error}. Try \
             another port in Settings and restart Dots."
        )
    }
}

pub fn serve(
    listener: TcpListener,
    state: ServerState,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
        {
            tracing::error!("http server stopped: {e}");
        }
    })
}

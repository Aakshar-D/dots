use std::future::Future;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
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
pub mod v1;
pub mod webhook;

pub const MAX_WEBHOOK_BODY: usize = 256 * 1024;
pub const MAX_MCP_BODY: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct ServerState {
    pub store: Store,
    pub runner: Arc<Runner>,
    pub hub: Arc<ApprovalHub>,
}

/// The routes, behind the loopback check. The Host check needs the bound port and is applied
/// in `serve`, so a test of it must go through `serve`.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/dots/{id}/trigger",
            post(webhook::trigger).layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY)),
        )
        // The API of a workspace's keys (accounts spec §10).
        .route("/v1/dots", get(v1::list_dots))
        .route(
            "/v1/dots/{id}/trigger",
            post(v1::trigger).layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY)),
        )
        .route("/v1/runs/{id}", get(v1::run))
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
        // The door of a run's Playwright to its dot's Edge (browser spec section 14).
        .route("/cdp/{secret}", get(browser::door))
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

/// Whether a `Host` header names this server: exactly `127.0.0.1:<port>` or `localhost:<port>`
/// (the name is case-insensitive) for the port it is listening on, the port in its plain digits
/// (no `04321` or `+4321`). No `Host` is refused.
pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some((name, p)) = host.and_then(|h| h.rsplit_once(':')) else {
        return false;
    };
    p == port.to_string() && (name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost"))
}

/// Refuses a request whose `Host` does not name this server, so a page on another name that was
/// re-pointed at 127.0.0.1 (DNS rebinding) reaches no route. The server speaks HTTP/1.1, where
/// the name is the `Host` header; HTTP/2's `:authority` is not read. A request with more than
/// one `Host` is refused too. `/v1` answers in JSON words, the other routes with a bare 403 like
/// `loopback_only`.
async fn host_only(State(port): State<u16>, req: Request, next: Next) -> Response {
    let hosts = req.headers().get_all(header::HOST);
    let host = match hosts.iter().count() {
        1 => hosts.iter().next().and_then(|h| h.to_str().ok()),
        _ => None,
    };
    if host_allowed(host, port) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path == "/v1" || path.starts_with("/v1/") {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "This server only answers requests addressed to 127.0.0.1 or localhost."
            })),
        )
            .into_response();
    }
    StatusCode::FORBIDDEN.into_response()
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
        // The check needs the port actually bound (tests bind port 0).
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                // Port 0 is never a request's port: every request is then refused.
                tracing::error!("http server cannot read its port, so refuses every request: {e}");
                0
            }
        };
        let app = router(state)
            .layer(middleware::from_fn_with_state(port, host_only))
            .into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
        {
            tracing::error!("http server stopped: {e}");
        }
    })
}

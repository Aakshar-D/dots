use std::net::SocketAddr;
use std::sync::Arc;

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

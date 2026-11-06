//! The phone view's HTTPS server (phone view spec sections 4 and 8). Every response carries a
//! strict Content-Security-Policy and no CORS header; a request must name the address it
//! reached in `Host`; a POST must come from the page itself (`Origin`) as JSON; everything but
//! the page's own files and the pairing exchange needs a paired phone's cookie.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

use super::pairing::{ClaimRefusal, Collected};
use super::{net, Inner};
use crate::inbox::title_cut;
use crate::model::{HoldReason, MessageKind};
use crate::store::PhoneDevice;
use crate::Error;

/// The cookie that carries a paired phone's credential. `__Host-` makes browsers insist on
/// `Secure`, `Path=/` and no `Domain`.
pub const COOKIE: &str = "__Host-dots_phone";
/// How long the cookie lasts (browsers cap it at 400 days); each page load renews it.
const COOKIE_MAX_AGE: u64 = 400 * 24 * 60 * 60;
/// The largest request body.
pub const MAX_BODY: usize = 16 * 1024;
/// The longest answer to a question, in characters.
pub const MAX_ANSWER: usize = 8000;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// A connection is closed after this long, however busy; the browser opens another.
const CONNECTION_LIFETIME: Duration = Duration::from_secs(2 * 60);
const MAX_CONNECTIONS: usize = 64;
/// Connections one address may hold open at once, so one device cannot take every slot.
const MAX_PER_PEER: usize = 8;

/// The Content-Security-Policy of every response: the page's own script and style, its own
/// API, nothing inline, nothing framed.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; \
    frame-ancestors 'none'";

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_JS: &str = include_str!("assets/app.js");
const APP_CSS: &str = include_str!("assets/app.css");

/// Where a request came from and which of the listener's addresses it reached.
#[derive(Debug, Clone, Copy)]
struct Conn {
    peer: SocketAddr,
    local: SocketAddr,
}

type Shared = Weak<Inner>;

/// Whether a connection from `ip` is served: from the private network (or this PC itself).
fn peer_allowed(ip: IpAddr) -> bool {
    net::is_private_lan(ip) || ip.is_loopback()
}

/// The open connections of each address.
type PeerCounts = Arc<std::sync::Mutex<HashMap<IpAddr, usize>>>;

/// One open connection of an address; counted down when dropped.
struct PeerSlot {
    counts: PeerCounts,
    ip: IpAddr,
}

impl PeerSlot {
    fn take(counts: &PeerCounts, ip: IpAddr) -> Option<PeerSlot> {
        let mut c = counts.lock().unwrap();
        let n = c.entry(ip).or_insert(0);
        if *n >= MAX_PER_PEER {
            return None;
        }
        *n += 1;
        Some(PeerSlot {
            counts: counts.clone(),
            ip,
        })
    }
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap();
        if let Some(n) = c.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                c.remove(&self.ip);
            }
        }
    }
}

/// Accepts connections on `listener` until `cancel`; then every open connection is dropped.
pub(super) async fn serve(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    inner: Shared,
    cancel: CancellationToken,
) {
    let app = router(inner);
    let acceptor = TlsAcceptor::from(config);
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let peers: PeerCounts = Arc::default();
    let mut conns = JoinSet::new();
    loop {
        let (tcp, peer) = tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok(a) => a,
                Err(e) => {
                    tracing::debug!("phone view: accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
        };
        while conns.try_join_next().is_some() {}
        if !peer_allowed(peer.ip()) {
            continue;
        }
        let Some(peer_slot) = PeerSlot::take(&peers, peer.ip()) else {
            continue;
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let Ok(local) = tcp.local_addr() else {
            continue;
        };
        let app = app.clone().layer(Extension(Conn { peer, local }));
        let acceptor = acceptor.clone();
        let cancel = cancel.clone();
        conns.spawn(async move {
            let _slots = (slot, peer_slot);
            let Ok(Ok(tls)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await
            else {
                return;
            };
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .max_buf_size(64 * 1024);
            let conn = builder.serve_connection(TokioIo::new(tls), TowerToHyperService::new(app));
            tokio::select! {
                _ = conn => {}
                _ = cancel.cancelled() => {}
                _ = tokio::time::sleep(CONNECTION_LIFETIME) => {}
            }
        });
    }
}

fn router(inner: Shared) -> Router {
    let paired = Router::new()
        .route("/api/session", get(session))
        .route("/api/inbox", get(inbox))
        .route("/api/approvals/{id}", post(decide))
        .route("/api/questions/{id}/answer", post(answer))
        .route("/api/runs/{id}/run-anyway", post(run_anyway))
        .route("/api/forget", post(forget))
        .route_layer(middleware::from_fn_with_state(inner.clone(), paired_only));
    Router::new()
        .route(
            "/",
            get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }),
        )
        .route(
            "/app.js",
            get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }),
        )
        .route(
            "/app.css",
            get(|| async { asset("text/css; charset=utf-8", APP_CSS) }),
        )
        .route("/api/pair", post(pair))
        .route("/api/pair/wait", post(pair_wait))
        .merge(paired)
        .fallback(|| async { error(StatusCode::NOT_FOUND, "not found") })
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn(time_limit))
        .layer(middleware::from_fn(same_origin))
        .layer(middleware::from_fn(security_headers))
        .with_state(inner)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

fn error(status: StatusCode, text: &str) -> Response {
    (status, Json(json!({ "error": text }))).into_response()
}

/// A core error as the phone sees it: the words a person reads for what they can act on, a
/// plain "went wrong" for the rest (its details stay in the PC's log).
fn core_error(e: Error) -> Response {
    match e {
        Error::NotFound(_) => error(StatusCode::NOT_FOUND, "This item is no longer there."),
        Error::Invalid(t) => error(StatusCode::BAD_REQUEST, &t),
        Error::Conflict(t) => error(StatusCode::CONFLICT, &t),
        Error::NotAllowed(t) => error(StatusCode::FORBIDDEN, &t),
        other => {
            tracing::error!("phone view: {other}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong on the PC. Try again, or answer there.",
            )
        }
    }
}

fn gone() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "Dots is turning the phone view off.",
    )
}

/// Outermost: every response, errors included, gets the security headers.
async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    let set = |h: &mut HeaderMap, name: header::HeaderName, value: &'static str| {
        h.insert(name, HeaderValue::from_static(value));
    };
    set(h, header::CONTENT_SECURITY_POLICY, CSP);
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(h, header::REFERRER_POLICY, "no-referrer");
    set(h, header::CACHE_CONTROL, "no-store");
    set(
        h,
        header::HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    );
    set(
        h,
        header::HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    );
    set(
        h,
        header::HeaderName::from_static("permissions-policy"),
        "camera=(), microphone=(), geolocation=()",
    );
    res
}

/// The one `value` of `name`, or `None` when there is none or more than one.
fn single<'a>(headers: &'a HeaderMap, name: &header::HeaderName) -> Option<&'a str> {
    let mut all = headers.get_all(name).iter();
    let first = all.next()?;
    if all.next().is_some() {
        return None;
    }
    first.to_str().ok()
}

/// `Host` must be exactly the address and port the connection reached (no other name, so a
/// page re-pointed here by DNS reaches nothing). Only GET and POST are served; a POST must
/// carry the page's own `Origin` and a JSON body. No preflight is ever answered.
async fn same_origin(Extension(conn): Extension<Conn>, req: Request, next: Next) -> Response {
    let expected = format!("{}:{}", net::url_host(conn.local.ip()), conn.local.port());
    if single(req.headers(), &header::HOST) != Some(expected.as_str()) {
        return error(StatusCode::MISDIRECTED_REQUEST, "wrong address");
    }
    match *req.method() {
        Method::GET | Method::HEAD => {}
        Method::POST => {
            let origin = format!("https://{expected}");
            if single(req.headers(), &header::ORIGIN) != Some(origin.as_str()) {
                return error(StatusCode::FORBIDDEN, "cross-site request refused");
            }
            let json = single(req.headers(), &header::CONTENT_TYPE)
                .is_some_and(|t| t.split(';').next() == Some("application/json"));
            if !json {
                return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "send JSON");
            }
        }
        _ => return error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"),
    }
    next.run(req).await
}

async fn time_limit(req: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(req)).await {
        Ok(res) => res,
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "the PC took too long"),
    }
}

/// The credential in the request's cookie, if exactly one is there.
fn credential(headers: &HeaderMap) -> Option<String> {
    let mut found = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().strip_prefix(COOKIE)?.strip_prefix('='));
    let first = found.next()?;
    if found.next().is_some() || first.is_empty() || first.len() > 128 {
        return None;
    }
    Some(first.to_string())
}

fn cookie(credential: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE}={credential}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age={COOKIE_MAX_AGE}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn clear_cookie() -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Lets through only a paired phone, which the handler gets as an extension. An address with
/// too many failures is refused before its cookie is read.
async fn paired_only(
    State(inner): State<Shared>,
    Extension(conn): Extension<Conn>,
    mut req: Request,
    next: Next,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    let ip = conn.peer.ip();
    if inner.blocked(ip) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tries. Wait a few minutes.",
        );
    }
    // Only a wrong credential counts as a failure: a request without one guesses nothing, and
    // counting it would let any page the phone opens lock the phone out (SameSite=Strict
    // leaves the cookie off a cross-site request).
    let presented = credential(req.headers());
    let device = match &presented {
        Some(c) => match inner.authenticate(c, ip).await {
            Ok(d) => d.map(|d| (d, c.clone())),
            Err(e) => return core_error(e),
        },
        None => None,
    };
    let Some((device, credential)) = device else {
        if presented.is_some() {
            inner.fail(ip);
        }
        let mut res = error(StatusCode::UNAUTHORIZED, "not paired");
        res.headers_mut().insert(header::SET_COOKIE, clear_cookie());
        return res;
    };
    req.extensions_mut().insert(device);
    req.extensions_mut().insert(Credential(credential));
    next.run(req).await
}

/// The credential the request was let in with, to renew its cookie.
#[derive(Clone)]
struct Credential(String);

#[derive(Deserialize)]
struct PairBody {
    secret: String,
    #[serde(default)]
    name: String,
}

#[derive(Serialize)]
struct PairAnswer {
    claim: String,
    #[serde(rename = "match")]
    match_code: String,
}

async fn pair(
    State(inner): State<Shared>,
    Extension(conn): Extension<Conn>,
    Json(body): Json<PairBody>,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    let ip = conn.peer.ip();
    if inner.blocked(ip) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tries. Wait a few minutes.",
        );
    }
    if body.secret.len() > 128 || body.name.len() > 400 {
        inner.fail(ip);
        return error(StatusCode::BAD_REQUEST, "That code is not right.");
    }
    let Ok(name) = super::clean_name(&body.name) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Give this phone a name of at most 40 letters, without special characters.",
        );
    };
    match inner.claim(&body.secret, name, ip).await {
        Ok((claim, match_code)) => Json(PairAnswer { claim, match_code }).into_response(),
        Err(ClaimRefusal::Busy) => error(
            StatusCode::CONFLICT,
            "Another phone is pairing right now. Try again in two minutes.",
        ),
        Err(ClaimRefusal::NoPairing) => {
            inner.fail(ip);
            error(
                StatusCode::GONE,
                "No pairing is waiting. On the PC, open Settings > Phone and choose Pair a phone.",
            )
        }
        Err(ClaimRefusal::Wrong) => {
            inner.fail(ip);
            error(StatusCode::FORBIDDEN, "That code is not right.")
        }
    }
}

#[derive(Deserialize)]
struct WaitBody {
    claim: String,
}

async fn pair_wait(
    State(inner): State<Shared>,
    Extension(conn): Extension<Conn>,
    Json(body): Json<WaitBody>,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    let ip = conn.peer.ip();
    if inner.blocked(ip) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tries. Wait a few minutes.",
        );
    }
    let collected = match inner.collect(&body.claim, ip).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            inner.fail(ip);
            return error(
                StatusCode::NOT_FOUND,
                "This pairing is over. Start again on the PC.",
            );
        }
        Err(e) => return core_error(e),
    };
    let state = match &collected.0 {
        Collected::Waiting => "waiting",
        Collected::Allowed { .. } => "paired",
        Collected::Refused => "refused",
        Collected::Expired => "expired",
    };
    let mut res = Json(json!({ "state": state })).into_response();
    if let Some(credential) = collected.1 {
        res.headers_mut()
            .insert(header::SET_COOKIE, cookie(&credential));
    }
    res
}

/// Who the phone is; renews its cookie.
async fn session(
    Extension(device): Extension<PhoneDevice>,
    Extension(Credential(credential)): Extension<Credential>,
) -> Response {
    let mut res = Json(json!({ "name": device.name })).into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, cookie(&credential));
    res
}

/// What the phone shows of the Inbox: pending approvals with the exact call, open questions,
/// and runs held for usage.
#[derive(Serialize)]
struct PhoneInbox {
    approvals: Vec<PhoneApproval>,
    questions: Vec<PhoneQuestion>,
    held: Vec<PhoneHeld>,
}

#[derive(Serialize)]
struct PhoneApproval {
    id: String,
    dot_name: String,
    /// The Inbox's words: "<dot> asks to use <tool>".
    title: String,
    tool: String,
    /// The call's input exactly, as indented JSON.
    input: String,
    parked: bool,
    /// What approving a parked call does.
    parked_note: Option<String>,
    created_at: String,
}

#[derive(Serialize)]
struct PhoneQuestion {
    id: String,
    dot_name: String,
    text: String,
    choices: Vec<String>,
    created_at: String,
}

#[derive(Serialize)]
struct PhoneHeld {
    run_id: String,
    dot_name: String,
    reason: String,
    queued_at: String,
    starts_after: Option<String>,
    /// Run anyway applies: held for usage, not for a role.
    can_run_anyway: bool,
}

/// What a parked approval's card says in the app (`parkedNote`).
fn parked_note(self_apply: bool) -> String {
    if self_apply {
        "The run stopped waiting and is parked. If you approve, Dots performs exactly this call \
         and then continues the run."
            .into()
    } else {
        "The run stopped waiting and is parked. Approving continues it in a new run.".into()
    }
}

async fn inbox(State(inner): State<Shared>) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    let (full, messages) =
        match tokio::try_join!(inner.backend.inbox(), inner.backend.waiting_messages()) {
            Ok(both) => both,
            Err(e) => return core_error(e),
        };
    let approvals = full
        .approvals
        .iter()
        .map(|a| PhoneApproval {
            id: a.approval.id.clone(),
            dot_name: a.dot_name.clone(),
            title: title_cut(&format!("{} asks to use {}", a.dot_name, a.approval.tool)),
            tool: a.approval.tool.clone(),
            input: serde_json::to_string_pretty(&a.approval.input).unwrap_or_default(),
            parked: a.approval.parked,
            parked_note: a
                .approval
                .parked
                .then(|| parked_note(a.approval.self_apply)),
            created_at: a.approval.created_at.clone(),
        })
        .collect();
    let questions = messages
        .into_iter()
        .filter(|m| m.kind == MessageKind::Question && m.waits())
        .filter(|m| !full.hidden_messages.contains(&m.id))
        .map(|m| PhoneQuestion {
            id: m.id,
            dot_name: m.dot_name,
            text: m.text,
            choices: m.choices.unwrap_or_default(),
            created_at: m.created_at,
        })
        .collect();
    let held = full
        .held
        .iter()
        .map(|h| {
            let (reason, can_run_anyway) = match &h.reason {
                HoldReason::Usage => (
                    "Waiting because your Claude usage is high.".to_string(),
                    true,
                ),
                HoldReason::Role { text, .. } => (text.clone(), false),
                // The phone's Run anyway releases a usage hold only; a capped run is let
                // past its cap from the PC (spend caps design).
                HoldReason::SpendCap { text, .. } => (text.clone(), false),
            };
            PhoneHeld {
                run_id: h.run_id.clone(),
                dot_name: h.dot_name.clone(),
                reason,
                queued_at: h.queued_at.clone(),
                starts_after: h.starts_after.clone(),
                can_run_anyway,
            }
        })
        .collect();
    Json(PhoneInbox {
        approvals,
        questions,
        held,
    })
    .into_response()
}

#[derive(Deserialize)]
struct DecideBody {
    approved: bool,
}

/// Approve once or Deny, through the app's own decision (no rule can be saved from a phone).
async fn decide(
    State(inner): State<Shared>,
    Extension(device): Extension<PhoneDevice>,
    Path(id): Path<String>,
    Json(body): Json<DecideBody>,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    let approval = match inner.backend.decide_approval(&id, body.approved).await {
        Ok(a) => a,
        Err(e) => return core_error(e),
    };
    let via = format!("phone: {}", device.name);
    if let Err(e) = inner.backend.note_decided_via(&id, &via).await {
        tracing::error!("recording where approval {id} was decided failed: {e}");
    }
    let (action, verb) = if body.approved {
        ("approve", "Approved")
    } else {
        ("deny", "Denied")
    };
    inner
        .record(&device, action, &id, format!("{verb} {}", approval.tool))
        .await;
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct AnswerBody {
    text: String,
}

async fn answer(
    State(inner): State<Shared>,
    Extension(device): Extension<PhoneDevice>,
    Path(id): Path<String>,
    Json(body): Json<AnswerBody>,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    if body.text.chars().count() > MAX_ANSWER {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("An answer is at most {MAX_ANSWER} characters."),
        );
    }
    let run = match inner.backend.reply(&id, &body.text).await {
        Ok(r) => r,
        Err(e) => return core_error(e),
    };
    inner
        .record(
            &device,
            "answer",
            &id,
            format!("Answered a question (run {})", run.id),
        )
        .await;
    Json(json!({ "ok": true })).into_response()
}

async fn run_anyway(
    State(inner): State<Shared>,
    Extension(device): Extension<PhoneDevice>,
    Path(id): Path<String>,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    if let Err(e) = inner.backend.run_release_hold(&id).await {
        return core_error(e);
    }
    inner
        .record(&device, "run_anyway", &id, "Ran a held run anyway".into())
        .await;
    Json(json!({ "ok": true })).into_response()
}

/// The phone forgets itself: it is removed, and its cookie cleared.
async fn forget(
    State(inner): State<Shared>,
    Extension(device): Extension<PhoneDevice>,
) -> Response {
    let Some(inner) = inner.upgrade() else {
        return gone();
    };
    if let Err(e) = inner.store.remove_phone_device(&device.id).await {
        return core_error(e);
    }
    inner
        .record(&device, "forgot", &device.id, "Unpaired itself".into())
        .await;
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, clear_cookie());
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credential_is_read_from_one_cookie_only() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; __Host-dots_phone=abc; b=2"),
        );
        assert_eq!(credential(&h).as_deref(), Some("abc"));
        h.append(
            header::COOKIE,
            HeaderValue::from_static("__Host-dots_phone=def"),
        );
        assert_eq!(credential(&h), None);
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("dots_phone=abc"));
        assert_eq!(credential(&h), None);
    }

    #[test]
    fn the_cookie_is_host_only_secure_http_only_and_strict() {
        let c = cookie("abc");
        let c = c.to_str().unwrap();
        assert!(c.starts_with("__Host-dots_phone=abc;"));
        for part in ["Path=/", "Secure", "HttpOnly", "SameSite=Strict"] {
            assert!(c.contains(part), "{c}");
        }
        assert!(!c.contains("Domain"));
    }

    #[test]
    fn only_private_or_local_peers_are_served() {
        assert!(peer_allowed("192.168.0.4".parse().unwrap()));
        assert!(peer_allowed("127.0.0.1".parse().unwrap()));
        assert!(!peer_allowed("8.8.8.8".parse().unwrap()));
        assert!(!peer_allowed("100.64.0.9".parse().unwrap()));
    }
}

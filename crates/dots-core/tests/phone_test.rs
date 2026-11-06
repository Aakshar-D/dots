//! Answer from your phone (phone view spec): the listener is off by default and closes when
//! switched off; pairing needs a one-time token and the person's yes on the PC; every route
//! but the page and the pairing exchange needs a paired phone; cross-site and misaddressed
//! requests are refused; and a phone's decision goes through the runtime's own. The listener
//! binds 127.0.0.1 here (`Phone::with_addresses`); the choice of real addresses is tested in
//! `phone::net`.

mod common;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::limits::FIVE_HOUR;
use dots_core::model::{
    Approval, ApprovalStatus, Dot, DotSpec, Message, NewRun, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::phone::{PairApprover, PairRequest, Phone};
use dots_core::settings::Settings;
use dots_core::store::{LimitReading, Store};
use dots_core::util::{timestamp, token_hash};
use dots_core::vault::org::TestProtector;
use dots_core::{Config, Runtime};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, HOST, ORIGIN, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use tempfile::TempDir;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-06T10:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// The person on the PC: answers every pairing dialog with `allow`, and keeps what it showed.
struct Person {
    allow: bool,
    asked: Mutex<Vec<PairRequest>>,
}

#[async_trait]
impl PairApprover for Person {
    async fn approve(&self, request: PairRequest) -> bool {
        self.asked.lock().unwrap().push(request);
        self.allow
    }
}

struct T {
    dir: TempDir,
    rt: Arc<Runtime>,
    phone: Phone,
    person: Arc<Person>,
    port: u16,
    http: reqwest::Client,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn start_with(allow: bool, usage_hold: bool) -> T {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    if usage_hold {
        let store = Store::open(&data.join(dots_core::store::DB_FILE))
            .await
            .unwrap();
        store
            .save_limit_readings(&[LimitReading {
                window: FIVE_HOUR.into(),
                utilization: 1.0,
                resets_at: timestamp(t0() + TimeDelta::hours(5)),
                status: "allowed".into(),
                read_at: timestamp(t0()),
            }])
            .await
            .unwrap();
        store.close().await;
    }
    let mut cfg = Config::new(data);
    cfg.port = 0;
    cfg.clock = FakeClock::new(t0());
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Arc::new(Runtime::start(cfg).await.unwrap());
    rt.hold_runs(true).await;
    let person = Arc::new(Person {
        allow,
        asked: Mutex::new(Vec::new()),
    });
    let phone = Phone::with_addresses(
        rt.store().clone(),
        rt.clone(),
        Arc::new(TestProtector),
        person.clone(),
        vec!["127.0.0.1".parse::<IpAddr>().unwrap()],
    );
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    T {
        dir,
        rt,
        phone,
        person,
        port: free_port(),
        http,
    }
}

async fn start() -> T {
    start_with(true, false).await
}

fn on(port: u16) -> Settings {
    Settings {
        phone_enabled: true,
        phone_port: port,
        ..Settings::default()
    }
}

/// A request's answer: status, headers and JSON body (`Null` when not JSON).
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl T {
    fn base(&self) -> String {
        format!("https://127.0.0.1:{}", self.port)
    }

    async fn turn_on(&self) {
        self.phone.apply(&on(self.port)).await;
        assert!(
            self.phone.status().await.running,
            "{:?}",
            self.phone.status().await
        );
    }

    /// A request as the page makes it: its own Host and, for a POST, its own Origin and JSON.
    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        cookie: Option<&str>,
    ) -> Answer {
        let mut req = self
            .http
            .request(method.clone(), format!("{}{path}", self.base()));
        if method == Method::POST {
            req = req
                .header(ORIGIN, self.base())
                .header(CONTENT_TYPE, "application/json")
                .body(body.unwrap_or(json!({})).to_string());
        }
        if let Some(c) = cookie {
            req = req.header(COOKIE, format!("__Host-dots_phone={c}"));
        }
        answer(req.send().await.unwrap()).await
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> Answer {
        self.send(Method::GET, path, None, cookie).await
    }

    async fn post(&self, path: &str, body: Value, cookie: Option<&str>) -> Answer {
        self.send(Method::POST, path, Some(body), cookie).await
    }

    /// Pairs a phone through the pairing link; returns its credential.
    async fn pair(&self) -> String {
        let view = self.phone.start_pairing(None).await.unwrap();
        let token = token_of(&view.url);
        let claimed = self
            .post(
                "/api/pair",
                json!({ "secret": token, "name": "iPhone" }),
                None,
            )
            .await;
        assert_eq!(claimed.status, StatusCode::OK, "{}", claimed.body);
        let claim = claimed.body["claim"].as_str().unwrap().to_string();
        let answered = self.wait(&claim).await;
        assert_eq!(answered.body["state"], "paired");
        credential_of(&answered.headers).expect("a paired phone gets its cookie")
    }

    /// Asks about `claim` until the person has answered.
    async fn wait(&self, claim: &str) -> Answer {
        for _ in 0..100 {
            let a = self
                .post("/api/pair/wait", json!({ "claim": claim }), None)
                .await;
            if a.body["state"] != "waiting" {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the claim kept waiting");
    }

    async fn dot(&self, name: &str) -> Dot {
        let mut s: DotSpec = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        self.rt.create_dot(s, None, None).await.unwrap()
    }

    async fn approval(&self, dot: &Dot) -> Approval {
        let store = self.rt.store();
        let run = store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        store
            .set_status(&run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();
        store
            .create_approval(&run.id, "Bash", &json!({"command": "rm -rf build"}))
            .await
            .unwrap()
    }

    async fn question(&self, dot: &Dot, text: &str) -> Message {
        let store = self.rt.store();
        let run = store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        assert!(store.claim_run(&run.id).await.unwrap());
        store.mark_delivered(&run.id, Utc::now()).await.unwrap();
        store
            .finish_run(&run.id, RunStatus::Succeeded, None, None)
            .await
            .unwrap();
        store
            .add_question(&dot.id, &run.id, text, None)
            .await
            .unwrap()
            .unwrap()
    }
}

async fn answer(res: reqwest::Response) -> Answer {
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.json::<Value>().await.unwrap_or(Value::Null);
    Answer {
        status,
        headers,
        body,
    }
}

fn token_of(url: &str) -> String {
    let fragment = url.split_once('#').unwrap().1;
    fragment
        .split('&')
        .find_map(|p| p.strip_prefix("t="))
        .unwrap()
        .to_string()
}

fn credential_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| v.strip_prefix("__Host-dots_phone="))
        .map(|v| v.split(';').next().unwrap().to_string())
        .filter(|v| !v.is_empty())
}

#[tokio::test]
async fn the_listener_is_off_by_default() {
    assert!(!Settings::default().phone_enabled);
    let t = start().await;
    t.phone.apply(&Settings::default()).await;
    let status = t.phone.status().await;
    assert!(!status.running);
    assert!(status.addresses.is_empty());
    assert!(t.phone.start_pairing(None).await.is_err());
    assert!(
        t.http.get(t.base()).send().await.is_err(),
        "nothing listens"
    );
}

#[tokio::test]
async fn turning_it_off_closes_the_listener_and_forgets_the_pairing() {
    let t = start().await;
    t.turn_on().await;
    let status = t.phone.status().await;
    assert_eq!(status.addresses, vec![t.base()]);
    let fingerprint = status.fingerprint.unwrap();
    assert_eq!(fingerprint.len(), 32 * 3 - 1);
    let page = t.get("/", None).await;
    assert_eq!(page.status, StatusCode::OK);
    t.phone.start_pairing(None).await.unwrap();
    assert!(t.phone.status().await.pairing);

    t.phone.apply(&Settings::default()).await;
    assert!(!t.phone.status().await.running);
    assert!(t.http.get(t.base()).send().await.is_err(), "closed at once");

    // On again: the same certificate, and no pairing left over.
    t.turn_on().await;
    let again = t.phone.status().await;
    assert_eq!(again.fingerprint.as_deref(), Some(fingerprint.as_str()));
    assert!(!again.pairing);
}

#[tokio::test]
async fn every_response_has_the_security_headers_and_no_cors() {
    let t = start().await;
    t.turn_on().await;
    for path in ["/", "/app.js", "/app.css", "/api/inbox", "/nope"] {
        let a = t.get(path, None).await;
        let h = &a.headers;
        let csp = h["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("default-src 'none'") && csp.contains("script-src 'self'"),
            "{path}"
        );
        assert!(!csp.contains("unsafe-inline"));
        assert_eq!(h["x-frame-options"], "DENY");
        assert_eq!(h["x-content-type-options"], "nosniff");
        assert_eq!(h["cache-control"], "no-store");
        assert!(h.get("access-control-allow-origin").is_none(), "{path}");
    }
    // A preflight is not answered.
    let pre = t
        .http
        .request(Method::OPTIONS, format!("{}/api/inbox", t.base()))
        .header(ORIGIN, "https://evil.example")
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(pre.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn pairing_needs_the_token_once_and_the_persons_yes() {
    let t = start().await;
    t.turn_on().await;
    let view = t.phone.start_pairing(None).await.unwrap();
    assert!(view.url.starts_with(&format!("{}/#t=", t.base())));
    assert!(view.url.contains("&fp="));
    assert_eq!(view.code.len(), 9);
    assert!(!view.qr_path.is_empty());
    let token = token_of(&view.url);

    let claimed = t
        .post(
            "/api/pair",
            json!({ "secret": token, "name": "iPhone" }),
            None,
        )
        .await;
    assert_eq!(claimed.status, StatusCode::OK);
    let match_code = claimed.body["match"].as_str().unwrap().to_string();
    // The token is used up.
    let again = t
        .post(
            "/api/pair",
            json!({ "secret": token, "name": "Thief" }),
            None,
        )
        .await;
    assert_eq!(again.status, StatusCode::GONE);

    let paired = t.wait(claimed.body["claim"].as_str().unwrap()).await;
    assert_eq!(paired.body["state"], "paired");
    let set_cookie = paired.headers[SET_COOKIE].to_str().unwrap();
    for part in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(set_cookie.contains(part), "{set_cookie}");
    }
    // The PC asked, with the name, the address and the number the phone shows.
    let asked = t.person.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].name, "iPhone");
    assert_eq!(asked[0].ip.to_string(), "127.0.0.1");
    assert_eq!(asked[0].match_code, match_code);

    let devices = t.phone.devices().await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "iPhone");
    // Only the hash is stored.
    let credential = credential_of(&paired.headers).unwrap();
    assert_eq!(credential.len(), 43);
    let store = t.rt.store();
    let (found, stored) = store
        .phone_device_by_hash(&token_hash(&credential))
        .await
        .unwrap()
        .expect("the phone is found by its credential's hash");
    assert_eq!(found.id, devices[0].id);
    assert!(stored.starts_with("sha256:"));
    assert!(store
        .phone_device_by_hash(&credential)
        .await
        .unwrap()
        .is_none());

    let inbox = t.get("/api/inbox", Some(&credential)).await;
    assert_eq!(inbox.status, StatusCode::OK);
    let actions = t.phone.actions(10).await.unwrap();
    assert_eq!(actions[0].action, "paired");
}

#[tokio::test]
async fn the_short_code_pairs_too() {
    let t = start().await;
    t.turn_on().await;
    let view = t.phone.start_pairing(None).await.unwrap();
    let typed = view.code.to_lowercase();
    let claimed = t
        .post(
            "/api/pair",
            json!({ "secret": typed, "name": "Pixel" }),
            None,
        )
        .await;
    assert_eq!(claimed.status, StatusCode::OK);
    let paired = t.wait(claimed.body["claim"].as_str().unwrap()).await;
    assert_eq!(paired.body["state"], "paired");
}

#[tokio::test]
async fn a_phone_the_person_refuses_is_not_paired() {
    let t = start_with(false, false).await;
    t.turn_on().await;
    let view = t.phone.start_pairing(None).await.unwrap();
    let claimed = t
        .post(
            "/api/pair",
            json!({ "secret": token_of(&view.url), "name": "iPhone" }),
            None,
        )
        .await;
    let refused = t.wait(claimed.body["claim"].as_str().unwrap()).await;
    assert_eq!(refused.body["state"], "refused");
    assert!(credential_of(&refused.headers).is_none());
    assert!(t.phone.devices().await.unwrap().is_empty());
}

#[tokio::test]
async fn wrong_codes_end_the_pairing_and_failures_are_rate_limited() {
    let t = start().await;
    t.turn_on().await;
    let view = t.phone.start_pairing(None).await.unwrap();
    for _ in 0..5 {
        let a = t
            .post(
                "/api/pair",
                json!({ "secret": "AAAA-AAAA", "name": "x" }),
                None,
            )
            .await;
        assert_eq!(a.status, StatusCode::FORBIDDEN);
    }
    // The pairing ended: even the right token is too late.
    let late = t
        .post(
            "/api/pair",
            json!({ "secret": token_of(&view.url), "name": "x" }),
            None,
        )
        .await;
    assert_eq!(late.status, StatusCode::GONE);
    // Bad cookies count too; the tenth failure locks the address out.
    for _ in 0..4 {
        assert_eq!(
            t.get("/api/inbox", Some("forged")).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        t.get("/api/inbox", Some("forged")).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    t.phone.start_pairing(None).await.unwrap();
    let locked = t
        .post(
            "/api/pair",
            json!({ "secret": "AAAA-AAAA", "name": "x" }),
            None,
        )
        .await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn every_api_route_needs_a_paired_phone() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    let routes: [(Method, &str); 6] = [
        (Method::GET, "/api/session"),
        (Method::GET, "/api/inbox"),
        (Method::POST, "/api/approvals/01J"),
        (Method::POST, "/api/questions/01J/answer"),
        (Method::POST, "/api/runs/01J/run-anyway"),
        (Method::POST, "/api/forget"),
    ];
    for (method, path) in routes.iter().cloned() {
        let none = t.send(method.clone(), path, None, None).await;
        assert_eq!(none.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(none.body["error"], "not paired");
    }
    // A forged credential of the right shape is refused as well.
    let forged = "A".repeat(43);
    assert_eq!(
        t.get("/api/inbox", Some(&forged)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.get("/api/session", Some(&credential)).await.body["name"],
        "iPhone"
    );
}

#[tokio::test]
async fn cross_site_and_misaddressed_requests_are_refused() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    let url = format!("{}/api/forget", t.base());
    let cookie = format!("__Host-dots_phone={credential}");
    // No Origin.
    let res = t
        .http
        .post(&url)
        .header(COOKIE, &cookie)
        .header(CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // Another site's Origin.
    let res = t
        .http
        .post(&url)
        .header(COOKIE, &cookie)
        .header(ORIGIN, "https://evil.example")
        .header(CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // The plain http origin of the same address.
    let res = t
        .http
        .post(&url)
        .header(COOKIE, &cookie)
        .header(ORIGIN, format!("http://127.0.0.1:{}", t.port))
        .header(CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    // A form post (no preflight in a browser) is not JSON.
    let res = t
        .http
        .post(&url)
        .header(COOKIE, &cookie)
        .header(ORIGIN, t.base())
        .header(CONTENT_TYPE, "text/plain")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    // Another name for this address (DNS rebinding) reaches nothing.
    let res = t
        .http
        .get(format!("{}/api/inbox", t.base()))
        .header(HOST, HeaderValue::from_static("evil.example"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST);
    // A body over the limit is refused.
    let big = "x".repeat(20 * 1024);
    let res = t
        .post(
            "/api/questions/01J/answer",
            json!({ "text": big }),
            Some(&credential),
        )
        .await;
    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    // The phone is still paired after all of that.
    assert_eq!(t.phone.devices().await.unwrap().len(), 1);
}

#[tokio::test]
async fn removing_a_phone_revokes_it_at_once() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    assert_eq!(
        t.get("/api/inbox", Some(&credential)).await.status,
        StatusCode::OK
    );
    let id = t.phone.devices().await.unwrap()[0].id.clone();
    t.phone.remove_device(&id).await.unwrap();
    let after = t.get("/api/inbox", Some(&credential)).await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
    // The phone is told to drop its cookie.
    let cleared = after.headers[SET_COOKIE].to_str().unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
}

#[tokio::test]
async fn a_phone_can_unpair_itself() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    let gone = t.post("/api/forget", json!({}), Some(&credential)).await;
    assert_eq!(gone.status, StatusCode::OK);
    assert!(t.phone.devices().await.unwrap().is_empty());
    assert_eq!(
        t.get("/api/inbox", Some(&credential)).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn approvals_decided_from_a_phone_go_through_the_runtime() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    let dot = t.dot("builder").await;
    let approve = t.approval(&dot).await;
    let deny = t.approval(&dot).await;

    let inbox = t.get("/api/inbox", Some(&credential)).await.body;
    let shown = inbox["approvals"].as_array().unwrap();
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0]["title"], "builder asks to use Bash");
    assert!(shown[0]["input"].as_str().unwrap().contains("rm -rf build"));

    let mut events = t.rt.subscribe();
    let ok = t
        .post(
            &format!("/api/approvals/{}", approve.id),
            json!({ "approved": true }),
            Some(&credential),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    let decided = t.rt.store().get_approval(&approve.id).await.unwrap();
    assert_eq!(decided.status, ApprovalStatus::Approved);
    assert_eq!(decided.decided_via.as_deref(), Some("phone: iPhone"));
    // The runtime announced the decision, as for one made in the app.
    let mut saw_decided = false;
    while let Ok(e) = events.try_recv() {
        if matches!(e, dots_core::events::RuntimeEvent::ApprovalDecided { ref approval } if approval.id == approve.id)
        {
            saw_decided = true;
        }
    }
    assert!(saw_decided);

    // A rule cannot ride along: the phone has no way to send one, and an extra field is not
    // read.
    let denied = t
        .post(
            &format!("/api/approvals/{}", deny.id),
            json!({ "approved": false, "allow_rule": "Bash(rm:*)" }),
            Some(&credential),
        )
        .await;
    assert_eq!(denied.status, StatusCode::OK);
    let after = t.rt.store().get_approval(&deny.id).await.unwrap();
    assert_eq!(after.status, ApprovalStatus::Denied);
    let policy = t.rt.get_dot(&dot.id).await.unwrap().spec.policy;
    assert!(!format!("{policy:?}").contains("rm:*"));

    // Deciding twice meets the runtime's own refusal.
    let twice = t
        .post(
            &format!("/api/approvals/{}", approve.id),
            json!({ "approved": false }),
            Some(&credential),
        )
        .await;
    assert_eq!(twice.status, StatusCode::CONFLICT);
    let still = t.rt.store().get_approval(&approve.id).await.unwrap();
    assert_eq!(still.status, ApprovalStatus::Approved);

    let actions = t.phone.actions(10).await.unwrap();
    assert_eq!(actions[0].action, "deny");
    assert_eq!(actions[0].device_name, "iPhone");
    assert_eq!(actions[1].action, "approve");
    assert_eq!(actions[1].target, approve.id);
}

#[tokio::test]
async fn questions_are_answered_through_the_runtime() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    let dot = t.dot("writer").await;
    let q = t.question(&dot, "Which title?").await;
    let note = t.rt.store().add_note(&dot.id, None, "FYI").await.unwrap();

    let inbox = t.get("/api/inbox", Some(&credential)).await.body;
    let questions = inbox["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 1, "notes are not shown: {}", note.id);
    assert_eq!(questions[0]["text"], "Which title?");

    let empty = t
        .post(
            &format!("/api/questions/{}/answer", q.id),
            json!({ "text": "  " }),
            Some(&credential),
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    let ok = t
        .post(
            &format!("/api/questions/{}/answer", q.id),
            json!({ "text": "The second one" }),
            Some(&credential),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    let answered = t.rt.store().get_message(&q.id).await.unwrap();
    assert!(answered.answered_at.is_some());
    let again = t
        .post(
            &format!("/api/questions/{}/answer", q.id),
            json!({ "text": "Again" }),
            Some(&credential),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(t.phone.actions(1).await.unwrap()[0].action, "answer");
}

#[tokio::test]
async fn a_run_held_for_usage_can_run_anyway_from_the_phone() {
    let t = start_with(true, true).await;
    t.turn_on().await;
    let credential = t.pair().await;
    let dot = t.dot("nightly").await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
            .await
            .unwrap();
    let inbox = t.get("/api/inbox", Some(&credential)).await.body;
    let held = inbox["held"].as_array().unwrap();
    assert_eq!(held.len(), 1, "{inbox}");
    assert_eq!(held[0]["run_id"], run.id.as_str());
    assert_eq!(held[0]["can_run_anyway"], true);
    let ok = t
        .post(
            &format!("/api/runs/{}/run-anyway", run.id),
            json!({}),
            Some(&credential),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert!(t.rt.store().get_run(&run.id).await.unwrap().hold_released);
    let twice = t
        .post(
            &format!("/api/runs/{}/run-anyway", run.id),
            json!({}),
            Some(&credential),
        )
        .await;
    assert!(twice.status.is_client_error());
}

#[tokio::test]
async fn requests_without_a_cookie_do_not_lock_the_phone_out() {
    let t = start().await;
    t.turn_on().await;
    // What any page the phone opens could send: no cookie rides along (SameSite=Strict).
    for _ in 0..20 {
        assert_eq!(
            t.get("/api/inbox", None).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let credential = t.pair().await;
    assert_eq!(
        t.get("/api/inbox", Some(&credential)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_phone_unused_for_90_days_is_unpaired() {
    let t = start().await;
    t.turn_on().await;
    let credential = t.pair().await;
    let id = t.phone.devices().await.unwrap()[0].id.clone();
    let old = timestamp(Utc::now() - TimeDelta::days(91));
    t.rt.store()
        .touch_phone_device(&id, &old, "127.0.0.1")
        .await
        .unwrap();
    assert_eq!(
        t.get("/api/inbox", Some(&credential)).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert!(t.phone.devices().await.unwrap().is_empty());
    assert_eq!(t.phone.actions(1).await.unwrap()[0].action, "expired");
}

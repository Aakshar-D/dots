//! Audit records end to end (env vault spec §10, §13): a runtime joins an account whose policy
//! names an audit endpoint on this PC, runs a dot of it through the fake claude CLI, and its
//! sender posts the records over HTTP to a local sink. The sink gets NDJSON with the token
//! entry's value as the bearer, a refused post is kept and sent again, and what arrives is a
//! whole chain that never holds a value. Nothing leaves 127.0.0.1.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::Duration;
use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{at, code, keys, openbao, policy, sign, token, ACME, ACME_NAME, START, TOKEN};
use common::{spec, wait_status};
use dots_core::model::{RunStatus, WorkspaceMode};
use dots_core::store::HistoryState;
use dots_core::vault::audit::{verify_lines, AuditDot, AuditRecord, Event};
use dots_core::vault::{Grants, MemorySecrets};
use dots_core::{Config, Runtime};
use serde_json::json;

const LEVEL: &str = "org-plain-level-audit-8c2f";
const AUDIT_TOKEN: &str = "audit-bearer-token-3a91e7";

#[derive(Debug, Clone)]
struct Got {
    content_type: Option<String>,
    authorization: Option<String>,
    body: String,
    answered: u16,
}

/// A receiver on 127.0.0.1 that keeps every post and answers with `status`.
#[derive(Clone)]
struct Sink {
    got: Arc<Mutex<Vec<Got>>>,
    status: Arc<Mutex<u16>>,
}

impl Sink {
    async fn start() -> (String, Sink) {
        let sink = Sink {
            got: Arc::default(),
            status: Arc::new(Mutex::new(200)),
        };
        let app = axum::Router::new()
            .route("/dots", axum::routing::post(receive))
            .with_state(sink.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://127.0.0.1:{port}/dots"), sink)
    }

    fn answer(&self, status: u16) {
        *self.status.lock().unwrap() = status;
    }

    fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }
}

async fn receive(State(sink): State<Sink>, headers: HeaderMap, body: String) -> StatusCode {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let answered = *sink.status.lock().unwrap();
    sink.got.lock().unwrap().push(Got {
        content_type: header("content-type"),
        authorization: header("authorization"),
        body,
        answered,
    });
    StatusCode::from_u16(answered).unwrap()
}

#[tokio::test]
async fn a_runs_records_reach_a_local_sink_as_a_whole_chain_with_the_bearer_token() {
    let (endpoint, sink) = Sink::start().await;
    // The sink refuses at first, whichever pass reaches it: the runtime's own loop or ours.
    sink.answer(503);
    let bao = FakeOpenBao::start().await;
    bao.allow_token(TOKEN, false);
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(at(START));
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let args_out = dir.path().join("args.json");
    cfg.claude_env.push((
        "FAKE_CLAUDE_ARGS_OUT".into(),
        args_out.to_string_lossy().to_string(),
    ));
    let rt = Runtime::start(cfg).await.unwrap();

    let keys = keys();
    let extra = json!({
        "plain": ["DOTS_TEST_LEVEL"],
        "offline_hours": 0,
        "audit": {"endpoint": endpoint, "token_entry": "DOTS_AUDIT_TOKEN"},
    });
    let signed = sign(&keys, &policy(ACME, "2026-10-22T08:00:00Z", extra));
    bao.put_env(
        "secret",
        "dots",
        json!({"DOTS_TEST_LEVEL": LEVEL, "DOTS_AUDIT_TOKEN": AUDIT_TOKEN}),
    );
    bao.put_policy("secret", "dots", &signed);
    let join = code(ACME, ACME_NAME, openbao(&bao.address(), "dots"), &keys);
    let orgs = rt.vault().orgs().expect("the runtime's orgs");
    orgs.join(&join, Some(token())).await.unwrap();

    let mut triage = spec("triage");
    triage.workdir = dir.path().to_string_lossy().to_string();
    triage.workspace_mode = WorkspaceMode::Folder;
    triage.variables = Grants([("shell".to_string(), vec!["DOTS_TEST_LEVEL".to_string()])].into());
    let dot = rt
        .create_dot(triage, Some(ACME.into()), None)
        .await
        .unwrap();
    let run = rt.run_now(&dot.id).await.unwrap();
    wait_status(rt.store(), &run.id, RunStatus::Succeeded, 15).await;

    // Refused: the records stay queued, and the account says why.
    let sender = rt.audit_sender();
    assert_eq!(sender.pass().await, 0);
    let queued = rt.store().audit_queued(ACME).await.unwrap();
    assert_eq!(queued, 2);
    assert_eq!(
        sender.state(ACME).last_error.as_deref(),
        Some("the endpoint answered 503")
    );

    // After the wait, the same records go again and are taken.
    sink.answer(200);
    clock.set(at(START) + Duration::minutes(2));
    sender.pass().await;
    for _ in 0..500 {
        if rt.store().audit_queued(ACME).await.unwrap() == 0 {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    assert_eq!(rt.store().audit_queued(ACME).await.unwrap(), 0);
    assert_eq!(sender.state(ACME).last_error, None);

    let got = sink.got();
    assert!(got.len() >= 2);
    for post in &got {
        assert_eq!(post.content_type.as_deref(), Some("application/x-ndjson"));
        assert_eq!(
            post.authorization.as_deref(),
            Some(format!("Bearer {AUDIT_TOKEN}").as_str())
        );
        assert!(!post.body.contains(LEVEL) && !post.body.contains(AUDIT_TOKEN));
        assert!(!post.body.contains(TOKEN));
    }
    let taken: String = got
        .iter()
        .filter(|p| p.answered == 200)
        .map(|p| p.body.clone())
        .collect();
    let report = verify_lines(&taken);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    let records: Vec<AuditRecord> = taken
        .lines()
        .map(|line| AuditRecord::parse(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!((records[0].seq, records[0].event), (1, Event::Fetched));
    let resolved = &records[1];
    assert_eq!((resolved.seq, resolved.event), (2, Event::Resolved));
    assert_eq!(resolved.name.as_deref(), Some("DOTS_TEST_LEVEL"));
    assert_eq!(resolved.consumer.as_deref(), Some("shell"));
    assert_eq!(
        resolved.dot,
        Some(AuditDot {
            id: dot.id.clone(),
            name: "triage".into()
        })
    );
    assert_eq!(resolved.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(resolved.install, rt.store().install_id().await.unwrap());

    // The account's page: its endpoint, nothing waiting, and the run's record sent. The fetch
    // is not listed.
    let view = rt.org_audit(ACME).await.unwrap();
    assert_eq!(view.endpoint.as_deref(), Some(endpoint.as_str()));
    assert_eq!(view.queued, 0);
    assert!(view.send.last_sent_at.is_some());
    let rows: Vec<(Event, Option<String>, HistoryState)> = view
        .history
        .into_iter()
        .map(|row| (row.event, row.name, row.state))
        .collect();
    assert_eq!(
        rows,
        vec![(
            Event::Resolved,
            Some("DOTS_TEST_LEVEL".into()),
            HistoryState::Sent
        )]
    );
    assert!(matches!(
        rt.org_audit("beta").await,
        Err(dots_core::Error::NotFound(_))
    ));
    rt.shutdown();
}

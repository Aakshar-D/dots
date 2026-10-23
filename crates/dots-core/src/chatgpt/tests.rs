use super::*;
use openidconnect::core::{CoreIdTokenClaims, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey};
use openidconnect::{JsonWebKeyId, PrivateSigningKey};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn key() -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(
        include_str!("../../tests/fixtures/chatgpt-test-key.pem"),
        Some(JsonWebKeyId::new("dots-test".into())),
    )
    .unwrap()
}

fn keys() -> CoreJsonWebKeySet {
    CoreJsonWebKeySet::new(vec![key().as_verification_key()])
}

fn claims(issuer: &str, nonce: &str) -> Value {
    json!({
        "iss": issuer, "aud": "oaiapp_dots", "sub": "account-123",
        "exp": Utc::now().timestamp() + 300, "iat": Utc::now().timestamp(),
        "nonce": nonce, "name": "Alex", "email": "alex@example.com",
    })
}

fn signed(claims: Value) -> String {
    let claims: CoreIdTokenClaims = serde_json::from_value(claims).unwrap();
    CoreIdToken::new(
        claims,
        &key(),
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .unwrap()
    .to_string()
}

fn config() -> ChatGptConfig {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    ChatGptConfig::new(
        "oaiapp_dots".into(),
        &format!(
            "http://127.0.0.1:{}/callback",
            listener.local_addr().unwrap().port()
        ),
    )
    .unwrap()
}

#[test]
fn verifies_identity_without_requiring_access_or_refresh_tokens() {
    let config = config();
    let session = verify_identity(
        &signed(claims(ISSUER, "nonce")),
        &config,
        ISSUER,
        keys(),
        &Nonce::new("nonce".into()),
    )
    .unwrap();
    assert_eq!(
        session._identity,
        (ISSUER.into(), "oaiapp_dots".into(), "account-123".into())
    );
    assert_eq!(session.account.email.as_deref(), Some("alex@example.com"));
    assert_eq!(session.account.name.as_deref(), Some("Alex"));
}

#[test]
fn rejects_invalid_identity_claims_and_signatures() {
    let config = config();
    let nonce = Nonce::new("nonce".into());
    for (field, value) in [
        ("iss", json!("https://attacker.example")),
        ("aud", json!("another-client")),
        ("nonce", json!("another-nonce")),
        ("sub", json!("")),
        ("exp", json!(Utc::now().timestamp() - 60)),
        ("iat", json!(Utc::now().timestamp() + 300)),
    ] {
        let mut invalid = claims(ISSUER, "nonce");
        invalid[field] = value;
        assert!(
            verify_identity(&signed(invalid), &config, ISSUER, keys(), &nonce).is_err(),
            "{field}"
        );
    }
    let mut token = signed(claims(ISSUER, "nonce")).into_bytes();
    let signature = token.iter().rposition(|byte| *byte == b'.').unwrap() + 1;
    token[signature] = if token[signature] == b'A' { b'B' } else { b'A' };
    assert!(verify_identity(
        std::str::from_utf8(&token).unwrap(),
        &config,
        ISSUER,
        keys(),
        &nonce
    )
    .is_err());
    let mut missing_nonce = claims(ISSUER, "nonce");
    missing_nonce.as_object_mut().unwrap().remove("nonce");
    assert!(verify_identity(&signed(missing_nonce), &config, ISSUER, keys(), &nonce).is_err());
}

async fn provider() -> (MockServer, ChatGptConfig) {
    let server = MockServer::start().await;
    let mut config = config();
    config.issuer = Url::parse(&server.uri()).unwrap();
    Mock::given(method("GET")).and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": server.uri(), "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": format!("{}/token", server.uri()), "jwks_uri": format!("{}/jwks", server.uri()),
        }))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(keys()))
        .mount(&server)
        .await;
    (server, config)
}

#[tokio::test]
async fn browser_callback_checks_state_pkce_identity_and_sign_out() {
    let (server, config) = provider().await;
    let opened: Arc<Mutex<Option<Url>>> = Arc::new(Mutex::new(None));
    let opened_token = opened.clone();
    let issuer = server.uri();
    let redirect_uri = config.redirect_uri.as_str().to_owned();
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(move |request: &Request| {
            let body = Url::parse(&format!(
                "http://localhost/?{}",
                String::from_utf8_lossy(&request.body)
            ))
            .unwrap();
            let form = body
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(form["client_id"], "oaiapp_dots");
            assert_eq!(form["redirect_uri"], redirect_uri);
            assert_eq!(form["grant_type"], "authorization_code");
            assert_eq!(form["code"], "test-code");
            assert!(!form.contains_key("client_secret"));
            let url = opened_token.lock().unwrap().clone().unwrap();
            let query = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(query["scope"], "openid profile email");
            assert_eq!(query["code_challenge_method"], "S256");
            let verifier = openidconnect::PkceCodeVerifier::new(form["code_verifier"].to_string());
            assert_eq!(
                query["code_challenge"],
                PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str()
            );
            ResponseTemplate::new(200)
                .set_body_json(json!({"id_token": signed(claims(&issuer, &query["nonce"]))}))
        })
        .expect(1)
        .mount(&server)
        .await;
    let auth = ChatGptAuth::new(Some(config.clone()));
    let (browser_done, browser_received) = oneshot::channel();
    let result = auth
        .sign_in(move |url| {
            let url = Url::parse(url).unwrap();
            *opened.lock().unwrap() = Some(url.clone());
            let query = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            let state = query["state"].to_string();
            let mut callback = config.redirect_uri.clone();
            tokio::spawn(async move {
                let client = Client::new();
                // Neither an unrelated request nor mismatched/duplicate state consumes login.
                assert_eq!(
                    client.get(callback.clone()).send().await.unwrap().status(),
                    StatusCode::BAD_REQUEST
                );
                callback.set_query(Some("code=bad&state=attacker"));
                assert_eq!(
                    client.get(callback.clone()).send().await.unwrap().status(),
                    StatusCode::BAD_REQUEST
                );
                callback.set_query(Some(&format!("code=bad&state={state}&state=attacker")));
                assert_eq!(
                    client.get(callback.clone()).send().await.unwrap().status(),
                    StatusCode::BAD_REQUEST
                );
                callback.set_query(None);
                callback
                    .query_pairs_mut()
                    .extend_pairs([("code", "test-code"), ("state", &state)]);
                let response = client.get(callback).send().await.unwrap();
                assert_eq!(response.headers()["cache-control"], "no-store");
                assert!(!response.text().await.unwrap().contains("test-code"));
                let _ = browser_done.send(());
            });
            Ok(())
        })
        .await
        .unwrap();
    browser_received.await.unwrap();
    assert!(!result.signing_in);
    assert_eq!(
        result.account.unwrap().email.as_deref(),
        Some("alex@example.com")
    );
    let json = serde_json::to_string(&auth.status()).unwrap();
    for secret in [
        "test-code",
        "id_token",
        "access_token",
        "refresh_token",
        "account-123",
        "oaiapp_dots",
    ] {
        assert!(!json.contains(secret), "{secret}");
    }
    assert!(auth.sign_out().await.account.is_none());
}

#[tokio::test]
async fn cancellation_clears_pending_login_and_releases_the_port() {
    let (server, config) = provider().await;
    let port = config.redirect_uri.port().unwrap();
    let auth = Arc::new(ChatGptAuth::new(Some(config)));
    let attempt = auth.clone();
    let (opened, received) = oneshot::channel();
    let task = tokio::spawn(async move {
        attempt
            .sign_in(move |_| {
                let _ = opened.send(());
                Ok(())
            })
            .await
    });
    received.await.unwrap();
    assert!(auth.status().signing_in);
    assert!(auth.sign_in(|_| panic!("second attempt")).await.is_err());
    let status = auth.sign_out().await;
    assert!(!status.signing_in);
    assert!(status.account.is_none());
    assert!(task.await.unwrap().is_err());
    let _available = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.url.path() != "/token"));
}

#[tokio::test]
async fn browser_failure_and_port_conflict_do_not_leave_a_pending_attempt() {
    let (_server, config) = provider().await;
    let port = config.redirect_uri.port().unwrap();
    let auth = ChatGptAuth::new(Some(config));
    assert!(auth
        .sign_in(|_| Err(Error::Other("browser missing".into())))
        .await
        .is_err());
    assert!(!auth.status().signing_in);
    let _listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    assert!(auth
        .sign_in(|_| panic!("occupied port opened browser"))
        .await
        .is_err());
    assert!(!auth.status().signing_in);
}

#[test]
fn application_session_expires_without_exposing_stored_identity() {
    let auth = ChatGptAuth::new(Some(config()));
    auth.state.lock().unwrap().session = Some(Session {
        _identity: (ISSUER.into(), "oaiapp_dots".into(), "account-123".into()),
        account: ChatGptAccount {
            name: Some("Alex".into()),
            email: None,
        },
        expires: Instant::now() - Duration::from_secs(1),
        expires_at: Utc::now() - chrono::Duration::seconds(1),
    });
    assert!(auth.status().account.is_none());
}

#[tokio::test]
async fn callback_is_consumed_once_and_provider_denial_has_no_code() {
    for query in [
        "state=nonce&code=test-code",
        "state=nonce&error=access_denied",
        "state=nonce",
    ] {
        let (send, receive) = oneshot::channel();
        let state = Arc::new(CallbackState {
            state: "nonce".into(),
            issuer: ISSUER.into(),
            send: Mutex::new(Some(send)),
        });
        let response = callback_handler(State(state.clone()), RawQuery(Some(query.into()))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let result = receive.await.unwrap();
        assert_eq!(result.is_ok(), query.contains("code=test-code"));
        let replay = callback_handler(State(state), RawQuery(Some(query.into()))).await;
        assert_eq!(replay.status(), StatusCode::GONE);
    }
}

#[tokio::test]
async fn timeout_clears_pending_attempt_and_releases_callback_socket() {
    let (_provider, config) = provider().await;
    let port = config.redirect_uri.port().unwrap();
    let mut auth = ChatGptAuth::new(Some(config));
    auth.sign_in_limit = Duration::from_millis(50);
    let error = auth.sign_in(|_| Ok(())).await.unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(!auth.status().signing_in);
    assert!(auth.status().account.is_none());
    let _available = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
}

#[tokio::test]
async fn cancellation_during_token_exchange_cannot_create_a_session_later() {
    let (server, config) = provider().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let auth = Arc::new(ChatGptAuth::new(Some(config.clone())));
    let attempt = auth.clone();
    let task = tokio::spawn(async move {
        attempt
            .sign_in(move |url| {
                let url = Url::parse(url).unwrap();
                let query = url
                    .query_pairs()
                    .collect::<std::collections::HashMap<_, _>>();
                let mut callback = config.redirect_uri.clone();
                callback
                    .query_pairs_mut()
                    .extend_pairs([("code", "test-code"), ("state", &query["state"])]);
                tokio::spawn(async move {
                    let _ = Client::new().get(callback).send().await;
                });
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/token")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(auth.sign_out().await.account.is_none());
    assert!(task.await.unwrap().is_err());
    assert!(auth.status().account.is_none());
}

#[tokio::test]
async fn discovery_cannot_redirect_credentials_to_another_origin() {
    let (server, config) = provider().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": server.uri(), "authorization_endpoint": format!("{}/authorize", server.uri()),
            "token_endpoint": "https://attacker.example/token",
            "jwks_uri": format!("{}/jwks", server.uri()),
        })))
        .with_priority(1)
        .mount(&server)
        .await;
    let auth = ChatGptAuth::new(Some(config));
    assert!(auth
        .sign_in(|_| panic!("invalid discovery opened browser"))
        .await
        .is_err());
    assert!(!auth.status().signing_in);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attempt_cannot_hold_the_gate_before_its_pending_state_is_published() {
    let auth = Arc::new(ChatGptAuth::new(Some(config())));
    let state = auth.state.lock().unwrap();
    let attempt = auth.clone();
    let (started, receive) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        started.send(()).unwrap();
        attempt.sign_in(|_| Ok(())).await
    });
    receive.recv_timeout(Duration::from_secs(1)).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let gate_available = auth.gate.try_lock().is_ok();
    drop(state);
    task.abort();
    let _ = task.await;
    assert!(
        gate_available,
        "a concurrent sign-out could miss the unpublished cancellation token"
    );
}

#[tokio::test]
async fn a_new_attempt_cannot_jump_a_sign_out_waiter() {
    let auth = ChatGptAuth::new(Some(config()));
    auth.state.lock().unwrap().signing_out = 1;
    let waiting = SignOutGuard(&auth);
    let error = auth
        .sign_in(|_| panic!("sign-out waiter was bypassed"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("sign-out is in progress"));
    drop(waiting);
    assert_eq!(auth.state.lock().unwrap().signing_out, 0);
}

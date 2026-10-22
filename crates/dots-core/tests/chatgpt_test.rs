use dots_core::chatgpt::{ChatGptAuth, ChatGptConfig, DEFAULT_REDIRECT_URI};

#[tokio::test]
async fn missing_registration_is_explicit_and_never_opens_a_browser() {
    let auth = ChatGptAuth::new(None);
    let status = auth.status();
    assert!(!status.configured);
    assert!(!status.signing_in);
    assert!(status.account.is_none());
    assert!(status
        .setup_error
        .unwrap()
        .contains("registered OpenAI client"));
    assert!(auth
        .sign_in(|_| panic!("unconfigured login opened a browser"))
        .await
        .is_err());
    assert!(auth.sign_out().await.account.is_none());
}

#[test]
fn desktop_callback_is_loopback_only_and_cannot_hide_credentials_or_query() {
    assert!(ChatGptConfig::new("oaiapp_dots".into(), DEFAULT_REDIRECT_URI).is_ok());
    for redirect in [
        "https://example.com/callback",
        "http://localhost:47322/callback",
        "http://0.0.0.0:47322/callback",
        "http://127.0.0.1:0/callback",
        "http://127.0.0.1/callback",
        "http://user@127.0.0.1:47322/callback",
        "http://127.0.0.1:47322/callback?state=attacker",
        "http://127.0.0.1:47322/callback#fragment",
    ] {
        assert!(
            ChatGptConfig::new("oaiapp_dots".into(), redirect).is_err(),
            "{redirect}"
        );
    }
    for id in ["", "bad id", "bad\nclient"] {
        assert!(ChatGptConfig::new(id.into(), DEFAULT_REDIRECT_URI).is_err());
    }
}

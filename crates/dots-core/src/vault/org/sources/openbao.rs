//! OpenBao or HashiCorp Vault, KV v2 (env vault spec §6.1). The entries are the keys of
//! `<mount>/data/<path>/env`, one per variable, and the signed policy is key `DOTS_POLICY` of
//! `<mount>/data/<path>/policy` (§6.3). The person signs in with a token, which is renewed while
//! it is renewable, or with a username and password against a `userpass` or `ldap` mount, whose
//! token is kept in memory until it expires.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::HeaderValue;
use reqwest::{RequestBuilder, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use super::{
    collect_entries, endpoint, path_segments, read_json, request_failed, status_failed, Fetched,
    Field, Source, SourceCreds, SourceError, POLICY_ENTRY,
};
use crate::vault::org::{BaoAuth, OpenBaoSettings};

/// How often a token is checked, and renewed if it is renewable: once an hour per source.
const RENEW_EVERY: Duration = Duration::from_secs(60 * 60);

/// A login token this close to its expiry is not used: the next fetch logs in again.
const EXPIRY_MARGIN: Duration = Duration::from_secs(10);

/// The longest lease believed from a login answer: one year.
const MAX_LEASE_SECONDS: u64 = 365 * 24 * 60 * 60;

/// The texts, lowercase, of a login's `errors` that mean the username or password was refused: a
/// wrong password on `userpass` or `ldap`, or a failed bind as the user. Any other HTTP 400 from a
/// login, such as an ldap directory that cannot be reached, is not a refusal.
const LOGIN_REFUSED: [&str; 3] = [
    "invalid username or password",
    "invalid credentials",
    "failed to bind as user",
];

#[derive(Deserialize)]
struct LoginErrors {
    errors: Option<Vec<String>>,
}

/// An org's OpenBao or Vault source. It keeps, in memory only, the token a login gave and when a
/// token was last checked for renewal.
pub struct OpenBao {
    settings: OpenBaoSettings,
    http: reqwest::Client,
    /// The token of the last login, for the username and password it was given for.
    login: Mutex<Option<CachedToken>>,
    /// When a token was last checked for renewal, and which one, by its SHA-256.
    renewed: Mutex<Option<(Instant, [u8; 32])>>,
}

struct CachedToken {
    token: Zeroizing<String>,
    /// The SHA-256 of the username and password the token was given for.
    creds: [u8; 32],
    /// `None` for a token that does not expire (a lease of 0).
    expires: Option<Instant>,
}

impl CachedToken {
    fn usable(&self, creds: &[u8; 32]) -> bool {
        &self.creds == creds
            && self
                .expires
                .is_none_or(|at| Instant::now() + EXPIRY_MARGIN < at)
    }
}

/// The two reads of a fetch, and the names they go by in messages.
struct Reads {
    env: Url,
    env_name: String,
    policy: Url,
    policy_name: String,
}

#[derive(Deserialize)]
struct KvRead {
    data: Option<KvData>,
}

#[derive(Deserialize)]
struct KvData {
    data: Option<BTreeMap<String, Field>>,
}

#[derive(Serialize)]
struct LoginBody<'a> {
    password: &'a str,
}

#[derive(Deserialize)]
struct LoginAnswer {
    auth: Option<LoginAuth>,
}

#[derive(Deserialize)]
struct LoginAuth {
    client_token: Option<Zeroizing<String>>,
    #[serde(default)]
    lease_duration: Option<u64>,
}

#[derive(Deserialize)]
struct LookupAnswer {
    data: Option<LookupData>,
}

#[derive(Deserialize)]
struct LookupData {
    #[serde(default)]
    renewable: Option<bool>,
}

impl OpenBao {
    pub fn new(settings: OpenBaoSettings, http: reqwest::Client) -> OpenBao {
        OpenBao {
            settings,
            http,
            login: Mutex::new(None),
            renewed: Mutex::new(None),
        }
    }

    fn address(&self) -> &str {
        &self.settings.address
    }

    /// The URLs of the two reads, checked before any request is sent.
    fn reads(&self) -> Result<Reads, SourceError> {
        let mount = path_segments("mount", &self.settings.mount)?;
        let path = path_segments("path", &self.settings.path)?;
        let url = |leaf: &'static str| {
            let segments = ["v1"]
                .into_iter()
                .chain(mount.iter().copied())
                .chain(["data"])
                .chain(path.iter().copied())
                .chain([leaf]);
            endpoint(self.address(), segments)
        };
        let name = |leaf: &str| format!("{}/{}/{leaf}", mount.join("/"), path.join("/"));
        Ok(Reads {
            env: url("env")?,
            env_name: name("env"),
            policy: url("policy")?,
            policy_name: name("policy"),
        })
    }

    /// `auth/<mount>/login/<username>`, checked before any request is sent.
    fn login_url(&self, mount: &str, username: &str) -> Result<Url, SourceError> {
        let mount = path_segments("auth_mount", mount)?;
        if username.chars().any(char::is_control) || super::is_dot_segment(username) {
            return Err(SourceError::Invalid(
                "the username cannot be '.' or '..', or hold a control character".into(),
            ));
        }
        let segments = ["v1", "auth"]
            .into_iter()
            .chain(mount.iter().copied())
            .chain(["login", username]);
        endpoint(self.address(), segments)
    }

    /// A request with the namespace header, if the join code names one, and with `token`.
    fn request(
        &self,
        request: RequestBuilder,
        token: Option<&str>,
    ) -> Result<RequestBuilder, SourceError> {
        let mut request = request;
        if let Some(namespace) = &self.settings.namespace {
            let value = HeaderValue::from_str(namespace).map_err(|_| {
                SourceError::Invalid(format!(
                    "the source's namespace '{}' cannot be sent as a header",
                    namespace.escape_debug()
                ))
            })?;
            request = request.header("X-Vault-Namespace", value);
        }
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(token).map_err(|_| {
                SourceError::Invalid("the token holds characters a header cannot carry".into())
            })?;
            value.set_sensitive(true);
            request = request.header("X-Vault-Token", value);
        }
        Ok(request)
    }

    /// Reads the entries, then the policy, with `token`.
    async fn read(&self, reads: &Reads, token: &str) -> Result<Fetched, SourceError> {
        let env = self.read_kv(&reads.env, &reads.env_name, token).await?;
        let Some(env) = env else {
            return Err(SourceError::Invalid(format!(
                "no secrets at {}",
                reads.env_name
            )));
        };
        let entries = collect_entries(env)?;

        let no_policy =
            || SourceError::Invalid(format!("no {POLICY_ENTRY} at {}", reads.policy_name));
        let policy = self
            .read_kv(&reads.policy, &reads.policy_name, token)
            .await?
            .ok_or_else(no_policy)?
            .remove(POLICY_ENTRY)
            .ok_or_else(no_policy)?;
        let Field::Text(mut policy) = policy else {
            return Err(SourceError::Invalid(format!("{POLICY_ENTRY} is not text")));
        };
        Ok(Fetched {
            entries,
            policy: std::mem::take(&mut *policy),
        })
    }

    /// One KV v2 read: its keys, or `None` when nothing is there (404, 204 or no data). A
    /// 404's body is never read: a deleted version's carries its metadata.
    async fn read_kv(
        &self,
        url: &Url,
        name: &str,
        token: &str,
    ) -> Result<Option<BTreeMap<String, Field>>, SourceError> {
        let request = self.request(self.http.get(url.clone()), Some(token))?;
        let response = request
            .send()
            .await
            .map_err(|e| request_failed(self.address(), e))?;
        match response.status() {
            StatusCode::NOT_FOUND | StatusCode::NO_CONTENT => Ok(None),
            StatusCode::SERVICE_UNAVAILABLE => Err(self.not_ready()),
            s if s.is_success() => {
                let read: KvRead = read_json(response, self.address(), name).await?;
                Ok(read.data.and_then(|d| d.data))
            }
            s => Err(status_failed(self.address(), name, s)),
        }
    }

    fn not_ready(&self) -> SourceError {
        SourceError::Unreachable(format!(
            "{} is sealed or not ready (HTTP 503)",
            self.address()
        ))
    }

    /// Logs in with `password` at `url`. A refusal (400, 401 or 403) is `Denied`.
    async fn log_in(
        &self,
        url: &Url,
        mount: &str,
        password: &str,
        creds: [u8; 32],
    ) -> Result<CachedToken, SourceError> {
        let body = serde_json::to_vec(&LoginBody { password }).expect("a login body serializes");
        let request = self.request(self.http.post(url.clone()), None)?;
        let response = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| request_failed(self.address(), e))?;
        let what = format!("the login at auth/{mount}");
        match response.status().as_u16() {
            200..=299 => {}
            // OpenBao and Vault answer HTTP 400 both for a wrong password and when the directory
            // behind an ldap mount cannot be reached or bound. Denied wipes the org's offline
            // copy, so a 400 is a refusal only when its `errors` say so (one of the texts in
            // `LOGIN_REFUSED`); any other 400 is trouble on the server's side, `Unreachable`.
            400 => {
                let code = response.status().as_u16();
                let errors: Option<LoginErrors> =
                    read_json(response, self.address(), &what).await.ok();
                let refused = errors.is_some_and(|e| {
                    e.errors.unwrap_or_default().iter().any(|text| {
                        let text = text.to_lowercase();
                        LOGIN_REFUSED.iter().any(|known| text.contains(known))
                    })
                });
                return Err(if refused {
                    SourceError::Denied(format!(
                        "{} refused the username or password (HTTP {code})",
                        self.address()
                    ))
                } else {
                    SourceError::Unreachable(format!(
                        "{} is not available (HTTP {code})",
                        self.address()
                    ))
                });
            }
            401 | 403 => {
                return Err(SourceError::Denied(format!(
                    "{} refused the username or password (HTTP {})",
                    self.address(),
                    response.status().as_u16()
                )))
            }
            404 => {
                return Err(SourceError::Invalid(format!(
                    "{} has no login at auth/{mount}",
                    self.address()
                )))
            }
            503 => return Err(self.not_ready()),
            _ => return Err(status_failed(self.address(), &what, response.status())),
        }
        let answer: LoginAnswer = read_json(response, self.address(), &what).await?;
        let auth = answer.auth.unwrap_or(LoginAuth {
            client_token: None,
            lease_duration: None,
        });
        let token = auth.client_token.filter(|t| !t.is_empty()).ok_or_else(|| {
            SourceError::Invalid(format!("{} gave no token for {what}", self.address()))
        })?;
        let expires = match auth.lease_duration.unwrap_or(0) {
            0 => None,
            seconds => {
                // The lease is the server's number: cap it, and never overflow an `Instant`.
                let lease = Duration::from_secs(seconds.min(MAX_LEASE_SECONDS));
                Instant::now()
                    .checked_add(lease)
                    .or_else(|| Instant::now().checked_add(Duration::from_secs(3600)))
            }
        };
        Ok(CachedToken {
            token,
            creds,
            expires,
        })
    }

    /// A fetch with a token from the creds.
    async fn fetch_with_token(&self, reads: &Reads, token: &str) -> Result<Fetched, SourceError> {
        let fetched = self.read(reads, token).await?;
        self.renew_if_due(token).await;
        Ok(fetched)
    }

    /// A fetch with a username and password. The token of the last login is used while it is
    /// good; if the source refuses it, the fetch logs in once more and tries again.
    async fn fetch_with_login(
        &self,
        reads: &Reads,
        mount: &str,
        username: &str,
        password: &str,
    ) -> Result<Fetched, SourceError> {
        let url = self.login_url(mount, username)?;
        let creds = creds_hash(username, password);
        let mut cached = self.login.lock().await;
        let reused = match cached.as_ref() {
            Some(c) if c.usable(&creds) => Some(c.token.clone()),
            _ => None,
        };
        let token = match reused {
            Some(token) => {
                match self.read(reads, &token).await {
                    Err(SourceError::Denied(_)) => {}
                    done => return done,
                }
                *cached = None;
                tracing::debug!(
                    "{}: the login token was refused; logging in again",
                    self.address()
                );
                self.log_in(&url, mount, password, creds).await?
            }
            None => {
                *cached = None;
                self.log_in(&url, mount, password, creds).await?
            }
        };
        let fetched = self.read(reads, &token.token).await;
        if !matches!(fetched, Err(SourceError::Denied(_))) {
            *cached = Some(token);
        }
        fetched
    }

    /// Checks the token once an hour, and renews it if it is renewable. A failure is logged,
    /// never an error: the next read will say if the token stopped working.
    async fn renew_if_due(&self, token: &str) {
        let id: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        {
            let mut renewed = self.renewed.lock().await;
            if let Some((at, last)) = *renewed {
                if last == id && at.elapsed() < RENEW_EVERY {
                    return;
                }
            }
            *renewed = Some((Instant::now(), id));
        }
        match self.renewable(token).await {
            Ok(false) => {}
            Ok(true) => {
                if let Err(e) = self.renew(token).await {
                    tracing::warn!("renewing the OpenBao token failed: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("looking up the OpenBao token failed: {e}");
            }
        }
    }

    async fn renewable(&self, token: &str) -> Result<bool, SourceError> {
        let what = "auth/token/lookup-self";
        let url = endpoint(self.address(), ["v1", "auth", "token", "lookup-self"])?;
        let response = self
            .request(self.http.get(url), Some(token))?
            .send()
            .await
            .map_err(|e| request_failed(self.address(), e))?;
        if !response.status().is_success() {
            return Err(status_failed(self.address(), what, response.status()));
        }
        let answer: LookupAnswer = read_json(response, self.address(), what).await?;
        Ok(answer.data.and_then(|d| d.renewable).unwrap_or(false))
    }

    async fn renew(&self, token: &str) -> Result<(), SourceError> {
        let url = endpoint(self.address(), ["v1", "auth", "token", "renew-self"])?;
        let response = self
            .request(self.http.post(url), Some(token))?
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .map_err(|e| request_failed(self.address(), e))?;
        if !response.status().is_success() {
            return Err(status_failed(
                self.address(),
                "auth/token/renew-self",
                response.status(),
            ));
        }
        Ok(())
    }
}

/// The SHA-256 of a username and password, to tell whether a kept token is theirs.
fn creds_hash(username: &str, password: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(username.as_bytes());
    hash.update([0]);
    hash.update(password.as_bytes());
    hash.finalize().into()
}

#[async_trait]
impl Source for OpenBao {
    async fn fetch(&self, creds: &SourceCreds) -> Result<Fetched, SourceError> {
        let reads = self.reads()?;
        let auth = self.settings.auth;
        match (auth, creds) {
            (BaoAuth::Token, SourceCreds::Token { token }) if !token.is_empty() => {
                self.fetch_with_token(&reads, token).await
            }
            (BaoAuth::Token, _) => Err(SourceError::Invalid(
                "this org's OpenBao signs in with a token, so it needs a token".into(),
            )),
            (_, SourceCreds::Login { username, password })
                if !username.is_empty() && !password.is_empty() =>
            {
                let mount = self.settings.login_mount().unwrap_or(auth.as_str());
                self.fetch_with_login(&reads, mount, username, password)
                    .await
            }
            (_, _) => Err(SourceError::Invalid(format!(
                "this org's OpenBao signs in with {}, so it needs a username and password",
                auth.as_str()
            ))),
        }
    }
}

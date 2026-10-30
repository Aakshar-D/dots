//! Infisical, self-hosted or cloud (env vault spec §6.1). The entries are the shared secrets of
//! one folder, `secret_path` in the join code's project and environment, and the signed policy
//! is secret `DOTS_POLICY` of its `policy` sub-folder (§6.3). The person signs in with a
//! Universal Auth machine identity. The access token a login gives is kept in memory until it
//! expires, and a login again replaces it: there is no renewal.
//!
//! Only the folder's own secrets are read. Imported secrets come from other folders and
//! environments an admin controls, so `includeImports=false` is sent, and an `imports` list in
//! the answer is ignored all the same: v1 keeps an org's entries to one folder. A personal
//! override (`type: personal`) is skipped, since an org's entries are the shared ones only.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use super::{
    collect_entries, endpoint, is_dot_segment, read_json, request_failed, roster_text,
    status_failed, Fetched, Field, Source, SourceCreds, SourceError, POLICY_ENTRY, ROSTER_ENTRY,
};
use crate::vault::entries::check_variable_name;
use crate::vault::org::InfisicalSettings;

/// An access token this close to its expiry is not used: the next fetch logs in again.
const EXPIRY_MARGIN: Duration = Duration::from_secs(10);

/// What the login is called in messages.
const LOGIN: &str = "the Universal Auth login";

/// An org's Infisical source. It keeps, in memory only, the access token of the last login.
pub struct Infisical {
    settings: InfisicalSettings,
    http: reqwest::Client,
    /// `site_url` without a slash at the end, for URLs and messages.
    site: String,
    login: Mutex<Option<CachedToken>>,
}

struct CachedToken {
    token: Zeroizing<String>,
    /// The SHA-256 of the client id and client secret the token was given for.
    creds: [u8; 32],
    /// `None` when the login gave no lifetime.
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

/// The two reads of a fetch, and the names they go by in messages: `dev:/dots`.
struct Reads {
    env: Url,
    env_name: String,
    policy: Url,
    policy_name: String,
}

#[derive(Serialize)]
struct LoginBody<'a> {
    #[serde(rename = "clientId")]
    client_id: &'a str,
    #[serde(rename = "clientSecret")]
    client_secret: &'a str,
}

#[derive(Deserialize)]
struct LoginAnswer {
    #[serde(rename = "accessToken", default)]
    access_token: Option<Zeroizing<String>>,
    /// Seconds until the token expires.
    #[serde(rename = "expiresIn", default)]
    expires_in: Option<u64>,
}

/// The answer of `GET /api/v4/secrets`. Every other field, `imports` included, is skipped
/// without being copied.
#[derive(Deserialize)]
struct ListAnswer {
    #[serde(default)]
    secrets: Option<Vec<RawSecret>>,
}

#[derive(Deserialize)]
struct RawSecret {
    #[serde(rename = "secretKey", default)]
    key: Option<String>,
    #[serde(rename = "secretValue", default)]
    value: Option<Field>,
    /// True when the identity may list the secret but not read its value.
    #[serde(rename = "secretValueHidden", default)]
    hidden: Option<bool>,
    /// `shared` or `personal`. A secret without one is taken as shared.
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

impl RawSecret {
    fn is_personal(&self) -> bool {
        self.kind
            .as_deref()
            .is_some_and(|k| k.eq_ignore_ascii_case("personal"))
    }
}

impl Infisical {
    pub fn new(settings: InfisicalSettings, http: reqwest::Client) -> Infisical {
        let site = settings.site_url.trim_end_matches('/').to_string();
        Infisical {
            settings,
            http,
            site,
            login: Mutex::new(None),
        }
    }

    /// The URLs of the two reads, checked before any request is sent.
    fn reads(&self) -> Result<Reads, SourceError> {
        let folder = folder_path(&self.settings.secret_path)?;
        let policy = policy_folder(&folder);
        let environment = &self.settings.environment;
        Ok(Reads {
            env: self.list_url(&folder)?,
            env_name: format!("{environment}:{folder}"),
            policy: self.list_url(&policy)?,
            policy_name: format!("{environment}:{policy}"),
        })
    }

    /// `GET /api/v4/secrets` for one folder, without its sub-folders or imports.
    fn list_url(&self, folder: &str) -> Result<Url, SourceError> {
        let mut url = endpoint(&self.site, ["api", "v4", "secrets"])?;
        url.query_pairs_mut()
            .append_pair("projectId", &self.settings.project_id)
            .append_pair("environment", &self.settings.environment)
            .append_pair("secretPath", folder)
            .append_pair("recursive", "false")
            .append_pair("includeImports", "false")
            .append_pair("viewSecretValue", "true");
        Ok(url)
    }

    /// Logs in with the machine identity. A refusal (401 or 403) is `Denied`.
    async fn log_in(
        &self,
        url: &Url,
        client_id: &str,
        client_secret: &str,
        creds: [u8; 32],
    ) -> Result<CachedToken, SourceError> {
        let body = serde_json::to_vec(&LoginBody {
            client_id,
            client_secret,
        })
        .expect("a login body serializes");
        let response = self
            .http
            .post(url.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| request_failed(&self.site, e))?;
        let status = response.status();
        if matches!(status.as_u16(), 401 | 403) {
            return Err(SourceError::Denied(format!(
                "{} refused the client id or client secret (HTTP {})",
                self.site,
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(status_failed(&self.site, LOGIN, status));
        }
        let answer: LoginAnswer = read_json(response, &self.site, LOGIN).await?;
        let token = answer
            .access_token
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                SourceError::Invalid(format!("{} gave no access token for {LOGIN}", self.site))
            })?;
        let expires = match answer.expires_in.unwrap_or(0) {
            0 => None,
            seconds => Instant::now().checked_add(Duration::from_secs(seconds)),
        };
        Ok(CachedToken {
            token,
            creds,
            expires,
        })
    }

    /// Reads the entries, then the policy and the roster if there is one, with `token`.
    async fn read(&self, reads: &Reads, token: &str) -> Result<Fetched, SourceError> {
        let Some(secrets) = self.list(&reads.env, &reads.env_name, token).await? else {
            return Err(SourceError::Invalid(format!(
                "no secrets at {}: the project, environment or folder does not exist",
                reads.env_name
            )));
        };
        let entries = shared_entries(secrets, &reads.env_name)?;

        // The roster sits beside the policy, so one listing holds both.
        let mut beside = self
            .list(&reads.policy, &reads.policy_name, token)
            .await?
            .unwrap_or_default();
        beside.retain(|s| !s.is_personal());
        let roster = match beside
            .iter()
            .position(|s| s.key.as_deref() == Some(ROSTER_ENTRY))
        {
            None => None,
            Some(at) => {
                let field = match (beside[at].hidden, beside[at].value.take()) {
                    (Some(true), _) => return Err(hidden(ROSTER_ENTRY)),
                    (_, Some(field)) => field,
                    (_, None) => return Err(no_value(ROSTER_ENTRY)),
                };
                Some(roster_text(field)?)
            }
        };
        let policy = beside
            .into_iter()
            .find(|s| s.key.as_deref() == Some(POLICY_ENTRY))
            .ok_or_else(|| {
                SourceError::Invalid(format!("no {POLICY_ENTRY} at {}", reads.policy_name))
            })?;
        let mut policy = match (policy.hidden, policy.value) {
            (Some(true), _) => return Err(hidden(POLICY_ENTRY)),
            (_, None) => return Err(no_value(POLICY_ENTRY)),
            (_, Some(Field::NotText)) => return Err(not_text(POLICY_ENTRY)),
            (_, Some(Field::Text(text))) => text,
        };
        Ok(Fetched {
            entries,
            policy: std::mem::take(&mut *policy),
            roster,
            roster_note: None,
        })
    }

    /// One folder's secrets, or `None` when the folder is not there (404).
    async fn list(
        &self,
        url: &Url,
        name: &str,
        token: &str,
    ) -> Result<Option<Vec<RawSecret>>, SourceError> {
        let bearer = Zeroizing::new(format!("Bearer {token}"));
        let mut bearer = HeaderValue::from_str(&bearer).map_err(|_| {
            SourceError::Invalid(format!(
                "the access token from {} holds characters a header cannot carry",
                self.site
            ))
        })?;
        bearer.set_sensitive(true);
        let response = self
            .http
            .get(url.clone())
            .header(AUTHORIZATION, bearer)
            .send()
            .await
            .map_err(|e| request_failed(&self.site, e))?;
        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => {
                let answer: ListAnswer = read_json(response, &self.site, name).await?;
                let secrets = answer.secrets.ok_or_else(|| {
                    SourceError::Invalid(format!(
                        "the answer from {} for {name} has no list of secrets",
                        self.site
                    ))
                })?;
                Ok(Some(secrets))
            }
            s => Err(status_failed(&self.site, name, s)),
        }
    }

    /// A fetch with the machine identity. The token of the last login is used while it is
    /// good; if the source refuses it, the fetch logs in once more and tries again.
    async fn fetch_with(
        &self,
        reads: &Reads,
        client_id: &str,
        client_secret: &str,
    ) -> Result<Fetched, SourceError> {
        let url = endpoint(&self.site, ["api", "v1", "auth", "universal-auth", "login"])?;
        let creds = creds_hash(client_id, client_secret);
        let mut cached = self.login.lock().await;
        let reused = match cached.as_ref() {
            Some(c) if c.usable(&creds) => Some(c.token.clone()),
            _ => None,
        };
        if let Some(token) = reused {
            match self.read(reads, &token).await {
                Err(SourceError::Denied(_)) => {}
                done => return done,
            }
            tracing::debug!(
                "{}: the access token was refused; logging in again",
                self.site
            );
        }
        *cached = None;
        let token = self.log_in(&url, client_id, client_secret, creds).await?;
        let fetched = self.read(reads, &token.token).await;
        if !matches!(fetched, Err(SourceError::Denied(_))) {
            *cached = Some(token);
        }
        fetched
    }
}

/// `secret_path` as Infisical names a folder: `/`, or `/team/dots` with no slash at the end. It
/// must start with `/`, and an empty segment (`/a//b`), a `.` or `..` segment (also written with
/// `%2e`) or a control character is refused. One slash at the end is dropped.
fn folder_path(value: &str) -> Result<String, SourceError> {
    let refuse = |why: &str| {
        SourceError::Invalid(format!(
            "the source's secret_path '{}' {why}",
            value.escape_debug()
        ))
    };
    if value.chars().any(char::is_control) {
        return Err(refuse("holds a control character"));
    }
    if value == "/" {
        return Ok(value.to_string());
    }
    let Some(rest) = value.strip_prefix('/') else {
        return Err(refuse("must start with '/'"));
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(refuse("has an empty segment"));
    }
    if segments.iter().any(|s| is_dot_segment(s)) {
        return Err(refuse("must not hold a '.' or '..' segment"));
    }
    Ok(format!("/{}", segments.join("/")))
}

/// The policy's sub-folder of `folder`: `/policy` under the root, `/dots/policy` under `/dots`.
fn policy_folder(folder: &str) -> String {
    match folder {
        "/" => "/policy".to_string(),
        _ => format!("{folder}/policy"),
    }
}

/// The entries of a folder's shared secrets.
fn shared_entries(
    secrets: Vec<RawSecret>,
    name: &str,
) -> Result<BTreeMap<String, Zeroizing<String>>, SourceError> {
    let mut raw = BTreeMap::new();
    for secret in secrets {
        if secret.is_personal() {
            continue;
        }
        let Some(key) = secret.key else {
            return Err(SourceError::Invalid(format!(
                "a secret at {name} has no name"
            )));
        };
        // `collect_entries` skips these names whatever their value, and no message names one: a
        // name that is not a variable name may be a value pasted in the wrong place.
        if skipped(&key) {
            raw.insert(key, Field::NotText);
            continue;
        }
        let field = match (secret.hidden, secret.value) {
            (Some(true), _) => return Err(hidden(&key)),
            (_, None) => return Err(no_value(&key)),
            (_, Some(field)) => field,
        };
        if raw.contains_key(&key) {
            return Err(SourceError::Invalid(format!("{key} is at {name} twice")));
        }
        raw.insert(key, field);
    }
    collect_entries(raw)
}

/// Whether `collect_entries` skips `name`: the policy or roster key, or not a variable name.
fn skipped(name: &str) -> bool {
    name.eq_ignore_ascii_case(POLICY_ENTRY)
        || name.eq_ignore_ascii_case(ROSTER_ENTRY)
        || check_variable_name(name).is_err()
}

fn hidden(name: &str) -> SourceError {
    SourceError::Invalid(format!(
        "the value of {name} is hidden from this machine identity"
    ))
}

fn no_value(name: &str) -> SourceError {
    SourceError::Invalid(format!("{name} has no value"))
}

fn not_text(name: &str) -> SourceError {
    SourceError::Invalid(format!("{name} is not text"))
}

/// The SHA-256 of a client id and secret, to tell whether a kept token is theirs.
fn creds_hash(client_id: &str, client_secret: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(client_id.as_bytes());
    hash.update([0]);
    hash.update(client_secret.as_bytes());
    hash.finalize().into()
}

#[async_trait]
impl Source for Infisical {
    async fn fetch(&self, creds: &SourceCreds) -> Result<Fetched, SourceError> {
        let reads = self.reads()?;
        match creds {
            SourceCreds::Machine {
                client_id,
                client_secret,
            } if !client_id.is_empty() && !client_secret.is_empty() => {
                self.fetch_with(&reads, client_id, client_secret).await
            }
            _ => Err(SourceError::Invalid(
                "this org's Infisical signs in with a machine identity, so it needs a client id \
                 and client secret"
                    .into(),
            )),
        }
    }
}

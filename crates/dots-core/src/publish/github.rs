//! The GitHub REST calls the Releases page makes, on `releases::REPO`, with the publisher's
//! token (updates UI spec section 5). The base URL is a parameter so tests use a mock server.

use std::time::Duration;

use base64::Engine;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::releases::REPO;
use crate::{Error, Result};

const TIMEOUT: Duration = Duration::from_secs(30);

/// What a token may do on the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    pub login: String,
    pub push: bool,
    /// A classic token's scopes (`X-OAuth-Scopes`); `None` for a fine-grained token.
    pub scopes: Option<Vec<String>>,
}

impl Access {
    /// Why this token cannot publish, or `None` when it can.
    pub fn publisher_problem(&self) -> Option<&'static str> {
        if !self.push {
            return Some("This token cannot push to Aakshar-D/dots.");
        }
        match &self.scopes {
            Some(scopes)
                if !(scopes.iter().any(|s| s == "repo")
                    && scopes.iter().any(|s| s == "workflow")) =>
            {
                Some("This token needs the repo and workflow scopes.")
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged(String),
    AlreadyMerged,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedPull {
    pub number: u64,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub id: u64,
    pub tag: String,
    pub name: String,
    pub body: String,
    pub prerelease: bool,
    pub draft: bool,
    pub url: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    /// Each step's name, status and conclusion.
    pub steps: Vec<(String, String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    pub id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: String,
    pub jobs: Vec<JobInfo>,
}

/// A client for one token. Debug never prints the token.
pub struct GitHub {
    api: String,
    token: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for GitHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHub").field("api", &self.api).finish()
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

impl GitHub {
    pub fn new(api: &str, token: &str) -> Result<GitHub> {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("dots/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(GitHub {
            api: api.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client,
        })
    }

    fn scrubbed(&self, text: String) -> Error {
        Error::Other(super::scrub(&text, &[&self.token]))
    }

    /// One call. `ok`: statuses that are answers, not errors (2xx always are).
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        ok: &[u16],
    ) -> Result<(StatusCode, reqwest::header::HeaderMap, Value)> {
        let url = format!("{}{path}", self.api);
        let mut request = self
            .client
            .request(method, &url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| self.scrubbed(format!("GitHub could not be reached: {e}")))?;
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_success() || ok.contains(&status.as_u16()) {
            return Ok((status, headers, value));
        }
        let message = value["message"]
            .as_str()
            .unwrap_or(text.as_str())
            .to_string();
        Err(self.scrubbed(format!("GitHub answered {}: {message}", status.as_u16())))
    }

    async fn get(&self, path: &str) -> Result<Value> {
        Ok(self.call(Method::GET, path, None, &[]).await?.2)
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        Ok(self.call(Method::POST, path, Some(body), &[]).await?.2)
    }

    pub async fn access(&self) -> Result<Access> {
        let (_, headers, user) = self.call(Method::GET, "/user", None, &[]).await?;
        let scopes = headers
            .get("X-OAuth-Scopes")
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            });
        let repo = self.get(&format!("/repos/{REPO}")).await?;
        Ok(Access {
            login: str_of(&user, "login"),
            push: repo["permissions"]["push"].as_bool().unwrap_or(false),
            scopes,
        })
    }

    pub async fn secret_exists(&self, name: &str) -> Result<bool> {
        let path = format!("/repos/{REPO}/actions/secrets/{name}");
        let (status, _, _) = self.call(Method::GET, &path, None, &[404]).await?;
        Ok(status != StatusCode::NOT_FOUND)
    }

    /// The repository's public key for secrets: its id and the key (base64).
    pub async fn secrets_key(&self) -> Result<(String, String)> {
        let v = self
            .get(&format!("/repos/{REPO}/actions/secrets/public-key"))
            .await?;
        Ok((str_of(&v, "key_id"), str_of(&v, "key")))
    }

    /// Stores a secret that does not exist yet; never replaces one.
    pub async fn put_secret(&self, name: &str, key_id: &str, encrypted_b64: &str) -> Result<()> {
        if self.secret_exists(name).await? {
            return Err(Error::Other(format!("{name} already exists on GitHub.")));
        }
        let path = format!("/repos/{REPO}/actions/secrets/{name}");
        let body = json!({"encrypted_value": encrypted_b64, "key_id": key_id});
        self.call(Method::PUT, &path, Some(body), &[]).await?;
        Ok(())
    }

    /// A file's text at `reference`, `None` when it is not there.
    pub async fn file(&self, path: &str, reference: &str) -> Result<Option<String>> {
        let url = format!("/repos/{REPO}/contents/{path}?ref={reference}");
        let (status, _, v) = self.call(Method::GET, &url, None, &[404]).await?;
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let encoded: String = str_of(&v, "content").split_whitespace().collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Error::Other(format!("{path} could not be read: {e}")))?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| Error::Other(format!("{path} is not text: {e}")))
    }

    /// The commit a branch points at.
    pub async fn head(&self, branch: &str) -> Result<String> {
        let v = self
            .get(&format!("/repos/{REPO}/git/ref/heads/{branch}"))
            .await?;
        Ok(v["object"]["sha"].as_str().unwrap_or_default().to_string())
    }

    /// One commit on `parent` that writes `files` (path, text); its sha.
    pub async fn commit_files(
        &self,
        parent: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String> {
        let parent_commit = self
            .get(&format!("/repos/{REPO}/git/commits/{parent}"))
            .await?;
        let base_tree = parent_commit["tree"]["sha"].as_str().unwrap_or_default();
        let mut entries = Vec::new();
        for (path, text) in files {
            let blob = self
                .post(
                    &format!("/repos/{REPO}/git/blobs"),
                    json!({"content": text, "encoding": "utf-8"}),
                )
                .await?;
            entries
                .push(json!({"path": path, "mode": "100644", "type": "blob", "sha": blob["sha"]}));
        }
        let tree = self
            .post(
                &format!("/repos/{REPO}/git/trees"),
                json!({"base_tree": base_tree, "tree": entries}),
            )
            .await?;
        let commit = self
            .post(
                &format!("/repos/{REPO}/git/commits"),
                json!({"message": message, "tree": tree["sha"], "parents": [parent]}),
            )
            .await?;
        Ok(str_of(&commit, "sha"))
    }

    /// Creates the branch at `sha`, or moves it there (`force` allows a move that is not a
    /// fast-forward).
    pub async fn set_branch(&self, branch: &str, sha: &str, force: bool) -> Result<()> {
        let path = format!("/repos/{REPO}/git/refs/heads/{branch}");
        let (status, _, _) = self
            .call(
                Method::PATCH,
                &path,
                Some(json!({"sha": sha, "force": force})),
                &[404, 422],
            )
            .await?;
        if status.is_success() {
            return Ok(());
        }
        self.post(
            &format!("/repos/{REPO}/git/refs"),
            json!({"ref": format!("refs/heads/{branch}"), "sha": sha}),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_branch(&self, branch: &str) -> Result<()> {
        let path = format!("/repos/{REPO}/git/refs/heads/{branch}");
        self.call(Method::DELETE, &path, None, &[404, 422]).await?;
        Ok(())
    }

    /// A lightweight tag on `sha`.
    pub async fn tag(&self, tag: &str, sha: &str) -> Result<()> {
        self.post(
            &format!("/repos/{REPO}/git/refs"),
            json!({"ref": format!("refs/tags/{tag}"), "sha": sha}),
        )
        .await?;
        Ok(())
    }

    /// The commit a (lightweight) tag points at.
    pub async fn tag_sha(&self, tag: &str) -> Result<String> {
        let v = self
            .get(&format!("/repos/{REPO}/git/ref/tags/{tag}"))
            .await?;
        Ok(v["object"]["sha"].as_str().unwrap_or_default().to_string())
    }

    pub async fn tag_exists(&self, tag: &str) -> Result<bool> {
        let path = format!("/repos/{REPO}/git/ref/tags/{tag}");
        let (status, _, _) = self.call(Method::GET, &path, None, &[404]).await?;
        Ok(status != StatusCode::NOT_FOUND)
    }

    /// Every tag's name.
    pub async fn tags(&self) -> Result<Vec<String>> {
        let v = self
            .get(&format!("/repos/{REPO}/git/matching-refs/tags/v"))
            .await?;
        Ok(v.as_array()
            .map(|refs| {
                refs.iter()
                    .filter_map(|r| r["ref"].as_str())
                    .map(|r| r.trim_start_matches("refs/tags/").to_string())
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn merge(&self, base: &str, head: &str, message: &str) -> Result<MergeOutcome> {
        let (status, _, v) = self
            .call(
                Method::POST,
                &format!("/repos/{REPO}/merges"),
                Some(json!({"base": base, "head": head, "commit_message": message})),
                &[409],
            )
            .await?;
        Ok(match status.as_u16() {
            201 => MergeOutcome::Merged(str_of(&v, "sha")),
            409 => MergeOutcome::Conflict,
            _ => MergeOutcome::AlreadyMerged,
        })
    }

    /// Opens a pull request; its number and page.
    pub async fn open_pull(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)> {
        let v = self
            .post(
                &format!("/repos/{REPO}/pulls"),
                json!({"head": head, "base": base, "title": title, "body": body}),
            )
            .await?;
        Ok((
            v["number"].as_u64().unwrap_or_default(),
            str_of(&v, "html_url"),
        ))
    }

    pub async fn pull_merged(&self, number: u64) -> Result<bool> {
        let path = format!("/repos/{REPO}/pulls/{number}/merge");
        let (status, _, _) = self.call(Method::GET, &path, None, &[404]).await?;
        Ok(status == StatusCode::NO_CONTENT)
    }

    /// The pull requests merged between `base` (a tag or sha) and `head`, newest first, read
    /// from their merge commits. With no base, the 30 newest merged into master.
    pub async fn merged_pulls_since(
        &self,
        base: Option<&str>,
        head: &str,
    ) -> Result<Vec<MergedPull>> {
        let Some(base) = base else {
            let v = self
                .get(&format!(
                    "/repos/{REPO}/pulls?state=closed&base=master&sort=updated&direction=desc&per_page=30"
                ))
                .await?;
            return Ok(v
                .as_array()
                .map(|pulls| {
                    pulls
                        .iter()
                        .filter(|p| !p["merged_at"].is_null())
                        .map(|p| MergedPull {
                            number: p["number"].as_u64().unwrap_or_default(),
                            title: str_of(p, "title"),
                        })
                        .collect()
                })
                .unwrap_or_default());
        };
        let v = self
            .get(&format!("/repos/{REPO}/compare/{base}...{head}"))
            .await?;
        let mut pulls: Vec<MergedPull> = v["commits"]
            .as_array()
            .map(|commits| {
                commits
                    .iter()
                    .filter_map(|c| merged_pull(c["commit"]["message"].as_str()?))
                    .collect()
            })
            .unwrap_or_default();
        pulls.reverse();
        Ok(pulls)
    }

    /// How many commits `branch` has that `base_sha` has not.
    pub async fn commits_ahead(&self, base_sha: &str, branch: &str) -> Result<u32> {
        let v = self
            .get(&format!("/repos/{REPO}/compare/{base_sha}...{branch}"))
            .await?;
        Ok(v["ahead_by"].as_u64().unwrap_or_default() as u32)
    }

    pub async fn releases(&self) -> Result<Vec<ReleaseInfo>> {
        let v = self
            .get(&format!("/repos/{REPO}/releases?per_page=30"))
            .await?;
        Ok(v.as_array()
            .map(|all| {
                all.iter()
                    .map(|r| ReleaseInfo {
                        id: r["id"].as_u64().unwrap_or_default(),
                        tag: str_of(r, "tag_name"),
                        name: str_of(r, "name"),
                        body: str_of(r, "body"),
                        prerelease: r["prerelease"].as_bool().unwrap_or(false),
                        draft: r["draft"].as_bool().unwrap_or(false),
                        url: str_of(r, "html_url"),
                        published_at: r["published_at"].as_str().map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn edit_release(
        &self,
        id: u64,
        name: Option<&str>,
        body: Option<&str>,
        prerelease: Option<bool>,
    ) -> Result<()> {
        let mut patch = serde_json::Map::new();
        if let Some(name) = name {
            patch.insert("name".into(), json!(name));
        }
        if let Some(body) = body {
            patch.insert("body".into(), json!(body));
        }
        if let Some(prerelease) = prerelease {
            patch.insert("prerelease".into(), json!(prerelease));
        }
        let path = format!("/repos/{REPO}/releases/{id}");
        self.call(Method::PATCH, &path, Some(Value::Object(patch)), &[])
            .await?;
        Ok(())
    }

    /// The newest workflow run a push of `tag` started, with its jobs.
    pub async fn run_for_tag(&self, tag: &str) -> Result<Option<RunInfo>> {
        let v = self
            .get(&format!(
                "/repos/{REPO}/actions/runs?event=push&per_page=30"
            ))
            .await?;
        let Some(run) = v["workflow_runs"]
            .as_array()
            .and_then(|runs| runs.iter().find(|r| r["head_branch"].as_str() == Some(tag)))
            .cloned()
        else {
            return Ok(None);
        };
        let id = run["id"].as_u64().unwrap_or_default();
        let jobs = self
            .get(&format!("/repos/{REPO}/actions/runs/{id}/jobs"))
            .await?;
        let jobs = jobs["jobs"]
            .as_array()
            .map(|all| {
                all.iter()
                    .map(|j| JobInfo {
                        name: str_of(j, "name"),
                        status: str_of(j, "status"),
                        conclusion: j["conclusion"].as_str().map(str::to_string),
                        steps: j["steps"]
                            .as_array()
                            .map(|steps| {
                                steps
                                    .iter()
                                    .map(|s| {
                                        (
                                            str_of(s, "name"),
                                            str_of(s, "status"),
                                            s["conclusion"].as_str().map(str::to_string),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(RunInfo {
            id,
            status: str_of(&run, "status"),
            conclusion: run["conclusion"].as_str().map(str::to_string),
            url: str_of(&run, "html_url"),
            jobs,
        }))
    }

    pub async fn rerun(&self, run_id: u64) -> Result<()> {
        self.post(
            &format!("/repos/{REPO}/actions/runs/{run_id}/rerun"),
            json!({}),
        )
        .await?;
        Ok(())
    }
}

/// "Merge pull request #N from …\n\n<title>" as a pull; other messages are not.
fn merged_pull(message: &str) -> Option<MergedPull> {
    let rest = message.strip_prefix("Merge pull request #")?;
    let number: u64 = rest.split_whitespace().next()?.parse().ok()?;
    let title = message
        .split_once("\n\n")
        .map(|(_, t)| t.lines().next().unwrap_or_default().trim().to_string())
        .unwrap_or_default();
    Some(MergedPull { number, title })
}

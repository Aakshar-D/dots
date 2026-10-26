//! GitHub watchers (watchers spec section 4.3): each repository polled once a minute through the
//! REST API with the token in the vault, ETags kept per URL so an unchanged answer costs nothing,
//! and a cursor per repository and event kind. A kind seen for the first time starts from now.
//! Anything by the token's own account never wakes the dot. A rate limit only waits; a refused
//! token or a repository the token cannot see pauses the watcher.
//!
//! Where the REST API decides the shape (this plan's choices):
//! - `pull_request`: open PRs that are not drafts, and `ready_for_review` issue events (a draft
//!   wakes the dot when it is marked ready);
//! - `comment`: issue and PR comments, and PR reviews with a body, read per open PR updated
//!   since the last review handled (there is no repository-wide reviews list);
//! - `push`: a branch head that moved, its commits in the body;
//! - `label`: filters issues and PRs only.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{entry_gone, Failure, Fired, Poll, SourceEvent, Watchers, MAX_BODY_CHARS, POLL_EVERY};
use crate::model::{GithubConfig, GithubEvent, Watcher};
use crate::Result;

/// The most commits a push run lists.
pub const MAX_PUSH_COMMITS: usize = 20;

const TIMEOUT: Duration = Duration::from_secs(30);

pub fn token_refused(secret: &str) -> String {
    format!("GitHub refused the token in {secret}.")
}

pub fn cannot_see(secret: &str, repo: &str) -> String {
    format!("The token in {secret} cannot see {repo}.")
}

pub fn server_error(status: &str, repo: &str) -> String {
    format!("GitHub answered {status} for {repo}.")
}

pub fn unreachable(error: &str) -> String {
    format!("Could not reach GitHub: {error}.")
}

/// Where a GitHub watcher has read up to, per repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubCursor {
    #[serde(default)]
    pub repos: BTreeMap<String, RepoCursor>,
}

/// Where a repository has been read up to, per event kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_since: Option<String>,
    #[serde(default)]
    pub heads: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci: Option<u64>,
}

/// Why a poll ends early.
enum Stop {
    /// The source failed.
    Failed(Failure),
    /// A rate limit: try again then.
    Wait(DateTime<Utc>),
    /// The cap paused the watcher, or it is off.
    Halt,
}

type Step<T> = std::result::Result<T, Stop>;

/// One poll's context.
struct Poller<'a> {
    service: &'a Watchers,
    watcher: &'a Watcher,
    config: &'a GithubConfig,
    client: reqwest::Client,
    token: String,
    own_login: String,
    cursor: GithubCursor,
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn body(v: &Value) -> String {
    v.as_str()
        .unwrap_or_default()
        .chars()
        .take(MAX_BODY_CHARS)
        .collect()
}

fn time(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn ids_above(items: &[Value], after: u64) -> Vec<&Value> {
    let mut out: Vec<&Value> = items
        .iter()
        .filter(|i| i["id"].as_u64().is_some_and(|id| id > after))
        .collect();
    out.sort_by_key(|i| i["id"].as_u64());
    out
}

fn newest_id(items: &[Value]) -> u64 {
    items
        .iter()
        .filter_map(|i| i["id"].as_u64())
        .max()
        .unwrap_or(0)
}

fn transient_unreachable(e: reqwest::Error) -> Stop {
    Stop::Failed(Failure::Transient(unreachable(
        &e.without_url().to_string(),
    )))
}

impl Poller<'_> {
    fn repo_cursor(&mut self, repo: &str) -> &mut RepoCursor {
        self.cursor.repos.entry(repo.to_string()).or_default()
    }

    async fn save(&self) -> Result<()> {
        self.service
            .deps
            .store
            .set_watcher_cursor(&self.watcher.id, Some(&serde_json::to_value(&self.cursor)?))
            .await
    }

    /// `GET <api><path>`: the JSON, the cached one on a `304`, or why the poll stops.
    async fn get(&self, path: &str, repo: &str) -> Step<Value> {
        let api = self.service.deps.sources.github_api.trim_end_matches('/');
        let url = format!("{api}{path}");
        let key = (self.watcher.id.clone(), url.clone());
        let cached = self.service.etags.lock().unwrap().get(&key).cloned();
        let mut request = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "Dots")
            .bearer_auth(&self.token);
        if let Some((etag, _)) = &cached {
            request = request.header("If-None-Match", etag);
        }
        let answer = request.send().await.map_err(transient_unreachable)?;
        let status = answer.status();
        let header = |name: &str| {
            answer
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let secret = &self.config.secret;
        if status == StatusCode::NOT_MODIFIED {
            return Ok(cached.map(|(_, v)| v).unwrap_or(Value::Null));
        }
        if status.is_success() {
            let etag = header("etag");
            let json: Value = answer.json().await.map_err(transient_unreachable)?;
            if let Some(etag) = etag {
                self.service
                    .etags
                    .lock()
                    .unwrap()
                    .insert(key, (etag, json.clone()));
            }
            return Ok(json);
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(Stop::Failed(Failure::Permanent(token_refused(secret))));
        }
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            let now = self.service.deps.clock.now();
            if header("x-ratelimit-remaining").as_deref() == Some("0") {
                let reset = header("x-ratelimit-reset")
                    .and_then(|r| r.parse::<i64>().ok())
                    .and_then(|r| DateTime::<Utc>::from_timestamp(r, 0))
                    .unwrap_or(now + POLL_EVERY);
                return Err(Stop::Wait(reset));
            }
            if let Some(secs) = header("retry-after").and_then(|r| r.parse::<i64>().ok()) {
                return Err(Stop::Wait(now + TimeDelta::seconds(secs)));
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(Stop::Wait(now + POLL_EVERY));
            }
            return Err(Stop::Failed(Failure::Permanent(cannot_see(secret, repo))));
        }
        if status == StatusCode::NOT_FOUND {
            return Err(Stop::Failed(Failure::Permanent(cannot_see(secret, repo))));
        }
        Err(Stop::Failed(Failure::Transient(server_error(
            status.as_str(),
            repo,
        ))))
    }

    async fn list(&self, path: &str, repo: &str) -> Step<Vec<Value>> {
        Ok(self
            .get(path, repo)
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// Whether an event by `author` (a GitHub user) with `labels` passes the watcher's filters.
    fn passes(&self, author: &Value, labels: Option<&Value>) -> bool {
        let login = author["login"].as_str().unwrap_or_default();
        if login.eq_ignore_ascii_case(&self.own_login) {
            return false;
        }
        if self.config.skip_bots && (author["type"] == "Bot" || login.ends_with("[bot]")) {
            return false;
        }
        if !self.config.authors.is_empty()
            && !self
                .config
                .authors
                .iter()
                .any(|a| a.eq_ignore_ascii_case(login))
        {
            return false;
        }
        if let (Some(label), Some(labels)) = (&self.config.label, labels) {
            let carries = labels.as_array().is_some_and(|ls| {
                ls.iter().any(|l| {
                    l["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(label))
                })
            });
            if !carries {
                return false;
            }
        }
        true
    }

    /// Fires one event; the cursor is saved first, so a stop keeps what was handled.
    async fn fire(&mut self, payload: Value) -> Step<()> {
        if let Err(e) = self.save().await {
            tracing::warn!(watcher = %self.watcher.id, "saving a GitHub cursor failed: {e}");
            return Err(Stop::Halt);
        }
        match self
            .service
            .fire(&self.watcher.id, SourceEvent::new(payload))
            .await
        {
            Ok(Fired::Run(_)) => Ok(()),
            Ok(Fired::Capped) | Ok(Fired::Off) => Err(Stop::Halt),
            Err(e) => {
                tracing::warn!(watcher = %self.watcher.id, "a GitHub event was lost: {e}");
                Err(Stop::Halt)
            }
        }
    }

    fn item_payload(&self, repo: &str, event: &str, item: &Value, author: &Value) -> Value {
        json!({
            "source": "github",
            "repo": repo,
            "event": event,
            "title": text(&item["title"]),
            "body": body(&item["body"]),
            "url": text(&item["html_url"]),
            "author": text(&author["login"]),
            "number": item["number"],
        })
    }

    async fn issues(&mut self, repo: &str) -> Step<()> {
        let path =
            format!("/repos/{repo}/issues?state=all&sort=created&direction=desc&per_page=50");
        let items: Vec<Value> = self
            .list(&path, repo)
            .await?
            .into_iter()
            .filter(|i| i.get("pull_request").is_none())
            .collect();
        let Some(after) = self.repo_cursor(repo).issue else {
            self.repo_cursor(repo).issue = Some(newest_id(&items));
            return Ok(());
        };
        for item in ids_above(&items, after) {
            self.repo_cursor(repo).issue = item["id"].as_u64();
            if self.passes(&item["user"], Some(&item["labels"])) {
                let payload = self.item_payload(repo, "issue", item, &item["user"]);
                self.fire(payload).await?;
            }
        }
        Ok(())
    }

    async fn pulls(&mut self, repo: &str) -> Step<()> {
        let path =
            format!("/repos/{repo}/pulls?state=open&sort=created&direction=desc&per_page=50");
        let pulls = self.list(&path, repo).await?;
        match self.repo_cursor(repo).pull {
            None => self.repo_cursor(repo).pull = Some(newest_id(&pulls)),
            Some(after) => {
                for pr in ids_above(&pulls, after) {
                    self.repo_cursor(repo).pull = pr["id"].as_u64();
                    if pr["draft"] != true && self.passes(&pr["user"], Some(&pr["labels"])) {
                        let payload = self.item_payload(repo, "pull_request", pr, &pr["user"]);
                        self.fire(payload).await?;
                    }
                }
            }
        }
        let events = self
            .list(&format!("/repos/{repo}/issues/events?per_page=100"), repo)
            .await?;
        let ready: Vec<Value> = events
            .into_iter()
            .filter(|e| e["event"] == "ready_for_review")
            .collect();
        let Some(after) = self.repo_cursor(repo).ready else {
            self.repo_cursor(repo).ready = Some(newest_id(&ready));
            return Ok(());
        };
        for event in ids_above(&ready, after) {
            self.repo_cursor(repo).ready = event["id"].as_u64();
            let pr = &event["issue"];
            if self.passes(&event["actor"], Some(&pr["labels"])) {
                let payload = self.item_payload(repo, "pull_request", pr, &event["actor"]);
                self.fire(payload).await?;
            }
        }
        Ok(())
    }

    async fn comments(&mut self, repo: &str) -> Step<()> {
        let path = format!("/repos/{repo}/issues/comments?sort=created&direction=desc&per_page=50");
        let comments = self.list(&path, repo).await?;
        match self.repo_cursor(repo).comment {
            None => self.repo_cursor(repo).comment = Some(newest_id(&comments)),
            Some(after) => {
                for comment in ids_above(&comments, after) {
                    self.repo_cursor(repo).comment = comment["id"].as_u64();
                    if !self.passes(&comment["user"], None) {
                        continue;
                    }
                    let number = comment["issue_url"]
                        .as_str()
                        .and_then(|u| u.rsplit('/').next())
                        .and_then(|n| n.parse::<u64>().ok());
                    let Some(number) = number else {
                        continue;
                    };
                    let issue = self
                        .get(&format!("/repos/{repo}/issues/{number}"), repo)
                        .await?;
                    let payload = json!({
                        "source": "github",
                        "repo": repo,
                        "event": "comment",
                        "title": text(&issue["title"]),
                        "body": body(&comment["body"]),
                        "url": text(&comment["html_url"]),
                        "author": text(&comment["user"]["login"]),
                        "number": number,
                    });
                    self.fire(payload).await?;
                }
            }
        }
        self.reviews(repo).await
    }

    async fn reviews(&mut self, repo: &str) -> Step<()> {
        let since = self.repo_cursor(repo).review_since.clone();
        let Some(since) = since.as_deref().and_then(|s| time(&json!(s))) else {
            let now = self.service.deps.clock.now();
            self.repo_cursor(repo).review_since = Some(crate::util::timestamp(now));
            return Ok(());
        };
        let path =
            format!("/repos/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=20");
        let pulls = self.list(&path, repo).await?;
        let mut found: Vec<(DateTime<Utc>, Value, Value)> = Vec::new();
        for pr in pulls {
            if !time(&pr["updated_at"]).is_some_and(|t| t > since) {
                continue;
            }
            let Some(number) = pr["number"].as_u64() else {
                continue;
            };
            let reviews = self
                .list(&format!("/repos/{repo}/pulls/{number}/reviews"), repo)
                .await?;
            for review in reviews {
                if let Some(at) = time(&review["submitted_at"]).filter(|t| *t > since) {
                    found.push((at, review, pr.clone()));
                }
            }
        }
        found.sort_by_key(|(at, _, _)| *at);
        for (at, review, pr) in found {
            self.repo_cursor(repo).review_since = Some(crate::util::timestamp(at));
            let said = review["body"].as_str().unwrap_or_default();
            if said.trim().is_empty() || !self.passes(&review["user"], None) {
                continue;
            }
            let payload = json!({
                "source": "github",
                "repo": repo,
                "event": "comment",
                "title": text(&pr["title"]),
                "body": body(&review["body"]),
                "url": text(&review["html_url"]),
                "author": text(&review["user"]["login"]),
                "number": pr["number"],
            });
            self.fire(payload).await?;
        }
        Ok(())
    }

    async fn pushes(&mut self, repo: &str) -> Step<()> {
        let branches = if self.config.branches.is_empty() {
            let info = self.get(&format!("/repos/{repo}"), repo).await?;
            vec![text(&info["default_branch"])]
        } else {
            self.config.branches.clone()
        };
        for branch in branches.into_iter().filter(|b| !b.is_empty()) {
            let info = self
                .get(&format!("/repos/{repo}/branches/{branch}"), repo)
                .await?;
            let head = text(&info["commit"]["sha"]);
            let old = self.repo_cursor(repo).heads.get(&branch).cloned();
            self.repo_cursor(repo)
                .heads
                .insert(branch.clone(), head.clone());
            let Some(old) = old.filter(|o| *o != head) else {
                continue;
            };
            let compare = self
                .get(&format!("/repos/{repo}/compare/{old}...{head}"), repo)
                .await?;
            let commits = compare["commits"].as_array().cloned().unwrap_or_default();
            let Some(last) = commits.last() else {
                continue;
            };
            let author = last["author"].clone();
            if !self.passes(&author, None) {
                continue;
            }
            let n = commits.len();
            let mut lines = Vec::new();
            if n > MAX_PUSH_COMMITS {
                lines.push(format!(
                    "({n} commits; the newest {MAX_PUSH_COMMITS} listed)"
                ));
            }
            for commit in &commits[n.saturating_sub(MAX_PUSH_COMMITS)..] {
                let sha: String = text(&commit["sha"]).chars().take(7).collect();
                let message = text(&commit["commit"]["message"]);
                let first = message.lines().next().unwrap_or_default();
                let login = commit["author"]["login"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| text(&commit["commit"]["author"]["name"]));
                lines.push(format!("- {sha} {first} ({login})"));
            }
            let title = if n == 1 {
                "1 commit".to_string()
            } else {
                format!("{n} commits")
            };
            let payload = json!({
                "source": "github",
                "repo": repo,
                "event": "push",
                "title": title,
                "body": body(&json!(lines.join("\n"))),
                "url": text(&compare["html_url"]),
                "author": text(&author["login"]),
                "branch": branch,
            });
            self.fire(payload).await?;
        }
        Ok(())
    }

    async fn ci(&mut self, repo: &str) -> Step<()> {
        let answer = self
            .get(
                &format!("/repos/{repo}/actions/runs?status=completed&per_page=30"),
                repo,
            )
            .await?;
        let runs = answer["workflow_runs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let Some(after) = self.repo_cursor(repo).ci else {
            self.repo_cursor(repo).ci = Some(newest_id(&runs));
            return Ok(());
        };
        for run in ids_above(&runs, after) {
            self.repo_cursor(repo).ci = run["id"].as_u64();
            let conclusion = text(&run["conclusion"]);
            let branch = text(&run["head_branch"]);
            if conclusion != "success" && conclusion != "failure" {
                continue;
            }
            if !self.config.branches.is_empty() && !self.config.branches.contains(&branch) {
                continue;
            }
            if !self.passes(&run["triggering_actor"], None) {
                continue;
            }
            let payload = json!({
                "source": "github",
                "repo": repo,
                "event": "ci",
                "title": text(&run["name"]),
                "body": body(&run["display_title"]),
                "url": text(&run["html_url"]),
                "author": text(&run["triggering_actor"]["login"]),
                "branch": branch,
                "conclusion": conclusion,
            });
            self.fire(payload).await?;
        }
        Ok(())
    }

    async fn run(&mut self) -> Step<()> {
        for repo in self.config.repos.clone() {
            for kind in GithubEvent::ALL {
                if !self.config.events.contains(kind) {
                    continue;
                }
                match kind {
                    GithubEvent::Issue => self.issues(&repo).await?,
                    GithubEvent::PullRequest => self.pulls(&repo).await?,
                    GithubEvent::Comment => self.comments(&repo).await?,
                    GithubEvent::Push => self.pushes(&repo).await?,
                    GithubEvent::Ci => self.ci(&repo).await?,
                }
            }
        }
        Ok(())
    }
}

/// The token's own login, asked once per watcher.
async fn own_login(poller: &Poller<'_>) -> Step<String> {
    let known = poller
        .service
        .logins
        .lock()
        .unwrap()
        .get(&poller.watcher.id)
        .cloned();
    if let Some(login) = known {
        return Ok(login);
    }
    let me = poller.get("/user", "your account").await?;
    let login = text(&me["login"]);
    poller
        .service
        .logins
        .lock()
        .unwrap()
        .insert(poller.watcher.id.clone(), login.clone());
    Ok(login)
}

/// One check of a GitHub watcher (spec section 4.3).
pub(crate) async fn poll(
    service: &Watchers,
    watcher: &Watcher,
    config: &GithubConfig,
) -> Result<Poll> {
    let Some(token) = service.deps.vault.personal_value(&config.secret).await? else {
        let failure = Failure::Permanent(entry_gone(&config.secret));
        return service.note_failure(&watcher.id, failure).await;
    };
    let cursor: GithubCursor = service
        .deps
        .store
        .get_watcher_cursor(&watcher.id)
        .await?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| crate::Error::Other(e.to_string()))?;
    let mut poller = Poller {
        service,
        watcher,
        config,
        client,
        token: token.to_string(),
        own_login: String::new(),
        cursor,
    };
    let outcome = match own_login(&poller).await {
        Ok(login) => {
            poller.own_login = login;
            poller.run().await
        }
        Err(stop) => Err(stop),
    };
    poller.save().await?;
    match outcome {
        Ok(()) => {
            service.note_success(&watcher.id).await?;
            Ok(Poll::Next(service.deps.clock.now() + POLL_EVERY))
        }
        Err(Stop::Wait(at)) => Ok(Poll::Next(at)),
        Err(Stop::Halt) => Ok(Poll::Stopped),
        Err(Stop::Failed(failure)) => service.note_failure(&watcher.id, failure).await,
    }
}

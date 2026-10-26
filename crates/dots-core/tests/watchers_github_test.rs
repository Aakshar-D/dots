//! GitHub watchers (watchers spec section 4.3) against a wiremock GitHub: starting from now,
//! each event kind and its runs, the filters, the token's own account, ETags, the rate limit,
//! the refusals that pause (Review Focus 5) and the errors that back off.

mod common;

use chrono::TimeDelta;
use common::watchers::{t0, Harness};
use dots_core::model::{Dot, GithubConfig, GithubEvent, PausedReason, Watcher, WatcherConfig};
use dots_core::watchers::github::{cannot_see, server_error, token_refused};
use dots_core::watchers::{Poll, Sources};
use serde_json::{json, Value};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "github_pat_0123456789abcdef";

async fn setup(f: impl FnOnce(&mut GithubConfig)) -> (Harness, MockServer, Dot, Watcher) {
    let server = MockServer::start().await;
    let sources = Sources {
        github_api: server.uri(),
        ..Sources::default()
    };
    let h = Harness::with_sources(sources).await;
    h.secret("GITHUB_TOKEN", TOKEN).await;
    let dot = h.dot("triage").await;
    let mut config = GithubConfig::new(&["owner/repo"], "GITHUB_TOKEN");
    f(&mut config);
    let w = h
        .watcher(&dot, "Repo events", WatcherConfig::Github(config))
        .await;
    (h, server, dot, w)
}

/// From now on, `/user` and each of `routes` (a path and its JSON) answer 200; nothing else does.
async fn serve(server: &MockServer, routes: &[(&str, Value)]) {
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "dots-bot"})))
        .mount(server)
        .await;
    for (p, body) in routes {
        Mock::given(method("GET"))
            .and(path(*p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
            .mount(server)
            .await;
    }
}

fn user(login: &str) -> Value {
    json!({"login": login, "type": if login.ends_with("[bot]") { "Bot" } else { "User" }})
}

fn issue(id: u64, n: u64, title: &str, author: &str, labels: &[&str]) -> Value {
    json!({"id": id, "number": n, "title": title, "body": format!("Body of {n}."),
        "html_url": format!("https://github.com/owner/repo/issues/{n}"), "user": user(author),
        "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>()})
}

fn pull(id: u64, n: u64, title: &str, draft: bool) -> Value {
    json!({"id": id, "number": n, "title": title, "body": format!("Body of {n}."), "draft": draft,
        "html_url": format!("https://github.com/owner/repo/pull/{n}"), "user": user("alice"),
        "labels": [], "updated_at": "2026-10-23T09:05:00Z"})
}

fn subjects(runs: &[dots_core::model::Run]) -> Vec<(String, String)> {
    runs.iter()
        .map(|r| {
            let p = r.payload.as_ref().unwrap();
            (
                p["event"].as_str().unwrap().to_string(),
                p["title"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_new_watcher_starts_from_now_and_then_runs_once_per_new_issue() {
    let (h, server, dot, w) = setup(|c| c.events = vec![GithubEvent::Issue]).await;
    serve(
        &server,
        &[(
            "/repos/owner/repo/issues",
            json!([issue(1011, 11, "Old", "alice", &[])]),
        )],
    )
    .await;
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(1))
    );
    assert!(h.runs(&dot.id).await.is_empty());
    let mut pr = issue(1013, 13, "A PR", "alice", &[]);
    pr["pull_request"] = json!({});
    serve(
        &server,
        &[(
            "/repos/owner/repo/issues",
            json!([
                pr,
                issue(1012, 12, "Crash on start", "alice", &[]),
                issue(1011, 11, "Old", "alice", &[])
            ]),
        )],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].payload,
        Some(
            json!({"source": "github", "repo": "owner/repo", "event": "issue",
            "title": "Crash on start", "body": "Body of 12.",
            "url": "https://github.com/owner/repo/issues/12", "author": "alice", "number": 12,
            "watcher_id": w.id, "watcher_name": "Repo events"})
        )
    );
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
}

#[tokio::test]
async fn pull_requests_opened_or_marked_ready_wake_it_once() {
    let (h, server, dot, w) = setup(|c| c.events = vec![GithubEvent::PullRequest]).await;
    serve(
        &server,
        &[
            (
                "/repos/owner/repo/pulls",
                json!([pull(2005, 5, "Old", false)]),
            ),
            ("/repos/owner/repo/issues/events", json!([])),
        ],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let mut ready = pull(2007, 7, "Draft made ready", true);
    ready["pull_request"] = json!({});
    serve(&server, &[
        ("/repos/owner/repo/pulls", json!([pull(2007, 7, "Draft made ready", true), pull(2006, 6, "Add export", false), pull(2005, 5, "Old", false)])),
        ("/repos/owner/repo/issues/events", json!([{"id": 3001, "event": "ready_for_review", "actor": user("bob"), "issue": ready}])),
    ]).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(
        subjects(&runs),
        [
            ("pull_request".into(), "Add export".into()),
            ("pull_request".into(), "Draft made ready".into())
        ]
    );
    let p = runs[1].payload.as_ref().unwrap();
    assert_eq!(
        (p["number"].clone(), p["author"].clone()),
        (json!(7), json!("bob"))
    );
    assert_eq!(p["url"], "https://github.com/owner/repo/pull/7");
}

#[tokio::test]
async fn comments_and_reviews_with_a_body_wake_it() {
    let (h, server, dot, w) = setup(|c| c.events = vec![GithubEvent::Comment]).await;
    serve(&server, &[("/repos/owner/repo/issues/comments", json!([{"id": 4001, "body": "old",
        "html_url": "u", "user": user("bob"), "issue_url": "https://api.github.com/repos/owner/repo/issues/12"}])),
        ("/repos/owner/repo/pulls", json!([]))]).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    serve(&server, &[
        ("/repos/owner/repo/issues/comments", json!([{"id": 4002, "body": "Still crashes.",
            "html_url": "https://github.com/owner/repo/issues/12#issuecomment-4002", "user": user("bob"),
            "issue_url": "https://api.github.com/repos/owner/repo/issues/12"}])),
        ("/repos/owner/repo/issues/12", issue(1012, 12, "Crash on start", "alice", &[])),
        ("/repos/owner/repo/pulls", json!([pull(2006, 6, "Add export", false)])),
        ("/repos/owner/repo/pulls/6/reviews", json!([
            {"id": 5001, "body": "", "state": "APPROVED", "submitted_at": "2026-10-23T09:04:00Z",
             "html_url": "r1", "user": user("carol")},
            {"id": 5002, "body": "Please add tests.", "state": "COMMENTED",
             "submitted_at": "2026-10-23T09:05:00Z",
             "html_url": "https://github.com/owner/repo/pull/6#pullrequestreview-5002", "user": user("carol")}])),
    ]).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(
        subjects(&runs),
        [
            ("comment".into(), "Crash on start".into()),
            ("comment".into(), "Add export".into())
        ]
    );
    let (c, r) = (
        runs[0].payload.as_ref().unwrap(),
        runs[1].payload.as_ref().unwrap(),
    );
    assert_eq!(
        (c["body"].clone(), c["number"].clone(), c["author"].clone()),
        (json!("Still crashes."), json!(12), json!("bob"))
    );
    assert_eq!(
        (r["body"].clone(), r["number"].clone(), r["author"].clone()),
        (json!("Please add tests."), json!(6), json!("carol"))
    );
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 2);
}

#[tokio::test]
async fn a_push_lists_its_newest_20_commits_and_completed_ci_runs_on_the_branches_wake_it() {
    let (h, server, dot, w) = setup(|c| {
        c.events = vec![GithubEvent::Push, GithubEvent::Ci];
        c.branches = vec!["main".into()];
    })
    .await;
    let run = |id: u64, branch: &str, conclusion: &str| {
        json!({"id": id, "name": "CI",
        "display_title": "Fix crash", "head_branch": branch, "conclusion": conclusion,
        "html_url": format!("https://github.com/owner/repo/actions/runs/{id}"), "triggering_actor": user("alice")})
    };
    serve(
        &server,
        &[
            (
                "/repos/owner/repo/branches/main",
                json!({"name": "main", "commit": {"sha": "aaa"}}),
            ),
            (
                "/repos/owner/repo/actions/runs",
                json!({"workflow_runs": [run(6001, "main", "success")]}),
            ),
        ],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let commits: Vec<Value> = (1..=25)
        .map(|i| {
            json!({"sha": format!("{i:07}abcdef"),
        "commit": {"message": format!("Change {i}\n\nMore words.")}, "author": user("alice")})
        })
        .collect();
    serve(&server, &[
        ("/repos/owner/repo/branches/main", json!({"name": "main", "commit": {"sha": "bbb"}})),
        ("/repos/owner/repo/compare/aaa...bbb", json!({"commits": commits,
            "html_url": "https://github.com/owner/repo/compare/aaa...bbb"})),
        ("/repos/owner/repo/actions/runs", json!({"workflow_runs": [run(6004, "dev", "success"),
            run(6003, "main", "cancelled"), run(6002, "main", "failure"), run(6001, "main", "success")]})),
    ]).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 2);
    let push = runs[0].payload.as_ref().unwrap();
    assert_eq!(
        (
            push["event"].clone(),
            push["title"].clone(),
            push["branch"].clone()
        ),
        (json!("push"), json!("25 commits"), json!("main"))
    );
    let body = push["body"].as_str().unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 21);
    assert_eq!(lines[0], "(25 commits; the newest 20 listed)");
    assert_eq!(lines[1], "- 0000006 Change 6 (alice)");
    assert_eq!(lines[20], "- 0000025 Change 25 (alice)");
    assert_eq!(
        push["url"],
        "https://github.com/owner/repo/compare/aaa...bbb"
    );
    assert_eq!(
        runs[1].payload.as_ref().unwrap().clone(),
        json!({"source": "github", "repo": "owner/repo", "event": "ci", "title": "CI",
            "body": "Fix crash", "url": "https://github.com/owner/repo/actions/runs/6002",
            "author": "alice", "branch": "main", "conclusion": "failure",
            "watcher_id": w.id, "watcher_name": "Repo events"})
    );
}

#[tokio::test]
async fn labels_authors_and_bots_filter() {
    let (h, server, dot, w) = setup(|c| {
        c.events = vec![GithubEvent::Issue];
        c.label = Some("Bug".into());
        c.authors = vec!["Alice".into(), "dependabot[bot]".into()];
    })
    .await;
    serve(&server, &[("/repos/owner/repo/issues", json!([]))]).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    serve(
        &server,
        &[(
            "/repos/owner/repo/issues",
            json!([
                issue(23, 23, "Bump", "dependabot[bot]", &["bug"]),
                issue(22, 22, "Bob's", "bob", &["bug"]),
                issue(21, 21, "No label", "alice", &[]),
                issue(20, 20, "Alice's bug", "alice", &["bug"]),
            ]),
        )],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(
        subjects(&h.runs(&dot.id).await),
        [("issue".into(), "Alice's bug".into())]
    );
}

#[tokio::test]
async fn the_tokens_own_account_never_wakes_it_and_is_asked_for_once() {
    let (h, server, dot, w) = setup(|c| c.events = vec![GithubEvent::Issue]).await;
    serve(&server, &[("/repos/owner/repo/issues", json!([]))]).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    serve(
        &server,
        &[(
            "/repos/owner/repo/issues",
            json!([issue(31, 31, "Mine", "Dots-Bot", &[])]),
        )],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    h.watchers.poll_now(&w.id).await.unwrap();
    assert!(h.runs(&dot.id).await.is_empty());
    let asked = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/user")
        .count();
    assert_eq!(asked, 0, "asked again after the first poll");
}

#[tokio::test]
async fn an_unchanged_answer_is_asked_again_with_its_etag() {
    let (h, server, dot, w) = setup(|c| c.events = vec![GithubEvent::Issue]).await;
    serve(&server, &[]).await;
    Mock::given(method("GET"))
        .and(path("/repos/owner/repo/issues"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"abc\"")
                .set_body_json(json!([issue(1, 1, "x", "alice", &[])])),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    Mock::given(method("GET"))
        .and(path("/repos/owner/repo/issues"))
        .and(header("If-None-Match", "\"abc\""))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(1))
    );
    assert!(h.runs(&dot.id).await.is_empty());
    assert_eq!(h.store.get_watcher(&w.id).await.unwrap().last_error, None);
    server.verify().await;
}

#[tokio::test]
async fn a_rate_limit_waits_until_the_reset_without_an_error() {
    let (h, server, _dot, w) = setup(|c| c.events = vec![GithubEvent::Issue]).await;
    serve(&server, &[]).await;
    let reset = t0() + TimeDelta::minutes(15);
    Mock::given(method("GET"))
        .and(path("/repos/owner/repo/issues"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", reset.timestamp().to_string().as_str()),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert_eq!(h.watchers.poll_now(&w.id).await.unwrap(), Poll::Next(reset));
    Mock::given(method("GET"))
        .and(path("/repos/owner/repo/issues"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .mount(&server)
        .await;
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(2))
    );
    let now = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!((now.paused_reason, now.last_error), (None, None));
}

#[tokio::test]
async fn a_refused_token_pauses_the_watcher() {
    let (h, server, _dot, w) = setup(|c| c.events = vec![GithubEvent::Issue]).await;
    serve(&server, &[]).await;
    Mock::given(method("GET"))
        .and(path("/repos/owner/repo/issues"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    assert_eq!(h.watchers.poll_now(&w.id).await.unwrap(), Poll::Stopped);
    let now = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(now.paused_reason, Some(PausedReason::Error));
    assert_eq!(now.last_error, Some(token_refused("GITHUB_TOKEN")));
    assert!(!now.last_error.unwrap().contains(TOKEN));
}

#[tokio::test]
async fn a_repo_the_token_cannot_see_pauses_the_watcher_naming_it() {
    let (h, server, dot, w) = setup(|c| {
        c.repos = vec!["owner/repo".into(), "owner/private".into()];
        c.events = vec![GithubEvent::Issue];
    })
    .await;
    serve(
        &server,
        &[
            (
                "/repos/owner/repo/issues",
                json!([issue(11, 11, "Old", "alice", &[])]),
            ),
            ("/repos/owner/private/issues", json!([])),
        ],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    serve(
        &server,
        &[(
            "/repos/owner/repo/issues",
            json!([
                issue(12, 12, "New", "alice", &[]),
                issue(11, 11, "Old", "alice", &[])
            ]),
        )],
    )
    .await;
    assert_eq!(h.watchers.poll_now(&w.id).await.unwrap(), Poll::Stopped);
    assert_eq!(
        subjects(&h.runs(&dot.id).await),
        [("issue".into(), "New".into())]
    );
    assert_eq!(
        h.store.get_watcher(&w.id).await.unwrap().last_error,
        Some(cannot_see("GITHUB_TOKEN", "owner/private"))
    );
    h.watchers.resume(&w.id).await.unwrap();
    serve(
        &server,
        &[
            (
                "/repos/owner/repo/issues",
                json!([
                    issue(12, 12, "New", "alice", &[]),
                    issue(11, 11, "Old", "alice", &[])
                ]),
            ),
            ("/repos/owner/private/issues", json!([])),
        ],
    )
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
}

#[tokio::test]
async fn a_server_error_is_retried_with_backoff() {
    let (h, server, _dot, w) = setup(|c| c.events = vec![GithubEvent::Issue]).await;
    serve(&server, &[]).await;
    Mock::given(method("GET"))
        .and(path("/repos/owner/repo/issues"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(1))
    );
    let now = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(
        (now.paused_reason, now.last_error),
        (None, Some(server_error("502", "owner/repo")))
    );
}

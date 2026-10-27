use dots_core::publish::github::{GitHub, MergeOutcome};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "ghp_secrettoken123";
const REPO: &str = "/repos/Aakshar-D/dots";

async fn gh(server: &MockServer) -> GitHub {
    GitHub::new(&server.uri(), TOKEN).unwrap()
}

async fn access_server(scopes: Option<&str>, push: bool) -> MockServer {
    let server = MockServer::start().await;
    let mut user = ResponseTemplate::new(200).set_body_json(json!({"login": "shraddesh"}));
    if let Some(scopes) = scopes {
        user = user.insert_header("X-OAuth-Scopes", scopes);
    }
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("Authorization", format!("Bearer {TOKEN}").as_str()))
        .and(header("X-GitHub-Api-Version", "2022-11-28"))
        .respond_with(user)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(REPO))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"permissions": {"push": push}})),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_publisher_token_reads_its_login_scopes_and_push() {
    let server = access_server(Some("repo, workflow, gist"), true).await;
    let access = gh(&server).await.access().await.unwrap();
    assert_eq!(access.login, "shraddesh");
    assert!(access.push);
    assert_eq!(
        access.scopes,
        Some(vec!["repo".to_string(), "workflow".into(), "gist".into()])
    );
    assert_eq!(access.publisher_problem(), None);
}

#[tokio::test]
async fn a_token_without_workflow_or_push_is_not_a_publisher() {
    let server = access_server(Some("repo"), true).await;
    let access = gh(&server).await.access().await.unwrap();
    assert_eq!(
        access.publisher_problem(),
        Some("This token needs the repo and workflow scopes.")
    );
    let server = access_server(Some("repo, workflow"), false).await;
    let access = gh(&server).await.access().await.unwrap();
    assert_eq!(
        access.publisher_problem(),
        Some("This token cannot push to Aakshar-D/dots.")
    );
    // A fine-grained token has no scopes header; push decides.
    let server = access_server(None, true).await;
    let access = gh(&server).await.access().await.unwrap();
    assert_eq!(access.publisher_problem(), None);
}

#[tokio::test]
async fn errors_never_carry_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"message": format!("Bad credentials: {TOKEN}")})),
        )
        .mount(&server)
        .await;
    let err = gh(&server).await.access().await.unwrap_err().to_string();
    assert!(err.contains("401"), "{err}");
    assert!(err.contains("[hidden]"), "{err}");
    assert!(!err.contains(TOKEN), "{err}");
}

#[tokio::test]
async fn put_secret_refuses_an_existing_secret() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!(
            "{REPO}/actions/secrets/TAURI_SIGNING_PRIVATE_KEY"
        )))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"name": "TAURI_SIGNING_PRIVATE_KEY"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    let err = gh(&server)
        .await
        .put_secret("TAURI_SIGNING_PRIVATE_KEY", "kid", "c2VhbGVk")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("TAURI_SIGNING_PRIVATE_KEY already exists on GitHub."),
        "{err}"
    );
}

#[tokio::test]
async fn commit_files_builds_blobs_a_tree_and_a_commit_on_the_parent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO}/git/commits/parent1")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"sha": "parent1", "tree": {"sha": "tree0"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO}/git/blobs")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "blob1"})))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO}/git/trees")))
        .and(body_partial_json(json!({"base_tree": "tree0"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "tree1"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{REPO}/git/commits")))
        .and(body_partial_json(
            json!({"tree": "tree1", "parents": ["parent1"], "message": "Dots 0.3.0-rc.1"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "commit1"})))
        .expect(1)
        .mount(&server)
        .await;
    let files = vec![
        ("ui/package.json".to_string(), "{}".to_string()),
        ("release-notes.md".to_string(), "- New".to_string()),
    ];
    let sha = gh(&server)
        .await
        .commit_files("parent1", &files, "Dots 0.3.0-rc.1")
        .await
        .unwrap();
    assert_eq!(sha, "commit1");
}

#[tokio::test]
async fn merge_reports_merged_already_merged_and_conflict() {
    for (status, expected) in [
        (201, MergeOutcome::Merged("m1".into())),
        (204, MergeOutcome::AlreadyMerged),
        (409, MergeOutcome::Conflict),
    ] {
        let server = MockServer::start().await;
        let mut answer = ResponseTemplate::new(status);
        if status == 201 {
            answer = answer.set_body_json(json!({"sha": "m1"}));
        } else if status == 409 {
            answer = answer.set_body_json(json!({"message": "Merge conflict"}));
        }
        Mock::given(method("POST"))
            .and(path(format!("{REPO}/merges")))
            .and(body_partial_json(
                json!({"base": "master", "head": "release/0.3.0"}),
            ))
            .respond_with(answer)
            .mount(&server)
            .await;
        let got = gh(&server)
            .await
            .merge("master", "release/0.3.0", "Dots 0.3.0")
            .await
            .unwrap();
        assert_eq!(got, expected);
    }
}

#[tokio::test]
async fn merged_pulls_since_reads_merge_commits_newest_first() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO}/compare/v0.2.0...master")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "commits": [
                {"commit": {"message": "Merge pull request #25 from x/feat/watchers\n\nWatchers: dots that start on their own"}},
                {"commit": {"message": "fix: something small"}},
                {"commit": {"message": "Merge pull request #26 from x/feat/usage-limits\n\nUsage and limits"}}
            ]
        })))
        .mount(&server)
        .await;
    let pulls = gh(&server)
        .await
        .merged_pulls_since(Some("v0.2.0"), "master")
        .await
        .unwrap();
    let got: Vec<(u64, &str)> = pulls.iter().map(|p| (p.number, p.title.as_str())).collect();
    assert_eq!(
        got,
        [
            (26, "Usage and limits"),
            (25, "Watchers: dots that start on their own")
        ]
    );
}

#[tokio::test]
async fn a_run_for_a_tag_reads_its_jobs_and_steps() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO}/actions/runs")))
        .and(query_param("event", "push"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "workflow_runs": [
                {"id": 7, "head_branch": "v0.3.0-rc.1", "status": "in_progress", "conclusion": null, "html_url": "https://github.com/run/7"},
                {"id": 6, "head_branch": "v0.2.0", "status": "completed", "conclusion": "success", "html_url": "https://github.com/run/6"}
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{REPO}/actions/runs/7/jobs")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobs": [
                {"name": "test", "status": "completed", "conclusion": "success", "steps": []},
                {"name": "release", "status": "in_progress", "conclusion": null, "steps": [
                    {"name": "Sign the installer", "status": "in_progress", "conclusion": null}
                ]}
            ]
        })))
        .mount(&server)
        .await;
    let run = gh(&server)
        .await
        .run_for_tag("v0.3.0-rc.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((run.id, run.status.as_str()), (7, "in_progress"));
    assert_eq!(run.url, "https://github.com/run/7");
    assert_eq!(run.jobs.len(), 2);
    assert_eq!(run.jobs[1].steps[0].0, "Sign the installer");
    assert!(gh(&server)
        .await
        .run_for_tag("v9.9.9")
        .await
        .unwrap()
        .is_none());
}

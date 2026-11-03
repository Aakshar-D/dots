//! Deploys against a local bare repository as the remote and the fake `gh`: no network.

mod common;

use std::path::PathBuf;

use common::{git_out, init_repo};
use dots_core::deploy::{ApprovedBranch, Deployer, BRANCH_CHANGED, GH_MISSING};
use dots_core::workflow::{DeployMode, MergeMethod};
use tempfile::TempDir;

/// `FAKE_GH_LOG` and `FAKE_GH_MODE` are process-wide: a test that runs the fake `gh` holds this
/// for as long as it sets them and deploys.
static GH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const MIB: usize = 1024 * 1024;

/// A repository on `main` with a bare repository as its `origin`.
struct Scratch {
    dir: TempDir,
    repo: PathBuf,
    remote: PathBuf,
}

fn scratch() -> Scratch {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let remote = dir.path().join("remote.git");
    init_repo(&repo);
    git_out(dir.path(), &["init", "-q", "--bare", "remote.git"]);
    git_out(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git_out(&repo, &["push", "-q", "origin", "main"]);
    Scratch { dir, repo, remote }
}

impl Scratch {
    /// Commits `files` (path, content; `None` deletes) on `branch`, made from `main` when it does
    /// not exist yet, and returns the new tip.
    fn commit(&self, branch: &str, files: &[(&str, Option<&[u8]>)]) -> String {
        let exists = git_out(&self.repo, &["branch", "--list", branch]);
        if exists.is_empty() {
            git_out(&self.repo, &["checkout", "-q", "-b", branch, "main"]);
        } else {
            git_out(&self.repo, &["checkout", "-q", branch]);
        }
        for (path, content) in files {
            let file = self.repo.join(path);
            match content {
                Some(bytes) => {
                    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                    std::fs::write(&file, bytes).unwrap();
                }
                None => std::fs::remove_file(&file).unwrap(),
            }
        }
        git_out(&self.repo, &["add", "-A"]);
        git_out(&self.repo, &["commit", "-q", "-m", "work"]);
        let tip = git_out(&self.repo, &["rev-parse", "HEAD"]);
        git_out(&self.repo, &["checkout", "-q", "main"]);
        tip
    }

    /// The commit `branch` points at on the remote, if it is there.
    fn remote_tip(&self, branch: &str) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.remote)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("refs/heads/{branch}"))
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn log(&self) -> PathBuf {
        self.dir.path().join("gh.log")
    }

    /// The fake `gh`'s calls, one argument list each.
    fn gh_calls(&self) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(self.log()).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Deploys `branches` with the fake `gh` in `mode`.
    async fn deploy(
        &self,
        gh_mode: &str,
        mode: &DeployMode,
        branches: &[ApprovedBranch],
    ) -> Result<dots_core::deploy::DeployResult, String> {
        let _held = GH.lock().await;
        std::env::set_var("FAKE_GH_LOG", self.log());
        std::env::set_var("FAKE_GH_MODE", gh_mode);
        fake_gh()
            .deploy(
                &self.repo,
                "origin",
                "main",
                mode,
                branches,
                "Fix the bug\nThe rest of the task.",
                "The summary.",
            )
            .await
    }
}

fn fake_gh() -> Deployer {
    Deployer {
        gh: PathBuf::from(env!("CARGO_BIN_EXE_fake-gh")),
    }
}

fn approved(branch: &str, commit: &str) -> ApprovedBranch {
    ApprovedBranch {
        branch: branch.to_string(),
        commit: commit.to_string(),
        base: None,
    }
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// The fake's calls of `gh pr <sub>`.
fn calls_of(calls: &[Vec<String>], sub: &str) -> Vec<Vec<String>> {
    calls
        .iter()
        .filter(|c| c.len() > 1 && c[0] == "pr" && c[1] == sub)
        .cloned()
        .collect()
}

#[tokio::test]
async fn push_lands_the_approved_commit_on_the_remote() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let result = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    assert_eq!(s.remote_tip("dots/fix"), Some(tip.clone()));
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::json!({
            "branches": [{"branch": "dots/fix", "commit": tip, "pushed": true, "pr_url": null,
                "auto_merge": false}],
            "ready": false,
        })
    );
    assert!(s.gh_calls().is_empty(), "push runs no gh");
}

#[tokio::test]
async fn branch_mode_pushes_nothing_and_lists_the_branches_as_ready() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let result = s
        .deploy("ok", &DeployMode::Branch, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    assert!(result.ready);
    assert_eq!(result.branches.len(), 1);
    assert!(!result.branches[0].pushed);
    assert_eq!(result.branches[0].commit, tip);
    assert_eq!(s.remote_tip("dots/fix"), None);
    assert!(s.gh_calls().is_empty());
}

#[tokio::test]
async fn pr_pushes_then_opens_a_pull_request_against_the_base() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let result = s
        .deploy("ok", &DeployMode::Pr, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    assert_eq!(s.remote_tip("dots/fix"), Some(tip));
    let b = &result.branches[0];
    assert!(b.pushed);
    assert_eq!(b.pr_url.as_deref(), Some("https://github.com/o/r/pull/1"));
    assert!(!b.auto_merge);
    assert!(!result.ready);
    let calls = s.gh_calls();
    assert_eq!(calls[0], strings(&["--version"]));
    let view = calls_of(&calls, "view");
    assert_eq!(view.len(), 1);
    assert_eq!(view[0][2], "dots/fix");
    assert_eq!(
        calls_of(&calls, "create"),
        vec![strings(&[
            "pr",
            "create",
            "--head",
            "dots/fix",
            "--base",
            "main",
            "--title",
            "Fix the bug",
            "--body",
            "The summary.",
        ])]
    );
    assert!(calls_of(&calls, "merge").is_empty());
}

#[tokio::test]
async fn a_long_title_is_cut_to_256_characters() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let title = "é".repeat(300);
    {
        let _held = GH.lock().await;
        std::env::set_var("FAKE_GH_LOG", s.log());
        std::env::set_var("FAKE_GH_MODE", "ok");
        fake_gh()
            .deploy(
                &s.repo,
                "origin",
                "main",
                &DeployMode::Pr,
                &[approved("dots/fix", &tip)],
                &title,
                "",
            )
            .await
            .unwrap();
    }
    let create = calls_of(&s.gh_calls(), "create");
    assert_eq!(create[0][7], "é".repeat(256));
    assert_eq!(create[0][9], "");
}

#[tokio::test]
async fn an_open_pull_request_for_the_branch_is_reused() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let result = s
        .deploy("existing", &DeployMode::Pr, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    assert_eq!(
        result.branches[0].pr_url.as_deref(),
        Some("https://github.com/o/r/pull/7")
    );
    assert!(calls_of(&s.gh_calls(), "create").is_empty());
}

#[tokio::test]
async fn pr_merge_sets_auto_merge_with_the_merge_method() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let squash = DeployMode::PrMerge {
        merge_method: MergeMethod::default(),
    };
    let result = s
        .deploy("ok", &squash, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    assert!(result.branches[0].auto_merge);
    assert_eq!(
        calls_of(&s.gh_calls(), "merge"),
        vec![strings(&[
            "pr",
            "merge",
            "https://github.com/o/r/pull/1",
            "--auto",
            "--squash",
        ])]
    );

    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let rebase = DeployMode::PrMerge {
        merge_method: MergeMethod::Rebase,
    };
    s.deploy("ok", &rebase, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    let merge = calls_of(&s.gh_calls(), "merge");
    assert_eq!(merge[0][3..], strings(&["--auto", "--rebase"]));
}

#[tokio::test]
async fn a_failed_pr_create_fails_with_ghs_message_and_keeps_the_push() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let err = s
        .deploy(
            "fail-create",
            &DeployMode::Pr,
            &[approved("dots/fix", &tip)],
        )
        .await
        .unwrap_err();
    assert!(
        err.starts_with("gh pr create failed: pull request create failed: GraphQL:"),
        "{err}"
    );
    assert_eq!(s.remote_tip("dots/fix"), Some(tip), "nothing is undone");
}

#[tokio::test]
async fn a_failed_auto_merge_fails_with_ghs_message_and_keeps_the_push() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let mode = DeployMode::PrMerge {
        merge_method: MergeMethod::Squash,
    };
    let err = s
        .deploy("fail-merge", &mode, &[approved("dots/fix", &tip)])
        .await
        .unwrap_err();
    assert_eq!(
        err,
        "gh pr merge failed: GraphQL: Pull request Auto merge is not allowed for this \
         repository (enablePullRequestAutoMerge)"
    );
    assert_eq!(s.remote_tip("dots/fix"), Some(tip), "nothing is undone");
    assert_eq!(calls_of(&s.gh_calls(), "create").len(), 1);
}

#[tokio::test]
async fn a_missing_gh_fails_before_anything_is_pushed() {
    let s = scratch();
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let deployer = Deployer {
        gh: s.dir.path().join("no-such-gh.exe"),
    };
    let err = deployer
        .deploy(
            &s.repo,
            "origin",
            "main",
            &DeployMode::Pr,
            &[approved("dots/fix", &tip)],
            "Fix the bug",
            "The summary.",
        )
        .await
        .unwrap_err();
    assert_eq!(err, GH_MISSING);
    assert_eq!(err, "gh is not installed or not on the PATH");
    assert_eq!(s.remote_tip("dots/fix"), None);
}

#[tokio::test]
async fn a_branch_moved_after_approval_is_refused_and_nothing_is_pushed() {
    let s = scratch();
    let first = s.commit("dots/a", &[("a.txt", Some(b"a\n"))]);
    let approved_tip = s.commit("dots/b", &[("b.txt", Some(b"b\n"))]);
    s.commit("dots/b", &[("b.txt", Some(b"b, later\n"))]);
    let branches = [
        approved("dots/a", &first),
        approved("dots/b", &approved_tip),
    ];
    let err = s
        .deploy("ok", &DeployMode::Push, &branches)
        .await
        .unwrap_err();
    assert_eq!(err, BRANCH_CHANGED);
    assert_eq!(err, "The branch changed after it was approved.");
    assert_eq!(
        s.remote_tip("dots/a"),
        None,
        "the unchanged branch waits too"
    );
    assert_eq!(s.remote_tip("dots/b"), None);

    let gone = approved("dots/gone", &first);
    let err = s
        .deploy("ok", &DeployMode::Branch, &[gone])
        .await
        .unwrap_err();
    assert_eq!(err, BRANCH_CHANGED);
}

#[tokio::test]
async fn files_that_look_like_secrets_or_are_too_big_refuse_the_deploy() {
    let s = scratch();
    let big = vec![b'x'; 10 * MIB + 1];
    let exact = vec![b'x'; 10 * MIB];
    let tip = s.commit(
        "dots/fix",
        &[
            (".env", Some(b"TOKEN=1\n")),
            (".env.example", Some(b"TOKEN=\n")),
            ("config/id_rsa", Some(b"key\n")),
            ("x.PEM", Some(b"pem\n")),
            ("big.bin", Some(&big)),
            ("exact.bin", Some(&exact)),
            ("src/fix.rs", Some(b"fn fix() {}\n")),
        ],
    );
    let err = s
        .deploy("ok", &DeployMode::Pr, &[approved("dots/fix", &tip)])
        .await
        .unwrap_err();
    assert_eq!(
        err,
        "Not pushed: dots/fix holds files that look like secrets or are over 10 MB: .env, \
         big.bin, config/id_rsa, x.PEM. Remove them from the branch, then retry the step."
    );
    assert_eq!(s.remote_tip("dots/fix"), None);
    assert!(s.gh_calls().is_empty(), "refused before gh runs");

    let allowed = s.commit(
        "dots/ok",
        &[
            (".env.example", Some(b"TOKEN=\n")),
            ("docs/.ENV.sample", Some(b"TOKEN=\n")),
            ("exact.bin", Some(&exact)),
        ],
    );
    s.deploy("ok", &DeployMode::Push, &[approved("dots/ok", &allowed)])
        .await
        .unwrap();
    assert_eq!(s.remote_tip("dots/ok"), Some(allowed));
}

#[tokio::test]
async fn a_refusal_names_ten_files_and_counts_the_rest() {
    let s = scratch();
    let names: Vec<String> = (0..12).map(|i| format!("k{i:02}.key")).collect();
    let files: Vec<(&str, Option<&[u8]>)> = names
        .iter()
        .map(|n| (n.as_str(), Some(b"k\n".as_slice())))
        .collect();
    let tip = s.commit("dots/keys", &files);
    let err = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/keys", &tip)])
        .await
        .unwrap_err();
    let listed = names[..10].join(", ");
    assert_eq!(
        err,
        format!(
            "Not pushed: dots/keys holds files that look like secrets or are over 10 MB: \
             {listed} and 2 more. Remove them from the branch, then retry the step."
        )
    );
}

#[tokio::test]
async fn a_secret_added_and_then_deleted_on_the_branch_is_refused() {
    let s = scratch();
    let base = git_out(&s.repo, &["rev-parse", "main"]);
    s.commit("dots/fix", &[(".env", Some(b"TOKEN=1\n"))]);
    let tip = s.commit(
        "dots/fix",
        &[(".env", None), ("src/fix.rs", Some(b"fn fix() {}\n"))],
    );
    let mut branch = approved("dots/fix", &tip);
    let err = s
        .deploy("ok", &DeployMode::Pr, std::slice::from_ref(&branch))
        .await
        .unwrap_err();
    assert!(err.contains(": .env. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/fix"), None);
    assert!(s.gh_calls().is_empty(), "refused before gh runs");

    // A given base does not narrow the scan: it covers whatever the push would send.
    branch.base = Some(base);
    let err = s
        .deploy("ok", &DeployMode::Push, &[branch])
        .await
        .unwrap_err();
    assert!(err.contains(": .env. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/fix"), None);
}

#[tokio::test]
async fn a_huge_file_added_and_then_deleted_on_the_branch_is_refused() {
    let s = scratch();
    let big = vec![b'x'; 10 * MIB + 1];
    s.commit("dots/fix", &[("big.bin", Some(&big))]);
    let tip = s.commit(
        "dots/fix",
        &[("big.bin", None), ("src/fix.rs", Some(b"fn fix() {}\n"))],
    );
    let err = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/fix", &tip)])
        .await
        .unwrap_err();
    assert!(err.contains(": big.bin. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/fix"), None);
}

#[tokio::test]
async fn a_secret_in_an_unpushed_commit_below_the_merge_base_is_refused() {
    let s = scratch();
    // An unpushed commit on the default branch, which the feature branch is made from.
    s.commit("main", &[(".env", Some(b"TOKEN=1\n"))]);
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let err = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/fix", &tip)])
        .await
        .unwrap_err();
    assert!(err.contains(": .env. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/fix"), None);
}

#[tokio::test]
async fn a_given_base_does_not_hide_unpushed_commits_below_it() {
    let s = scratch();
    // An unpushed commit the branch's work started from, given as its base.
    let below = s.commit("main", &[(".env", Some(b"TOKEN=1\n"))]);
    let tip = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    let mut branch = approved("dots/fix", &tip);
    branch.base = Some(below);
    let err = s
        .deploy("ok", &DeployMode::Push, &[branch])
        .await
        .unwrap_err();
    assert!(err.contains(": .env. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/fix"), None);
}

#[tokio::test]
async fn a_forged_tracking_ref_does_not_hide_an_unpushed_commit() {
    let s = scratch();
    let leaked = s.commit("dots/leak", &[(".env", Some(b"TOKEN=1\n"))]);
    // A local claim that the remote has it; the bare remote does not.
    git_out(&s.repo, &["update-ref", "refs/remotes/origin/x", &leaked]);
    git_out(&s.repo, &["checkout", "-q", "-b", "dots/fix", "dots/leak"]);
    std::fs::write(s.repo.join("fix.rs"), "fn fix() {}\n").unwrap();
    git_out(&s.repo, &["add", "-A"]);
    git_out(&s.repo, &["commit", "-q", "-m", "work"]);
    let tip = git_out(&s.repo, &["rev-parse", "HEAD"]);
    git_out(&s.repo, &["checkout", "-q", "main"]);
    let err = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/fix", &tip)])
        .await
        .unwrap_err();
    assert!(err.contains(": .env. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/fix"), None);
}

#[tokio::test]
async fn a_merge_of_a_pushed_branch_holding_a_secret_is_not_refused() {
    let s = scratch();
    // `main` gains a key file after the branch point and is pushed; the branch merges it.
    let first = s.commit("dots/fix", &[("src/fix.rs", Some(b"fn fix() {}\n"))]);
    s.commit("main", &[("server.key", Some(b"key\n"))]);
    git_out(&s.repo, &["push", "-q", "origin", "main"]);
    git_out(&s.repo, &["checkout", "-q", "dots/fix"]);
    git_out(&s.repo, &["merge", "-q", "--no-ff", "-m", "merge", "main"]);
    let tip = git_out(&s.repo, &["rev-parse", "HEAD"]);
    git_out(&s.repo, &["checkout", "-q", "main"]);
    assert_ne!(first, tip);
    s.deploy("ok", &DeployMode::Push, &[approved("dots/fix", &tip)])
        .await
        .unwrap();
    assert_eq!(s.remote_tip("dots/fix"), Some(tip));
}

#[tokio::test]
async fn a_root_commit_is_scanned_whatever_the_log_config() {
    let s = scratch();
    git_out(&s.repo, &["config", "log.showRoot", "false"]);
    git_out(&s.repo, &["checkout", "-q", "--orphan", "dots/root"]);
    git_out(&s.repo, &["rm", "-rfq", "."]);
    std::fs::write(s.repo.join(".env"), "TOKEN=1\n").unwrap();
    git_out(&s.repo, &["add", "-A"]);
    git_out(&s.repo, &["commit", "-q", "-m", "root"]);
    let tip = git_out(&s.repo, &["rev-parse", "HEAD"]);
    git_out(&s.repo, &["checkout", "-q", "main"]);
    let err = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/root", &tip)])
        .await
        .unwrap_err();
    assert!(err.contains(": .env. Remove"), "{err}");
    assert_eq!(s.remote_tip("dots/root"), None);
}

#[tokio::test]
async fn the_same_content_under_a_secrets_name_is_refused() {
    let s = scratch();
    let tip = s.commit(
        "dots/fix",
        &[("a.txt", Some(b"same\n")), ("config/.env", Some(b"same\n"))],
    );
    let err = s
        .deploy("ok", &DeployMode::Push, &[approved("dots/fix", &tip)])
        .await
        .unwrap_err();
    assert!(err.contains(".env. Remove"), "{err}");
}

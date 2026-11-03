//! Deploys (spec 5.5): Dots, not an agent, pushes the branches a check approved, opens a pull
//! request for each and sets it to merge on its own, so what is pushed is exactly what the
//! person approved. `git` and `gh` run as the person, with their own credentials, which Dots
//! never reads. Nothing already pushed is undone when a later part fails: the step fails with
//! the tool's message and the person retries it.

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::workflow::{DeployMode, MergeMethod};

/// The longest one `git` or `gh` call of a deploy may take (a slow remote, a credential helper
/// waiting): past it the call is stopped (`kill_on_drop`) and the deploy fails.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The largest file a deploy pushes: a bigger one is refused as the secrets are (10 MiB).
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// The file names a deploy refuses to push (the person's decision of 2026-11-02), matched
/// against the last part of each path, ignoring case. An entry is a whole name (`.netrc`), a
/// name start ending in `*` (`id_rsa*`) or a name end starting with `*` (`*.pem`). An entry that
/// starts with `!` is an exception: a name it matches is allowed whatever else matches it.
pub const SECRET_NAMES: &[&str] = &[
    ".env",
    ".env.*",
    "!.env.example",
    "!.env.sample",
    "!.env.template",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa*",
    "id_dsa*",
    "id_ecdsa*",
    "id_ed25519*",
    ".netrc",
    ".npmrc",
    ".pypirc",
    "credentials.json",
];

/// The error of a deploy whose branch no longer points at the commit its check approved.
pub const BRANCH_CHANGED: &str = "The branch changed after it was approved.";

/// The error of a `pr` or `pr_merge` deploy when `gh` cannot be started.
pub const GH_MISSING: &str = "gh is not installed or not on the PATH";

/// Files a refused deploy names; the rest are counted.
const LISTED_FILES: usize = 10;
/// The longest pull request title, in characters.
const TITLE_CHARS: usize = 256;
/// The lines of a tool's error output a failed deploy quotes.
const ERROR_LINES: usize = 3;
/// The longest quote of a tool's error output, in characters.
const ERROR_CHARS: usize = 600;

/// Runs deploys. `gh` is the GitHub CLI to run: `"gh"` (found on the PATH) unless a test sets
/// another program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployer {
    pub gh: PathBuf,
}

impl Default for Deployer {
    fn default() -> Self {
        Deployer {
            gh: PathBuf::from("gh"),
        }
    }
}

/// A branch a check approved, at the commit it showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedBranch {
    pub branch: String,
    pub commit: String,
    /// The commit the branch's work started from, for the scan of the files it changed. `None`:
    /// the merge-base of the branch with the project's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

/// What a deploy did to one branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployedBranch {
    pub branch: String,
    pub commit: String,
    pub pushed: bool,
    pub pr_url: Option<String>,
    pub auto_merge: bool,
}

/// What a deploy did, kept as the step run's `deploy_result`. `ready` is set when the branches
/// were kept as they are and listed as ready (mode `branch`) rather than pushed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployResult {
    pub branches: Vec<DeployedBranch>,
    pub ready: bool,
}

impl Deployer {
    /// Deploys `branches` of the repository at `repo` as `mode` says: `branch` (and `member`,
    /// which the workflow turns into an agent step before it gets here) pushes nothing; `push`
    /// pushes each branch to `remote`; `pr` also opens a pull request against `base` (or reuses
    /// the open one, so a retry does not fail) titled with the first line of `title` and with
    /// `body` as its text; `pr_merge` also sets it to merge once GitHub allows. Before anything
    /// is pushed, every branch must still be at its approved commit and must hold no file that
    /// looks like a secret or is too big. `Err` is the step's error.
    #[allow(clippy::too_many_arguments)]
    pub async fn deploy(
        &self,
        repo: &Path,
        remote: &str,
        base: &str,
        mode: &DeployMode,
        branches: &[ApprovedBranch],
        title: &str,
        body: &str,
    ) -> Result<DeployResult, String> {
        for b in branches {
            check_tip(repo, b).await?;
        }
        let mut result = DeployResult {
            branches: branches
                .iter()
                .map(|b| DeployedBranch {
                    branch: b.branch.clone(),
                    commit: b.commit.clone(),
                    pushed: false,
                    pr_url: None,
                    auto_merge: false,
                })
                .collect(),
            ready: false,
        };
        let (pr, merge) = match mode {
            DeployMode::Branch | DeployMode::Member { .. } => {
                result.ready = true;
                return Ok(result);
            }
            DeployMode::Push => (false, None),
            DeployMode::Pr => (true, None),
            DeployMode::PrMerge { merge_method } => (true, Some(*merge_method)),
        };
        for b in branches {
            guard(repo, remote, base, b).await?;
        }
        // Found missing before the first push, so a deploy that cannot open its pull requests
        // pushes nothing.
        if pr {
            self.gh(repo, "gh", &["--version"]).await?;
        }
        for (b, done) in branches.iter().zip(&mut result.branches) {
            push(repo, remote, b).await?;
            done.pushed = true;
            if !pr {
                continue;
            }
            let url = match self.open_pr(repo, &b.branch).await? {
                Some(url) => url,
                None => self.create_pr(repo, base, &b.branch, title, body).await?,
            };
            done.pr_url = Some(url.clone());
            if let Some(method) = merge {
                let method = match method {
                    MergeMethod::Merge => "--merge",
                    MergeMethod::Squash => "--squash",
                    MergeMethod::Rebase => "--rebase",
                };
                let args = ["pr", "merge", url.as_str(), "--auto", method];
                let out = self.gh(repo, "gh pr merge", &args).await?;
                succeeded("gh pr merge", &out)?;
                done.auto_merge = true;
            }
        }
        Ok(result)
    }

    /// The URL of the open pull request for `branch`, if there is one. A failed lookup counts
    /// as none: creating one then fails with gh's own message.
    async fn open_pr(&self, repo: &Path, branch: &str) -> Result<Option<String>, String> {
        // `gh pr view <branch>` also finds a closed or merged pull request: only an open one is
        // reused, so a branch pushed again after a merge gets a new one.
        let args = [
            "pr",
            "view",
            branch,
            "--json",
            "url,state",
            "--jq",
            "select(.state == \"OPEN\") | .url",
        ];
        let out = self.gh(repo, "gh pr view", &args).await?;
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok((out.status.success() && !url.is_empty()).then_some(url))
    }

    /// Opens a pull request of `branch` into `base` and returns its URL, which `gh` prints as
    /// its last line.
    async fn create_pr(
        &self,
        repo: &Path,
        base: &str,
        branch: &str,
        title: &str,
        body: &str,
    ) -> Result<String, String> {
        let title = pr_title(title, branch);
        let args = [
            "pr", "create", "--head", branch, "--base", base, "--title", &title, "--body", body,
        ];
        let out = self.gh(repo, "gh pr create", &args).await?;
        let stdout = succeeded("gh pr create", &out)?;
        stdout
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "gh pr create printed no pull request URL.".to_string())
    }

    /// Runs `gh` in `repo`, without prompts. Exiting with an error is not an `Err` here: the
    /// caller decides what it means.
    async fn gh(&self, repo: &Path, tool: &str, args: &[&str]) -> Result<Output, String> {
        let mut cmd = Command::new(&self.gh);
        cmd.args(args)
            .current_dir(repo)
            .env("GH_PROMPT_DISABLED", "1");
        run(tool, &mut cmd, None).await
    }
}

/// Refuses `b` unless its branch still points at the approved commit.
async fn check_tip(repo: &Path, b: &ApprovedBranch) -> Result<(), String> {
    let commit = format!("refs/heads/{}^{{commit}}", b.branch);
    let args = [
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &commit,
    ];
    let out = run("git rev-parse", &mut git(repo, &args), None).await?;
    let tip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !tip.eq_ignore_ascii_case(b.commit.trim()) {
        return Err(BRANCH_CHANGED.to_string());
    }
    Ok(())
}

/// The secret and size check of one branch: every file in every commit the push would send,
/// even one deleted again by a later commit, as the history it pushes keeps it. With a given
/// `base`, the commits after it; else the commits the remote's tracking refs lack, or, when the
/// remote has none, those after the merge-base with the project's default branch.
async fn guard(repo: &Path, remote: &str, base: &str, b: &ApprovedBranch) -> Result<(), String> {
    let mut args: Vec<String> = [
        "log",
        "-z",
        "--raw",
        "--no-renames",
        "--diff-filter=d",
        "--format=",
        "--no-abbrev",
        "--diff-merges=separate",
        "--not",
    ]
    .map(String::from)
    .to_vec();
    let tracking = format!("refs/remotes/{remote}/");
    let refs = [
        "for-each-ref",
        "--count=1",
        "--format=%(refname)",
        &tracking,
    ];
    let out = run("git for-each-ref", &mut git(repo, &refs), None).await?;
    let has_tracking = !succeeded("git for-each-ref", &out)?.trim().is_empty();
    match &b.base {
        Some(from) => args.push(from.clone()),
        None if has_tracking => args.push(format!("--remotes={remote}")),
        None => {
            let default = crate::workspace::default_branch_ref(repo, base, remote).await;
            let mb = ["merge-base", "--end-of-options", &default, &b.commit];
            let out = run("git merge-base", &mut git(repo, &mb), None).await?;
            args.push(succeeded("git merge-base", &out)?.trim().to_string());
        }
    }
    // The second `--not` turns the selection back on for the commit itself.
    args.extend(["--not", "--end-of-options"].map(String::from));
    args.push(b.commit.clone());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run("git log", &mut git(repo, &args), None).await?;
    let raw = succeeded("git log", &out)?;
    // Plumbing output, so no setting of the repository changes it; `d` leaves deleted files
    // out, and with renames off a renamed file shows as added under its new name.
    let mut files: std::collections::BTreeSet<(String, Option<String>)> = Default::default();
    let mut fields = raw.split('\0').filter(|f| !f.is_empty());
    while let Some(meta) = fields.next() {
        let path = fields.next();
        let (Some(rest), Some(path)) = (meta.strip_prefix(':'), path) else {
            return Err("git log printed a record that could not be read.".to_string());
        };
        // `:<old mode> <new mode> <old id> <new id> <status>`; a submodule (mode 160000) is a
        // commit of another repository, not a file with a size.
        let meta: Vec<&str> = rest.split(' ').collect();
        let blob = match (meta.get(1), meta.get(3)) {
            (Some(&mode), Some(&id)) if mode != "160000" => Some(id.to_string()),
            (Some(_), Some(_)) => None,
            _ => return Err("git log printed a record that could not be read.".to_string()),
        };
        files.insert((path.to_string(), blob));
    }
    let ids: std::collections::BTreeSet<&str> =
        files.iter().filter_map(|(_, id)| id.as_deref()).collect();
    let mut sizes = std::collections::HashMap::new();
    if !ids.is_empty() {
        let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
        let args = ["cat-file", "--batch-check=%(objectname) %(objectsize)"];
        let out = run(
            "git cat-file",
            &mut git(repo, &args),
            Some(input.into_bytes()),
        )
        .await?;
        for line in succeeded("git cat-file", &out)?.lines() {
            if let Some((id, size)) = line.split_once(' ') {
                if let Ok(size) = size.trim().parse::<u64>() {
                    sizes.insert(id.to_string(), size);
                }
            }
        }
    }
    // A file whose size cannot be read is refused: the check fails closed.
    let refused: std::collections::BTreeSet<String> = files
        .iter()
        .filter_map(|(path, id)| {
            let size = id.as_ref().map(|id| sizes.get(id).copied());
            if looks_secret(path) || size.flatten().is_some_and(|s| s > MAX_FILE_BYTES) {
                Some(path.clone())
            } else if size == Some(None) {
                Some(format!("{path} (size unknown)"))
            } else {
                None
            }
        })
        .collect();
    if refused.is_empty() {
        return Ok(());
    }
    let refused: Vec<&str> = refused.iter().map(String::as_str).collect();
    let mut list = refused[..refused.len().min(LISTED_FILES)].join(", ");
    if refused.len() > LISTED_FILES {
        list.push_str(&format!(" and {} more", refused.len() - LISTED_FILES));
    }
    Err(format!(
        "Not pushed: {} holds files that look like secrets or are over 10 MB: {list}. Remove \
         them from the branch, then retry the step.",
        b.branch
    ))
}

/// Whether the file name of `path` matches [`SECRET_NAMES`].
fn looks_secret(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    let matches = |pattern: &str| {
        if let Some(end) = pattern.strip_prefix('*') {
            name.ends_with(end)
        } else if let Some(start) = pattern.strip_suffix('*') {
            name.starts_with(start)
        } else {
            name == pattern
        }
    };
    let mut secret = false;
    for pattern in SECRET_NAMES {
        match pattern.strip_prefix('!') {
            Some(allowed) if matches(allowed) => return false,
            Some(_) => {}
            None => secret |= matches(pattern),
        }
    }
    secret
}

/// Pushes the approved commit as the branch, without prompts. The commit, not the branch, is
/// the source: a commit made after the tip check is never pushed.
async fn push(repo: &Path, remote: &str, b: &ApprovedBranch) -> Result<(), String> {
    let refspec = format!("{}:refs/heads/{}", b.commit, b.branch);
    let args = ["push", "--end-of-options", remote, &refspec];
    let mut cmd = git(repo, &args);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    let out = run("git push", &mut cmd, None).await?;
    succeeded("git push", &out).map(drop)
}

/// `git -C <repo> <args>`.
fn git(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    cmd
}

/// The pull request title: the first line of `title`, at most [`TITLE_CHARS`] characters; the
/// branch name when that is blank (gh refuses an empty title).
fn pr_title(title: &str, branch: &str) -> String {
    let line = title.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return branch.to_string();
    }
    line.chars().take(TITLE_CHARS).collect()
}

/// Runs `cmd` to the end within [`TOOL_TIMEOUT`], feeding it `input`, and returns what it
/// printed. `tool` names it in the errors.
async fn run(tool: &str, cmd: &mut Command, input: Option<Vec<u8>>) -> Result<Output, String> {
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    crate::proc::hide_window(cmd);
    let mut child = cmd.spawn().map_err(|e| {
        if tool.starts_with("gh") && e.kind() == std::io::ErrorKind::NotFound {
            GH_MISSING.to_string()
        } else {
            format!("{tool} could not start: {e}")
        }
    })?;
    // Written from a task of its own, so a tool that answers while it reads never blocks on a
    // full output pipe.
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
        });
    }
    tokio::time::timeout(TOOL_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("{tool} did not finish within 5 minutes."))?
        .map_err(|e| format!("{tool} failed: {e}"))
}

/// The stdout of a tool that succeeded, else its error: the tool and the first lines of what it
/// wrote to stderr, with secret values scrubbed out.
fn succeeded(tool: &str, out: &Output) -> Result<String, String> {
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr = strip_userinfo(&crate::vault::redact::global().scrub(&stderr));
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(ERROR_LINES)
        .collect();
    let message = if lines.is_empty() {
        format!("exit code {}", out.status.code().unwrap_or(-1))
    } else {
        lines.join(" ")
    };
    let message: String = message.chars().take(ERROR_CHARS).collect();
    Err(format!("{tool} failed: {message}"))
}

/// `text` with the user and password of every `scheme://user:pass@host` URL cut out, which git
/// writes into its errors for a remote given that way.
fn strip_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => rest = &tail[at + 1..],
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::strip_userinfo;

    #[test]
    fn url_userinfo_is_cut_out_of_a_quoted_error() {
        assert_eq!(
            strip_userinfo("fatal: unable to access 'https://me:s3cret@github.com/o/r.git/': 403"),
            "fatal: unable to access 'https://github.com/o/r.git/': 403"
        );
        assert_eq!(
            strip_userinfo("https://tok@a.com/x and ssh://u@h/p"),
            "https://a.com/x and ssh://h/p"
        );
        assert_eq!(strip_userinfo("no url, a@b"), "no url, a@b");
        assert_eq!(
            strip_userinfo("https://github.com/o@r"),
            "https://github.com/o@r"
        );
    }
}

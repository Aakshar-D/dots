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
/// another program. `env` is set on each `gh` call beside Dots' own environment (empty, unless a
/// test gives its fake `gh` its settings this way, so tests running at once never share them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployer {
    pub gh: PathBuf,
    pub env: Vec<(String, String)>,
}

impl Default for Deployer {
    fn default() -> Self {
        Deployer {
            gh: PathBuf::from("gh"),
            env: Vec::new(),
        }
    }
}

/// A branch a check approved, at the commit it showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedBranch {
    pub branch: String,
    pub commit: String,
    /// The commit the branch's work started from. The secret scan does not use it: it covers
    /// whatever the push would send.
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
        check_tips(repo, branches).await?;
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
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .env("GH_PROMPT_DISABLED", "1");
        run(tool, &mut cmd, None).await
    }
}

/// Refuses unless every branch still points at the commit it was approved at.
pub async fn check_tips(repo: &Path, branches: &[ApprovedBranch]) -> Result<(), String> {
    for b in branches {
        check_tip(repo, b).await?;
    }
    Ok(())
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

/// The secret and size check of one branch: every file among the objects its push would send,
/// even one deleted again by a later commit, as the history it pushes keeps it. What is sent is
/// measured against the object ids `git ls-remote` reports for the URLs the push goes to, not
/// local refs. Replace refs are ignored (see [`git`]), as the push ignores them.
async fn guard(repo: &Path, remote: &str, _base: &str, b: &ApprovedBranch) -> Result<(), String> {
    let have = server_ids(repo, remote).await?;
    // The objects reachable from the commit and not from anything the server has, on stdin so
    // that a long list is no long command line. The commit was matched to a ref by `check_tip`;
    // each excluded id is given as `^<id>`, which every git version reads on stdin.
    let mut input = format!("{}\n", b.commit.trim());
    for id in &have {
        input.push('^');
        input.push_str(id);
        input.push('\n');
    }
    let args = ["rev-list", "--objects", "--ignore-missing", "--stdin"];
    let out = run(
        "git rev-list",
        &mut git(repo, &args),
        Some(input.into_bytes()),
    )
    .await?;
    let mut listed: Vec<(String, Option<String>)> = Vec::new();
    for line in succeeded("git rev-list", &out)?.lines() {
        let (id, path) = match line.split_once(' ') {
            Some((id, path)) => (id, Some(path.to_string())),
            None => (line, None),
        };
        if !is_object_id(id) {
            return Err("git rev-list printed a line that could not be read.".to_string());
        }
        listed.push((id.to_string(), path));
    }
    let ids: std::collections::BTreeSet<&str> = listed.iter().map(|(id, _)| id.as_str()).collect();
    // The type and size of each object. A line that cannot be read leaves the object out.
    let mut kinds: std::collections::HashMap<String, (String, u64)> = Default::default();
    if !ids.is_empty() {
        let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
        let args = [
            "cat-file",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ];
        let out = run(
            "git cat-file",
            &mut git(repo, &args),
            Some(input.into_bytes()),
        )
        .await?;
        for line in succeeded("git cat-file", &out)?.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            if let [id, kind, size] = parts[..] {
                if let Ok(size) = size.trim().parse::<u64>() {
                    kinds.insert(id.to_string(), (kind.to_string(), size));
                }
            }
        }
    }
    // A blob is listed once, under the first path it was found at: the names the sent trees
    // give it are read as well, so the same content under a secret's name is still seen.
    let blobs: std::collections::HashSet<Vec<u8>> = kinds
        .iter()
        .filter(|(_, (kind, _))| kind == "blob")
        .filter_map(|(id, _)| id_bytes(id))
        .collect();
    let tree_paths: std::collections::HashMap<&str, &str> = listed
        .iter()
        .filter_map(|(id, path)| Some((id.as_str(), path.as_deref()?)))
        .collect();
    let trees: String = kinds
        .iter()
        .filter(|(_, (kind, _))| kind == "tree")
        .map(|(id, _)| format!("{id}\n"))
        .collect();
    let mut names: Vec<String> = Vec::new();
    if !trees.is_empty() {
        let out = run(
            "git cat-file",
            &mut git(repo, &["cat-file", "--batch"]),
            Some(trees.into_bytes()),
        )
        .await?;
        if !out.status.success() {
            succeeded("git cat-file", &out)?;
        }
        names = tree_names(&out.stdout, &blobs, &tree_paths)
            .ok_or_else(|| "git cat-file printed a tree that could not be read.".to_string())?;
    }
    let mut refused: std::collections::BTreeSet<String> = Default::default();
    for (id, path) in &listed {
        let Some(path) = path else { continue };
        match kinds.get(id) {
            Some((kind, size)) if kind == "blob" => {
                if looks_secret(path) || *size > MAX_FILE_BYTES {
                    refused.insert(path.clone());
                }
            }
            // A tree or a commit has no size to check; a submodule is not listed at all.
            Some(_) => {}
            // Cannot be told what it is: refused, the check fails closed.
            None => {
                refused.insert(format!("{path} (size unknown)"));
            }
        }
    }
    // The paths the sent trees give a sent blob under a secret's name; one already listed
    // above is the same string, so it is named once.
    refused.extend(names);
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

/// The object ids the server a push to `remote` would go to already has: every id `git
/// ls-remote` lists for its push URL (`remote.<name>.pushurl` and `pushInsteadOf` applied), the
/// tips and the tags it peels. A remote with several push URLs pushes to each, so only the ids
/// every one of them lists count. A remote name or URL that starts with `-` is refused, so
/// neither is read as an option.
async fn server_ids(
    repo: &Path,
    remote: &str,
) -> Result<std::collections::BTreeSet<String>, String> {
    if remote.starts_with('-') {
        return Err("git remote get-url failed: the remote's name starts with \"-\"".to_string());
    }
    let args = [
        "remote",
        "get-url",
        "--push",
        "--all",
        "--end-of-options",
        remote,
    ];
    let out = run("git remote get-url", &mut git(repo, &args), None).await?;
    let urls: Vec<String> = succeeded("git remote get-url", &out)?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if urls.is_empty() {
        return Err("git remote get-url failed: the remote has no URL".to_string());
    }
    let mut have: Option<std::collections::BTreeSet<String>> = None;
    for url in &urls {
        if url.starts_with('-') {
            return Err(
                "git ls-remote failed: the remote's push URL starts with \"-\"".to_string(),
            );
        }
        let args = ["ls-remote", "--end-of-options", url.as_str()];
        let mut cmd = git(repo, &args);
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        let out = run("git ls-remote", &mut cmd, None).await?;
        let ids: std::collections::BTreeSet<String> = succeeded("git ls-remote", &out)?
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|id| is_object_id(id))
            .map(str::to_ascii_lowercase)
            .collect();
        have = Some(match have {
            None => ids,
            Some(have) => have.intersection(&ids).cloned().collect(),
        });
    }
    Ok(have.unwrap_or_default())
}

/// Whether `s` is a full object id (40 or 64 hex digits).
fn is_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// The raw bytes of the hex object id `id`, or `None` when it is not hex.
fn id_bytes(id: &str) -> Option<Vec<u8>> {
    if !id.len().is_multiple_of(2) {
        return None;
    }
    (0..id.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(id.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The paths of the entries in `batch`, the output of `git cat-file --batch` for trees, whose
/// object is one of `blobs` (raw ids) and whose name looks like a secret. A path is the tree's
/// path in `tree_paths` (by hex id, as `git rev-list --objects` printed it; empty for a root
/// tree) joined with the entry's name: a tree found at several paths is given at the first one
/// rev-list found, and a tree it gave no path at all gives the bare name. `None` when the
/// output does not parse.
fn tree_names(
    batch: &[u8],
    blobs: &std::collections::HashSet<Vec<u8>>,
    tree_paths: &std::collections::HashMap<&str, &str>,
) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut at = 0;
    while at < batch.len() {
        let eol = at + batch[at..].iter().position(|&c| c == b'\n')?;
        let header = std::str::from_utf8(&batch[at..eol]).ok()?;
        let parts: Vec<&str> = header.split(' ').collect();
        let [id, kind, size] = parts[..] else {
            return None;
        };
        if kind != "tree" {
            return None;
        }
        let size: usize = size.parse().ok()?;
        let body = batch.get(eol + 1..eol + 1 + size)?;
        let dir = tree_paths.get(id).copied().unwrap_or("");
        let hash_len = id.len() / 2;
        let mut rest = body;
        while !rest.is_empty() {
            let space = rest.iter().position(|&c| c == b' ')?;
            let nul = space + 1 + rest[space + 1..].iter().position(|&c| c == 0)?;
            let hash = rest.get(nul + 1..nul + 1 + hash_len)?;
            if blobs.contains(hash) {
                let name = String::from_utf8_lossy(&rest[space + 1..nul]);
                if looks_secret(&name) {
                    names.push(if dir.is_empty() {
                        name.into_owned()
                    } else {
                        format!("{dir}/{name}")
                    });
                }
            }
            rest = &rest[nul + 1 + hash_len..];
        }
        at = eol + 1 + size + 1;
    }
    Some(names)
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

/// `git -C <repo> <args>`, with replace refs (`refs/replace/*`) ignored: `git push` sends the
/// objects themselves, so the scan must read those, not what a replace ref puts in their place.
/// Set on each command, not the process, so commands run at once never share it.
fn git(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_NO_REPLACE_OBJECTS", "1");
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
    use super::{id_bytes, strip_userinfo, tree_names};

    #[test]
    fn tree_names_gives_full_paths_of_sent_blobs_under_secret_names_only() {
        let tree = "aa".repeat(20);
        let sent = "11".repeat(20);
        let kept = "22".repeat(20);
        let mut body = Vec::new();
        for (name, id) in [(".env", &sent), ("a.txt", &sent), ("b.key", &kept)] {
            body.extend_from_slice(format!("100644 {name}\0").as_bytes());
            body.extend_from_slice(&id_bytes(id).unwrap());
        }
        let mut batch = format!("{tree} tree {}\n", body.len()).into_bytes();
        batch.extend_from_slice(&body);
        batch.push(b'\n');
        let blobs = [id_bytes(&sent).unwrap()].into_iter().collect();
        let mut paths = std::collections::HashMap::new();
        paths.insert(tree.as_str(), "config");
        assert_eq!(
            tree_names(&batch, &blobs, &paths),
            Some(vec!["config/.env".to_string()])
        );
        paths.insert(tree.as_str(), "");
        assert_eq!(
            tree_names(&batch, &blobs, &paths),
            Some(vec![".env".to_string()])
        );
        assert_eq!(tree_names(&batch[..batch.len() - 3], &blobs, &paths), None);
        assert_eq!(id_bytes("0aFf"), Some(vec![0x0a, 0xff]));
        assert_eq!(id_bytes("0g"), None);
        assert_eq!(id_bytes("abc"), None);
    }

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

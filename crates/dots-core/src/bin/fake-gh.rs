//! Test stand-in for the GitHub CLI `gh`, for the deploy tests: it never reaches the network.
//!
//! Each call appends its arguments, as one JSON array of strings, to the file named in
//! `FAKE_GH_LOG` (when set). `FAKE_GH_MODE` selects the answers (default `ok`):
//! - `ok`: `pr view` prints nothing and exits 1 (the branch has no pull request), `pr create`
//!   prints `https://github.com/o/r/pull/1`, `pr merge` succeeds.
//! - `existing`: as `ok`, but `pr view` prints `https://github.com/o/r/pull/7`.
//! - `fail-create`: `pr create` fails as gh does, with a message on stderr and exit code 1.
//! - `fail-merge`: `pr merge` fails as gh does for a repository without auto-merge.
//!
//! `--version` always succeeds.

use std::io::Write as _;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(log) = std::env::var_os("FAKE_GH_LOG") {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(&args).unwrap()).unwrap();
    }
    let mode = std::env::var("FAKE_GH_MODE").unwrap_or_else(|_| "ok".to_string());
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match (words.as_slice(), mode.as_str()) {
        (["--version", ..], _) => println!("gh version 2.62.0 (fake)"),
        (["pr", "view", ..], "existing") => println!("https://github.com/o/r/pull/7"),
        (["pr", "view", branch, ..], _) => {
            eprintln!("no pull requests found for branch \"{branch}\"");
            std::process::exit(1);
        }
        (["pr", "create", ..], "fail-create") => {
            eprintln!("pull request create failed: GraphQL: Head sha can't be blank, Base sha can't be blank, No commits between main and dots/fix (createPullRequest)");
            std::process::exit(1);
        }
        (["pr", "create", ..], _) => println!("https://github.com/o/r/pull/1"),
        (["pr", "merge", ..], "fail-merge") => {
            eprintln!("GraphQL: Pull request Auto merge is not allowed for this repository (enablePullRequestAutoMerge)");
            std::process::exit(1);
        }
        (["pr", "merge", ..], _) => {}
        _ => {
            eprintln!("unknown command {words:?}");
            std::process::exit(1);
        }
    }
}

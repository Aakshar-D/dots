//! Test stand-in for the GitHub CLI `gh`, for the deploy tests: it never reaches the network.
//!
//! Each call appends its arguments, as one JSON array of strings, to the file named in
//! `FAKE_GH_LOG` (when set). `FAKE_GH_MODE` selects the answers (default `ok`):
//! - `ok`: `pr list` prints `[]` (the branch has no open pull request), `pr create` prints
//!   `https://github.com/o/r/pull/1`, `pr merge` succeeds.
//! - `existing`: as `ok`, but `pr list` prints one entry: the pull request
//!   `https://github.com/o/r/pull/7` of the `--head=` branch into the `--base=` branch, from the
//!   same repository.
//! - `existing-other-base`: as `existing`, but its base is `other`.
//! - `existing-fork`: as `existing`, but it comes from a fork (`isCrossRepository` true).
//! - `fail-create`: `pr create` fails as gh does, with a message on stderr and exit code 1.
//! - `fail-merge`: `pr merge` fails as gh does for a repository without auto-merge.
//!
//! When the arguments hold `--body-file=-`, all of stdin is written to the file `<FAKE_GH_LOG>.body`
//! (overwritten); the logged arguments are unchanged.
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
    if args.iter().any(|a| a == "--body-file=-") {
        if let Some(log) = std::env::var_os("FAKE_GH_LOG") {
            let mut body = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut body).unwrap();
            let mut path = log;
            path.push(".body");
            std::fs::write(path, body).unwrap();
        }
    }
    let mode = std::env::var("FAKE_GH_MODE").unwrap_or_else(|_| "ok".to_string());
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match (words.as_slice(), mode.as_str()) {
        (["--version", ..], _) => println!("gh version 2.62.0 (fake)"),
        (["pr", "list", ..], "ok" | "fail-create" | "fail-merge") => println!("[]"),
        (["pr", "list", ..], _) => {
            let flag = |name: &str| {
                args.iter()
                    .find_map(|a| a.strip_prefix(name))
                    .unwrap_or_default()
                    .to_string()
            };
            let base = if mode == "existing-other-base" {
                "other".to_string()
            } else {
                flag("--base=")
            };
            let entry = serde_json::json!({
                "url": "https://github.com/o/r/pull/7",
                "headRefName": flag("--head="),
                "baseRefName": base,
                "isCrossRepository": mode == "existing-fork",
            });
            println!("{}", serde_json::json!([entry]));
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

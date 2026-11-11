//! What a call hits of a workspace's guardrails (guardrails spec §4.1). Lexical, like the
//! send-and-delete guard whose line splitter it shares (`guard::split_line`), and it leans the
//! same way: a line it can't read that mentions something protected counts as a hit.

use std::path::Path;

use serde_json::Value;

use super::guard::{
    command_name, decode_encoded, is_flag, is_variable, name_words, operands, split_line,
    MAX_DEPTH, SHELLS, WRAPPERS,
};
use super::{glob_matches, lexical_normalize, normalize_pattern, workspace_key, COMMAND_TOOLS};
use crate::accounts::guardrails::{GitAction, Guardrails};

/// The file tools that write.
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Commands that only read the files they name: a shell line of nothing else, with no `>`,
/// may name a protected path.
const READ_COMMANDS: &[&str] = &[
    "cat",
    "type",
    "less",
    "more",
    "head",
    "tail",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "findstr",
    "ls",
    "dir",
    "wc",
    "stat",
    "file",
    "diff",
    "get-content",
    "gc",
    "select-string",
    "sls",
    "get-childitem",
    "gci",
    "test-path",
    "cd",
    "pwd",
    "echo",
];

/// The git commands that only read (and `add`, which stages).
const READ_GIT: &[&str] = &["status", "diff", "log", "show", "blame", "add"];

/// The words of a git action, for a line Dots can't read.
const GIT_WORDS: &[&str] = &["push", "merge", "release", "tag", "branch"];

/// What a call hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    /// A file write, or a shell line naming a file, under a protected path.
    Path,
    /// A protected git or GitHub action.
    Git,
    /// A protected command.
    Command,
    /// A shell line Dots can't read that mentions something protected.
    Unclear,
}

/// One thing a call hits: `what` reads "<what> is protected in the ... workspace".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailHit {
    pub kind: RailKind,
    pub what: String,
}

impl RailHit {
    fn git(action: GitAction) -> RailHit {
        RailHit {
            kind: RailKind::Git,
            what: action.label().to_string(),
        }
    }
}

/// The first thing the call of `tool` with `input` hits of `rails`, or `None`. `workspace` is
/// the run's folder, which protected paths are relative to.
pub fn check(
    rails: &Guardrails,
    tool: &str,
    input: &Value,
    workspace: Option<&Path>,
) -> Option<RailHit> {
    let tool = tool.trim();
    if WRITE_TOOLS.contains(&tool) {
        let raw = ["file_path", "path", "notebook_path"]
            .iter()
            .find_map(|k| input.get(k).and_then(Value::as_str))?;
        return protected_path(rails, raw, workspace).map(|shown| RailHit {
            kind: RailKind::Path,
            what: format!("the file {shown}"),
        });
    }
    if COMMAND_TOOLS.contains(&tool) {
        let command = input.get("command").and_then(Value::as_str)?;
        return shell(rails, command, workspace);
    }
    let rest = tool.strip_prefix("mcp__")?;
    let (_, name) = rest.split_once("__")?;
    mcp_name(rails, name).map(RailHit::git)
}

// ---- paths -----------------------------------------------------------------------------------

/// The path as the hit shows it, when `raw` is under one of the protected paths: relative to the
/// run's folder when it is inside it, otherwise matched at every depth (fails closed).
fn protected_path(rails: &Guardrails, raw: &str, workspace: Option<&Path>) -> Option<String> {
    if rails.protected_paths.is_empty() || raw.trim().is_empty() {
        return None;
    }
    let (abs, full) = lexical_normalize(raw.trim());
    let relative = if abs {
        workspace_key(workspace).and_then(|ws| {
            let ws = ws.trim_end_matches('/');
            full.strip_prefix(&format!("{ws}/")).map(str::to_string)
        })
    } else if full != ".." && !full.starts_with("../") {
        Some(full.clone())
    } else {
        None
    };
    let candidates: Vec<&str> = match &relative {
        Some(rel) => vec![rel.as_str()],
        None => std::iter::once(full.as_str())
            .chain(full.match_indices('/').map(|(i, _)| &full[i + 1..]))
            .collect(),
    };
    let hit = rails.protected_paths.iter().any(|pattern| {
        let (_, pattern) = normalize_pattern(pattern);
        let dir = pattern.strip_suffix("/**").filter(|d| !d.is_empty());
        candidates.iter().any(|path| {
            !path.is_empty()
                && (glob_matches(&pattern, path) || dir.is_some_and(|d| glob_matches(d, path)))
        })
    });
    if !hit {
        return None;
    }
    let shown = raw.trim().replace('\\', "/");
    Some(match relative {
        Some(rel) if abs => {
            let parts: Vec<&str> = shown.split('/').filter(|p| !p.is_empty()).collect();
            let keep = rel.split('/').count();
            parts[parts.len().saturating_sub(keep)..].join("/")
        }
        _ => shown.trim_start_matches("./").to_string(),
    })
}

// ---- shell lines -----------------------------------------------------------------------------

#[derive(Default)]
struct Walk {
    hits: Vec<RailHit>,
    /// The line has a part Dots can't read.
    unclear: bool,
}

fn shell(rails: &Guardrails, command: &str, workspace: Option<&Path>) -> Option<RailHit> {
    let mut walk = Walk::default();
    line(rails, command, 0, &mut walk);
    if let Some(hit) = walk.hits.into_iter().next() {
        return Some(hit);
    }
    if walk.unclear {
        if let Some(word) = mentions(rails, command) {
            return Some(RailHit {
                kind: RailKind::Unclear,
                what: format!("running a command Dots can't read (it mentions {word})"),
            });
        }
    }
    shell_path(rails, command, workspace).map(|shown| RailHit {
        kind: RailKind::Path,
        what: format!("the file {shown}"),
    })
}

fn line(rails: &Guardrails, text: &str, depth: usize, walk: &mut Walk) {
    if depth > MAX_DEPTH {
        walk.unclear = true;
        return;
    }
    let (commands, substitutions) = split_line(text);
    for sub in substitutions {
        line(rails, &sub, depth + 1, walk);
    }
    for words in &commands {
        simple(rails, words, depth, walk);
    }
}

/// One simple command, as `guard::command_hits` reads it: shell keywords and `VAR=value` before
/// it are skipped, wrappers and shells looked through.
fn simple(rails: &Guardrails, words: &[String], depth: usize, walk: &mut Walk) {
    let mut owned: Vec<String> = words.to_vec();
    loop {
        let Some(w) = owned.first().cloned() else {
            return;
        };
        if ["if", "then", "else", "elif", "do", "while", "until", "!"].contains(&w.as_str()) {
            owned.remove(0);
            continue;
        }
        if w.starts_with('$') && owned.get(1).is_some_and(|n| n == "=") {
            owned.drain(..2);
            continue;
        }
        if let Some((name, rest)) = w.split_once('=') {
            let plain = !name.is_empty()
                && !name.starts_with('-')
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            if plain {
                if name.starts_with('$') && !rest.is_empty() {
                    owned[0] = rest.to_string();
                } else {
                    owned.remove(0);
                }
                continue;
            }
        }
        break;
    }
    let words = owned.as_slice();
    let first = &words[0];
    if is_variable(first) {
        walk.unclear = true;
        return;
    }
    let name = command_name(first);
    if WRAPPERS.contains(&name.as_str()) {
        for i in 1..words.len() {
            simple(rails, &words[i..], depth, walk);
        }
        for w in &words[1..] {
            if w.contains(' ') {
                line(rails, w, depth + 1, walk);
            }
        }
        return;
    }
    if SHELLS.contains(&name.as_str()) {
        let mut it = words.iter().skip(1);
        while let Some(w) = it.next() {
            let is_c =
                w == "-c" || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('c'));
            if is_c {
                if let Some(script) = it.next() {
                    line(rails, script, depth + 1, walk);
                }
                break;
            }
        }
        return;
    }
    if name == "cmd" {
        if let Some(pos) = words.iter().position(|w| {
            w.get(..2)
                .is_some_and(|h| ["/c", "/k", "/r"].contains(&h.to_lowercase().as_str()))
        }) {
            let head = &words[pos][2..];
            let mut rest: Vec<&str> = Vec::new();
            if !head.is_empty() {
                rest.push(head);
            }
            rest.extend(words[pos + 1..].iter().map(String::as_str));
            line(rails, &rest.join(" "), depth + 1, walk);
        }
        return;
    }
    if name == "powershell" || name == "pwsh" {
        powershell(rails, words, depth, walk);
        return;
    }
    if name == "eval" || name == "iex" || name == "invoke-expression" {
        let rest: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        if rest.iter().any(|w| w.split_whitespace().any(is_variable)) {
            walk.unclear = true;
        }
        line(rails, &rest.join(" "), depth + 1, walk);
        return;
    }
    if name == "find" {
        for (i, w) in words.iter().enumerate() {
            if ["-exec", "-execdir", "-ok", "-okdir"].contains(&w.as_str()) {
                let tail: Vec<String> = words[i + 1..]
                    .iter()
                    .take_while(|t| *t != ";" && *t != "+")
                    .cloned()
                    .collect();
                simple(rails, &tail, depth, walk);
            }
        }
    }
    if name == "git" {
        git(rails, words, walk);
    }
    if name == "gh" {
        gh(rails, words, walk);
    }
    for pattern in &rails.protected_commands {
        if command_matches(pattern, &name, words) {
            walk.hits.push(RailHit {
                kind: RailKind::Command,
                what: format!("the command {}", pattern.trim()),
            });
        }
    }
}

fn powershell(rails: &Guardrails, words: &[String], depth: usize, walk: &mut Walk) {
    const VALUED: &[&str] = &[
        "-executionpolicy",
        "-ep",
        "-ex",
        "-windowstyle",
        "-w",
        "-workingdirectory",
        "-wd",
        "-configurationname",
        "-settingsfile",
        "-inputformat",
        "-outputformat",
        "-of",
        "-if",
        "-psconsolefile",
        "-version",
        "-v",
    ];
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut i = 1;
    while i < words.len() {
        let w = lower[i].as_str();
        let prefix_of = |full: &str, min: usize| w.len() >= min && full.starts_with(w);
        if VALUED.contains(&w) {
            i += 2;
            continue;
        }
        if w == "-e" || w == "-ec" || prefix_of("-encodedcommand", 3) {
            match words.get(i + 1).and_then(|b| decode_encoded(b)) {
                Some(script) => line(rails, &script, depth + 1, walk),
                None => walk.unclear = true,
            }
            return;
        }
        if prefix_of("-command", 2) {
            let rest: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
            line(rails, &rest.join(" "), depth + 1, walk);
            return;
        }
        if w.starts_with('-') && prefix_of("-file", 2) {
            return;
        }
        if w.starts_with('-') {
            i += 1;
            continue;
        }
        let rest: Vec<&str> = words[i..].iter().map(String::as_str).collect();
        line(rails, &rest.join(" "), depth + 1, walk);
        return;
    }
}

/// Whether the simple command `name words...` matches the protected command `pattern`: the same
/// command name and every other word of the pattern among its words. `rm -rf` reads as "rm with
/// recursive and force" in every spelling.
fn command_matches(pattern: &str, name: &str, words: &[String]) -> bool {
    let wanted: Vec<String> = pattern.split_whitespace().map(str::to_lowercase).collect();
    let Some(first) = wanted.first() else {
        return false;
    };
    if command_name(first) == "rm"
        && wanted.len() == 2
        && matches!(wanted[1].as_str(), "-rf" | "-fr")
    {
        return rm_rf(name, words);
    }
    if command_name(first) != name {
        return false;
    }
    let have: Vec<String> = words.iter().skip(1).map(|w| w.to_lowercase()).collect();
    wanted[1..].iter().all(|w| have.contains(w))
}

/// A recursive and forced delete: `rm -rf` (any flag spelling), PowerShell's `Remove-Item
/// -Recurse -Force` and its aliases, cmd's `rd /s /q`.
fn rm_rf(name: &str, words: &[String]) -> bool {
    const DELETES: &[&str] = &["rm", "remove-item", "ri", "del", "erase", "rd", "rmdir"];
    if !DELETES.contains(&name) {
        return false;
    }
    let flags: Vec<String> = words
        .iter()
        .skip(1)
        .map(|w| w.to_lowercase())
        .filter(|w| w.starts_with('-') || w.starts_with('/'))
        .collect();
    // `-rf`, `-Rf`, `-rfv`: a short cluster of letters, kept short so that PowerShell's
    // `-Force` is not read as `-r`.
    let short = |c: char| {
        words.iter().skip(1).any(|o| {
            o.starts_with('-')
                && !o.starts_with("--")
                && (2..=4).contains(&o.len())
                && o[1..].chars().all(|x| x.is_ascii_alphabetic())
                && o[1..].contains(c)
        })
    };
    let recursive = flags.iter().any(|f| {
        f == "--recursive" || f == "/s" || (f.len() >= 4 && "-recurse".starts_with(f.as_str()))
    }) || short('r')
        || short('R');
    let force = flags
        .iter()
        .any(|f| f == "--force" || f == "/q" || (f.len() >= 3 && "-force".starts_with(f.as_str())))
        || short('f');
    recursive && force
}

fn git(rails: &Guardrails, words: &[String], walk: &mut Walk) {
    let mut i = 1;
    while i < words.len() && words[i].starts_with('-') {
        i += if ["-C", "-c", "--git-dir", "--work-tree", "--namespace"].contains(&words[i].as_str())
        {
            2
        } else {
            1
        };
    }
    let Some(sub) = words.get(i).map(|s| s.to_lowercase()) else {
        return;
    };
    let args: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
    if args.iter().any(|a| *a == "--help" || *a == "-h") {
        return;
    }
    let mut found: Vec<GitAction> = Vec::new();
    match sub.as_str() {
        "push" => push(&args, &mut found),
        "branch" => {
            let delete = args.iter().any(|a| {
                *a == "--delete"
                    || (a.starts_with('-')
                        && !a.starts_with("--")
                        && (a.contains('d') || a.contains('D')))
            });
            if delete {
                found.push(GitAction::DeleteBranch);
            }
        }
        "tag" => {
            let lists = args.iter().any(|a| {
                [
                    "-l",
                    "--list",
                    "-n",
                    "--contains",
                    "--no-contains",
                    "--points-at",
                    "--merged",
                    "--no-merged",
                    "-v",
                    "--verify",
                ]
                .contains(a)
                    || a.starts_with("--sort")
                    || a.starts_with("--format")
                    || a.starts_with("-n")
            });
            let named = args.iter().any(|a| !a.starts_with('-'));
            if named && !lists {
                found.push(GitAction::Tags);
            }
        }
        _ => {}
    }
    for action in found {
        if rails.protected_git.contains(&action) {
            walk.hits.push(RailHit::git(action));
        }
    }
}

/// What a `git push` does of the protected git actions, in this order: force, delete, tags,
/// pushing to main or master.
fn push(args: &[&str], found: &mut Vec<GitAction>) {
    const VALUED: &[&str] = &["-o", "--push-option", "--repo", "--receive-pack", "--exec"];
    let mut flags: Vec<&str> = Vec::new();
    let mut operands: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if VALUED.contains(a) {
            it.next();
        } else if a.starts_with('-') {
            flags.push(a);
        } else {
            operands.push(a);
        }
    }
    let short = |c: char| {
        flags
            .iter()
            .any(|f| !f.starts_with("--") && f.len() > 1 && f[1..].contains(c))
    };
    let refspecs: Vec<&str> = operands.iter().skip(1).copied().collect();
    let force = flags.iter().any(|f| {
        *f == "--force" || f.starts_with("--force-with-lease") || *f == "--force-if-includes"
    }) || short('f')
        || refspecs.iter().any(|r| r.starts_with('+'));
    let delete = flags.contains(&"--delete")
        || short('d')
        || refspecs.iter().any(|r| r.starts_with(':') && r.len() > 1);
    let tag_word = refspecs.iter().position(|r| *r == "tag");
    let tags = flags.contains(&"--tags")
        || flags.contains(&"--follow-tags")
        || tag_word.is_some()
        || refspecs.iter().any(|r| r.contains("refs/tags/"));
    let all = flags
        .iter()
        .any(|f| ["--all", "--mirror", "--branches"].contains(f));
    let named: Vec<&str> = refspecs
        .iter()
        .enumerate()
        .filter(|(i, _)| !matches!(tag_word, Some(t) if *i == t || *i == t + 1))
        .map(|(_, r)| *r)
        .collect();
    let to_main = named.iter().any(|r| {
        let r = r.trim_start_matches('+');
        let dst = r.rsplit_once(':').map_or(r, |(_, d)| d);
        let dst = dst
            .strip_prefix("refs/heads/")
            .unwrap_or(dst)
            .to_lowercase();
        ["main", "master", "head", "@"].contains(&dst.as_str())
    });
    let unnamed = named.is_empty() && !tags && !delete;
    if force {
        found.push(GitAction::ForcePush);
    }
    if delete {
        found.push(GitAction::DeleteBranch);
    }
    if tags {
        found.push(GitAction::Tags);
    }
    if all || to_main || unnamed {
        found.push(GitAction::PushMain);
    }
}

fn gh(rails: &Guardrails, words: &[String], walk: &mut Walk) {
    let ops: Vec<String> = operands(words).iter().map(|o| o.to_lowercase()).collect();
    let head: Vec<&str> = ops.iter().take(4).map(String::as_str).collect();
    let pair = |a: &str, bs: &[&str]| head.windows(2).any(|w| w[0] == a && bs.contains(&w[1]));
    let mut found: Vec<GitAction> = Vec::new();
    if pair("pr", &["merge"]) {
        found.push(GitAction::MergePr);
    }
    if pair(
        "release",
        &["create", "edit", "delete", "upload", "delete-asset"],
    ) {
        found.push(GitAction::Release);
    }
    if head.first() == Some(&"api") {
        let mut method: Option<String> = None;
        let mut fields = false;
        let mut it = words.iter().skip(1);
        while let Some(w) = it.next() {
            if w == "-X" || w == "--method" {
                method = it.next().map(|m| m.to_uppercase());
            } else if let Some(m) = w.strip_prefix("--method=").or_else(|| w.strip_prefix("-X")) {
                if !m.is_empty() {
                    method = Some(m.to_uppercase());
                }
            } else if ["-f", "-F", "--field", "--raw-field", "--input"].contains(&w.as_str())
                || w.starts_with("--field=")
                || w.starts_with("--raw-field=")
            {
                fields = true;
            }
        }
        let writes = match method.as_deref() {
            Some("GET" | "HEAD") => false,
            Some(_) => true,
            None => fields,
        };
        let path = ops.iter().skip(1).find(|o| o.contains('/'));
        if let (true, Some(path)) = (writes, path) {
            if path.trim_end_matches('/').ends_with("/merge") {
                found.push(GitAction::MergePr);
            }
            if path.contains("/releases") {
                found.push(GitAction::Release);
            }
        }
    }
    for action in found {
        if rails.protected_git.contains(&action) {
            walk.hits.push(RailHit::git(action));
        }
    }
}

/// The protected GitHub action an MCP tool's name does (`merge_pull_request`,
/// `create_release`), if it is protected.
fn mcp_name(rails: &Guardrails, name: &str) -> Option<GitAction> {
    let words = name_words(name);
    let has = |w: &str| words.iter().any(|x| x == w);
    let merge = has("merge") && (has("pull") || has("pr") || has("request") || has("prs"));
    let release = (has("release") || has("releases"))
        && [
            "create", "publish", "delete", "update", "edit", "upload", "new",
        ]
        .iter()
        .any(|v| has(v));
    [(merge, GitAction::MergePr), (release, GitAction::Release)]
        .into_iter()
        .find(|(hit, action)| *hit && rails.protected_git.contains(action))
        .map(|(_, action)| action)
}

/// A word of `text` naming something protected, for a line Dots can't read: a git action's
/// word, a protected command's name, or a protected path's literal part.
fn mentions(rails: &Guardrails, text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || "-_.".contains(c)))
        .filter(|t| !t.is_empty())
        .collect();
    if !rails.protected_git.is_empty() {
        if let Some(w) = GIT_WORDS.iter().find(|w| tokens.contains(w)) {
            return Some(w.to_string());
        }
    }
    for pattern in &rails.protected_commands {
        if let Some(first) = pattern.split_whitespace().next() {
            let first = first.to_lowercase();
            if tokens.contains(&first.as_str()) {
                return Some(first);
            }
        }
    }
    for pattern in &rails.protected_paths {
        let pattern = pattern.to_lowercase();
        let literal = pattern
            .split(['*', '?', '/'])
            .filter(|p| p.len() >= 3)
            .find(|p| lower.contains(*p));
        if let Some(part) = literal {
            return Some(part.to_string());
        }
    }
    None
}

/// A protected path that a shell line names, unless the line only reads.
fn shell_path(rails: &Guardrails, command: &str, workspace: Option<&Path>) -> Option<String> {
    if rails.protected_paths.is_empty() {
        return None;
    }
    let (commands, _) = split_line(command);
    let reads_only = !command.contains('>') && commands.iter().all(|w| reads(w));
    if reads_only {
        return None;
    }
    command
        .split(|c: char| c.is_whitespace() || ";|&<>()'\"`=,".contains(c))
        .filter(|t| !t.is_empty() && !is_flag(t))
        .find_map(|t| protected_path(rails, t, workspace))
}

/// Whether the simple command only reads.
fn reads(words: &[String]) -> bool {
    let Some(first) = words.first() else {
        return true;
    };
    let name = command_name(first);
    if name == "git" {
        let sub = words
            .iter()
            .skip(1)
            .find(|w| !w.starts_with('-'))
            .map(|w| w.to_lowercase());
        return sub.is_some_and(|s| READ_GIT.contains(&s.as_str()));
    }
    READ_COMMANDS.contains(&name.as_str())
}

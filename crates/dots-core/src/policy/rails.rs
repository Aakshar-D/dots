//! What a call hits of a workspace's guardrails (guardrails spec §4.1). Lexical, like the
//! send-and-delete guard whose line splitter it shares (`guard::split_line`), and it leans the
//! same way: a line it can't read that mentions something protected counts as a hit.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::guard::{
    command_name, decode_encoded, is_flag, is_variable, name_words, operands, split_line,
    MAX_DEPTH, SHELLS, WRAPPERS,
};
use super::{glob_matches, lexical_normalize, normalize_pattern, workspace_key, COMMAND_TOOLS};
use crate::accounts::guardrails::{GitAction, GuardRole, Guardrails};

/// The file tools that write.
pub const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];

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
    "pwd",
    "echo",
];

/// Commands that change the folder the rest of the line runs in.
const CD_COMMANDS: &[&str] = &[
    "cd",
    "chdir",
    "pushd",
    "set-location",
    "sl",
    "push-location",
];

/// The git commands that only read (and `add`, which stages).
const READ_GIT: &[&str] = &["status", "diff", "log", "show", "blame", "add"];

/// Git's own commands. Any other word in their place is an alias or an extension, which Dots
/// can't read.
const GIT_BUILTINS: &[&str] = &[
    "add",
    "am",
    "annotate",
    "apply",
    "archive",
    "bisect",
    "blame",
    "branch",
    "bundle",
    "cat-file",
    "check-attr",
    "check-ignore",
    "check-mailmap",
    "check-ref-format",
    "checkout",
    "checkout-index",
    "cherry",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "commit-graph",
    "commit-tree",
    "config",
    "count-objects",
    "credential",
    "describe",
    "diff",
    "diff-files",
    "diff-index",
    "diff-tree",
    "difftool",
    "fetch",
    "for-each-ref",
    "format-patch",
    "fsck",
    "gc",
    "grep",
    "hash-object",
    "help",
    "init",
    "log",
    "ls-files",
    "ls-remote",
    "ls-tree",
    "maintenance",
    "merge",
    "merge-base",
    "merge-file",
    "merge-tree",
    "mergetool",
    "mv",
    "name-rev",
    "notes",
    "prune",
    "pull",
    "push",
    "range-diff",
    "read-tree",
    "rebase",
    "reflog",
    "remote",
    "repack",
    "replace",
    "rerere",
    "reset",
    "restore",
    "rev-list",
    "rev-parse",
    "revert",
    "rm",
    "shortlog",
    "show",
    "show-branch",
    "show-ref",
    "sparse-checkout",
    "stash",
    "status",
    "submodule",
    "switch",
    "symbolic-ref",
    "tag",
    "update-index",
    "update-ref",
    "var",
    "verify-commit",
    "verify-tag",
    "version",
    "whatchanged",
    "worktree",
    "lfs",
];

/// `git push`'s long options, for the prefixes git accepts (`--force-w`, `--del`).
const PUSH_OPTIONS: &[&str] = &[
    "--all",
    "--branches",
    "--mirror",
    "--tags",
    "--follow-tags",
    "--no-follow-tags",
    "--delete",
    "--force",
    "--force-with-lease",
    "--no-force-with-lease",
    "--force-if-includes",
    "--no-force-if-includes",
    "--prune",
    "--dry-run",
    "--porcelain",
    "--set-upstream",
    "--quiet",
    "--verbose",
    "--progress",
    "--no-progress",
    "--no-verify",
    "--verify",
    "--atomic",
    "--no-atomic",
    "--signed",
    "--no-signed",
    "--thin",
    "--no-thin",
    "--repo",
    "--receive-pack",
    "--exec",
    "--push-option",
    "--recurse-submodules",
    "--no-recurse-submodules",
    "--ipv4",
    "--ipv6",
];

/// The `git push` options that take a value.
const PUSH_VALUED: &[&str] = &["--repo", "--receive-pack", "--exec", "--push-option"];

/// `git branch`'s long options.
const BRANCH_OPTIONS: &[&str] = &[
    "--delete",
    "--move",
    "--copy",
    "--force",
    "--list",
    "--all",
    "--remotes",
    "--show-current",
    "--set-upstream-to",
    "--unset-upstream",
    "--edit-description",
    "--contains",
    "--no-contains",
    "--merged",
    "--no-merged",
    "--verbose",
    "--quiet",
    "--track",
    "--no-track",
    "--sort",
    "--points-at",
    "--format",
    "--color",
    "--no-color",
    "--column",
    "--no-column",
    "--ignore-case",
    "--abbrev",
    "--no-abbrev",
    "--create-reflog",
    "--recurse-submodules",
    "--omit-empty",
];

/// Interpreters: what they run is code Dots doesn't read.
const INTERPRETERS: &[&str] = &[
    "python", "python3", "py", "node", "deno", "bun", "ruby", "perl", "php",
];

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

    fn unclear(what: impl Into<String>) -> RailHit {
        RailHit {
            kind: RailKind::Unclear,
            what: what.into(),
        }
    }
}

/// `check_as` for a Member, the strictest reading.
pub fn check(
    rails: &Guardrails,
    tool: &str,
    input: &Value,
    workspace: Option<&Path>,
) -> Option<RailHit> {
    check_as(rails, GuardRole::Member, tool, input, workspace)
}

/// The first thing the call of `tool` with `input` hits of `rails` for a dot whose role there
/// is `role`, or `None`. `workspace` is the run's folder: protected paths are relative to it,
/// and the branch it has checked out decides a push that names none.
pub fn check_as(
    rails: &Guardrails,
    role: GuardRole,
    tool: &str,
    input: &Value,
    workspace: Option<&Path>,
) -> Option<RailHit> {
    let tool = tool.trim();
    if WRITE_TOOLS.contains(&tool) {
        let raw = ["file_path", "path", "notebook_path"]
            .iter()
            .find_map(|k| input.get(k).and_then(Value::as_str))?;
        return protected_path(rails, raw, workspace, true).map(|shown| RailHit {
            kind: RailKind::Path,
            what: format!("the file {shown}"),
        });
    }
    if COMMAND_TOOLS.contains(&tool) {
        let command = input.get("command").and_then(Value::as_str)?;
        return shell(rails, role, command, workspace);
    }
    let rest = tool.strip_prefix("mcp__")?;
    let (_, name) = rest.split_once("__")?;
    mcp_name(rails, name)
}

// ---- paths -----------------------------------------------------------------------------------

/// A protected path as a pattern: a bare folder (no glob characters, or ending in `/`) covers
/// everything in it.
fn path_pattern(raw: &str) -> String {
    let (_, p) = normalize_pattern(raw);
    if let Some(dir) = p.strip_suffix('/') {
        return format!("{dir}/**");
    }
    if p.contains(['*', '?', '[']) {
        p
    } else {
        format!("{p}/**")
    }
}

/// A Windows short (8.3) name such as `PROGRA~1`: the long name it stands for is unknown.
fn short_name(component: &str) -> bool {
    component
        .find('~')
        .is_some_and(|i| component[i + 1..].starts_with(|c: char| c.is_ascii_digit()))
}

/// The path as the hit shows it, when `raw` is under one of the protected paths: relative to the
/// run's folder when it is inside it, otherwise matched at every depth (fails closed). A short
/// (8.3) name is a hit while any path is protected, when `short_names` is true.
fn protected_path(
    rails: &Guardrails,
    raw: &str,
    workspace: Option<&Path>,
    short_names: bool,
) -> Option<String> {
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
    let short = short_names
        && candidates
            .first()
            .is_some_and(|c| c.split('/').any(short_name));
    let hit = short
        || rails.protected_paths.iter().any(|pattern| {
            let pattern = path_pattern(pattern);
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

/// The literal folder and file names of the protected paths (`migrations`, `.github`,
/// `cargo.lock`), lower case.
fn path_literals(rails: &Guardrails) -> Vec<String> {
    rails
        .protected_paths
        .iter()
        .flat_map(|p| {
            p.to_lowercase()
                .replace('\\', "/")
                .split('/')
                .filter(|s| !s.is_empty() && !s.contains(['*', '?', '[']) && *s != ".")
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---- shell lines -----------------------------------------------------------------------------

#[derive(Default)]
struct Walk {
    hits: Vec<RailHit>,
    /// The line has a part Dots can't read.
    unclear: bool,
    /// It runs code Dots never sees (a script piped into a shell): a hit by itself.
    blind: bool,
    /// The texts of scripts Dots read from inside the line (`-c`, an encoded command), for the
    /// mentions of an unclear line.
    texts: Vec<String>,
    /// It changes folder or repository first, so the checked-out branch is unknown.
    moved: bool,
    /// The branch the run's folder has checked out; `None` when detached or unreadable.
    head: Option<String>,
    workspace: Option<PathBuf>,
}

/// The branch checked out in `workspace`, from `.git/HEAD` (in a worktree, the `gitdir` file
/// `.git` names first). `None` when detached or unreadable.
fn read_head(workspace: Option<&Path>) -> Option<String> {
    let ws = workspace?;
    let dotgit = ws.join(".git");
    let gitdir = if dotgit.is_dir() {
        dotgit
    } else {
        let text = std::fs::read_to_string(&dotgit).ok()?;
        let named = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if named.is_relative() {
            ws.join(named)
        } else {
            named
        }
    };
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
}

fn shell(
    rails: &Guardrails,
    role: GuardRole,
    command: &str,
    workspace: Option<&Path>,
) -> Option<RailHit> {
    let mut walk = Walk {
        head: read_head(workspace),
        workspace: workspace.map(Path::to_path_buf),
        ..Walk::default()
    };
    line(rails, command, 0, &mut walk);
    if let Some(hit) = walk.hits.into_iter().next() {
        return Some(hit);
    }
    let anything = !(rails.protected_paths.is_empty()
        && rails.protected_git.is_empty()
        && rails.protected_commands.is_empty());
    if walk.blind && anything {
        return Some(RailHit::unclear(
            "running a script Dots can't read (it is piped into a shell)",
        ));
    }
    if walk.unclear {
        let mut text = command.to_string();
        for t in &walk.texts {
            text.push('\n');
            text.push_str(t);
        }
        if let Some(word) = mentions(rails, role, &text) {
            return Some(RailHit::unclear(format!(
                "running a command Dots can't read (it mentions {word})"
            )));
        }
    }
    shell_path(rails, command, workspace)
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

/// A nested script Dots reads: kept for the mentions, then read as a line.
fn script(rails: &Guardrails, text: &str, depth: usize, walk: &mut Walk) {
    walk.texts.push(text.to_string());
    line(rails, text, depth + 1, walk);
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
                script(rails, w, depth, walk);
            }
        }
        return;
    }
    if SHELLS.contains(&name.as_str()) {
        let mut it = words.iter().skip(1);
        let mut ran = false;
        while let Some(w) = it.next() {
            let is_c =
                w == "-c" || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('c'));
            if is_c {
                if let Some(text) = it.next() {
                    script(rails, text, depth, walk);
                    ran = true;
                }
                break;
            }
            if !w.starts_with('-') {
                // A script file: not read.
                walk.unclear = true;
                ran = true;
                break;
            }
        }
        if !ran {
            // Reads its commands from the pipe.
            walk.blind = true;
        }
        return;
    }
    if name == "cmd" {
        match words.iter().position(|w| {
            w.get(..2)
                .is_some_and(|h| ["/c", "/k", "/r"].contains(&h.to_lowercase().as_str()))
        }) {
            Some(pos) => {
                let head = &words[pos][2..];
                let mut rest: Vec<&str> = Vec::new();
                if !head.is_empty() {
                    rest.push(head);
                }
                rest.extend(words[pos + 1..].iter().map(String::as_str));
                script(rails, &rest.join(" "), depth, walk);
            }
            None => walk.blind = true,
        }
        return;
    }
    if name == "powershell" || name == "pwsh" {
        powershell(rails, words, depth, walk);
        return;
    }
    if name == "eval" || name == "iex" || name == "invoke-expression" {
        let rest: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        if rest.is_empty() {
            walk.blind = true;
            return;
        }
        if rest.iter().any(|w| w.split_whitespace().any(is_variable)) {
            walk.unclear = true;
        }
        script(rails, &rest.join(" "), depth, walk);
        return;
    }
    if INTERPRETERS.contains(&name.as_str()) {
        interpreter(&name, words, walk);
        return;
    }
    if CD_COMMANDS.contains(&name.as_str()) {
        walk.moved = true;
        let target = words
            .iter()
            .skip(1)
            .find(|w| !w.starts_with('-') && !w.starts_with('/'));
        let ws = walk.workspace.clone();
        if let Some(shown) = target.and_then(|t| protected_path(rails, t, ws.as_deref(), true)) {
            walk.hits.push(RailHit {
                kind: RailKind::Path,
                what: format!("the folder {shown}"),
            });
        }
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

/// An interpreter: inline code (`-c`, `-e`, `--eval`) is kept for the mentions; a script file
/// is not read. Either way the line is unclear, except `bun test` and its kin.
fn interpreter(name: &str, words: &[String], walk: &mut Walk) {
    let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();
    if name == "bun"
        && (matches!(args.first(), Some(&("test" | "install" | "add" | "remove")))
            || args.starts_with(&["run", "test"]))
    {
        return;
    }
    walk.unclear = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if ["-c", "-e", "--eval", "-p", "--print", "-r"].contains(a) {
            if let Some(code) = it.next() {
                walk.texts.push(code.to_string());
            }
        }
    }
    if args.is_empty() {
        walk.blind = true;
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
                Some(text) => script(rails, &text, depth, walk),
                None => walk.unclear = true,
            }
            return;
        }
        if prefix_of("-command", 2) {
            let rest: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
            if rest.is_empty() || rest == ["-"] {
                walk.blind = true;
            } else {
                script(rails, &rest.join(" "), depth, walk);
            }
            return;
        }
        if w.starts_with('-') && prefix_of("-file", 2) {
            walk.unclear = true;
            return;
        }
        if w.starts_with('-') {
            i += 1;
            continue;
        }
        let rest: Vec<&str> = words[i..].iter().map(String::as_str).collect();
        script(rails, &rest.join(" "), depth, walk);
        return;
    }
    // No command at all: it reads one from the pipe.
    walk.blind = true;
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

/// The long options `flag` stands for: itself when it is one, else every option it is a prefix
/// of, as git accepts (`--force-w` is `--force-with-lease`). The value after `=` is ignored.
fn long_options<'a>(flag: &str, options: &[&'a str]) -> Vec<&'a str> {
    let flag = flag.split('=').next().unwrap_or(flag);
    if let Some(exact) = options.iter().find(|o| **o == flag) {
        return vec![exact];
    }
    if flag.len() < 3 {
        return Vec::new();
    }
    options
        .iter()
        .filter(|o| o.starts_with(flag))
        .copied()
        .collect()
}

fn git(rails: &Guardrails, words: &[String], walk: &mut Walk) {
    let mut i = 1;
    while i < words.len() && words[i].starts_with('-') {
        let w = words[i].as_str();
        if ["-C", "--git-dir", "--work-tree", "--namespace"].contains(&w)
            || w.starts_with("--git-dir=")
            || w.starts_with("--work-tree=")
        {
            walk.moved = true;
        }
        if w == "-c"
            && words
                .get(i + 1)
                .is_some_and(|kv| kv.to_lowercase().starts_with("alias."))
        {
            // A config for this command only: an alias makes the command unreadable.
            walk.unclear = true;
        }
        i += if ["-C", "-c", "--git-dir", "--work-tree", "--namespace"].contains(&w) {
            2
        } else {
            1
        };
    }
    let Some(sub) = words.get(i).map(|s| s.to_lowercase()) else {
        return;
    };
    if !GIT_BUILTINS.contains(&sub.as_str()) {
        walk.unclear = true;
        return;
    }
    let args: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
    if args.iter().any(|a| *a == "--help" || *a == "-h") {
        return;
    }
    if sub == "config" && args.iter().any(|a| a.to_lowercase().starts_with("alias.")) {
        walk.unclear = true;
    }
    let mut found: Vec<GitAction> = Vec::new();
    match sub.as_str() {
        "push" => push(&args, walk, &mut found),
        "branch" => {
            let delete = args.iter().any(|a| {
                (a.starts_with("--") && long_options(a, BRANCH_OPTIONS).contains(&"--delete"))
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

/// Whether a push to `dst` (or to the branch checked out, for `HEAD`) lands on main or master.
/// The checked-out branch counts only while the line has not moved elsewhere.
fn lands_on_main(dst: &str, walk: &Walk) -> bool {
    let dst = dst.to_lowercase();
    match dst.as_str() {
        "main" | "master" => true,
        "head" | "@" | "" => {
            walk.moved
                || walk
                    .head
                    .as_deref()
                    .is_none_or(|b| ["main", "master"].contains(&b.to_lowercase().as_str()))
        }
        _ => false,
    }
}

/// What a `git push` does of the protected git actions, in this order: force, delete, tags,
/// pushing to main or master.
fn push(args: &[&str], walk: &Walk, found: &mut Vec<GitAction>) {
    let mut long: Vec<&str> = Vec::new();
    let mut short: Vec<&str> = Vec::new();
    let mut operands: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a.starts_with("--") {
            let options = long_options(a, PUSH_OPTIONS);
            if !a.contains('=')
                && !options.is_empty()
                && options.iter().all(|o| PUSH_VALUED.contains(o))
            {
                it.next();
            }
            long.extend(options);
        } else if *a == "-o" {
            it.next();
        } else if a.starts_with('-') && a.len() > 1 {
            short.push(a);
        } else {
            operands.push(a);
        }
    }
    let has = |o: &str| long.contains(&o);
    let cluster = |c: char| short.iter().any(|f| f[1..].contains(c));
    let refspecs: Vec<&str> = operands.iter().skip(1).copied().collect();
    let force = has("--force")
        || has("--force-with-lease")
        || has("--force-if-includes")
        || cluster('f')
        || refspecs.iter().any(|r| r.starts_with('+'));
    let delete = has("--delete")
        || cluster('d')
        || refspecs.iter().any(|r| r.starts_with(':') && r.len() > 1);
    let tag_word = refspecs.iter().position(|r| *r == "tag");
    let tags = has("--tags")
        || has("--follow-tags")
        || tag_word.is_some()
        || refspecs.iter().any(|r| r.contains("refs/tags/"));
    let all = has("--all") || has("--mirror") || has("--branches");
    let named: Vec<&str> = refspecs
        .iter()
        .enumerate()
        .filter(|(i, _)| !matches!(tag_word, Some(t) if *i == t || *i == t + 1))
        .map(|(_, r)| *r)
        .collect();
    let to_main = named.iter().any(|r| {
        let r = r.trim_start_matches('+');
        if r.contains('*') || r == ":" {
            return true;
        }
        let dst = r.rsplit_once(':').map_or(r, |(_, d)| d);
        let dst = dst
            .strip_prefix("refs/heads/")
            .or_else(|| dst.strip_prefix("heads/"))
            .unwrap_or(dst);
        lands_on_main(dst, walk)
    });
    // A push that names no branch pushes the one checked out.
    let unnamed = named.is_empty() && !tags && !delete && lands_on_main("HEAD", walk);
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
    if pair("alias", &["set", "import"]) {
        walk.unclear = true;
    }
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
    if let Some(at) = words.iter().position(|w| w == "api") {
        if let Some(hit) = gh_api(rails, &words[at + 1..], &mut found) {
            walk.hits.push(hit);
        }
    }
    for action in found {
        if rails.protected_git.contains(&action) {
            walk.hits.push(RailHit::git(action));
        }
    }
}

/// `gh api <endpoint> ...`: the protected actions its endpoint and method make, by the
/// endpoint's shape. A GraphQL mutation is a hit of its own.
fn gh_api(rails: &Guardrails, args: &[String], found: &mut Vec<GitAction>) -> Option<RailHit> {
    const VALUED: &[&str] = &[
        "-H",
        "--header",
        "-q",
        "--jq",
        "-t",
        "--template",
        "--hostname",
        "-p",
        "--preview",
        "--cache",
    ];
    const FIELDS: &[&str] = &["-f", "-F", "--field", "--raw-field"];
    let mut method: Option<String> = None;
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut input = false;
    let mut endpoint: Option<String> = None;
    let mut it = args.iter();
    while let Some(w) = it.next() {
        let w = w.as_str();
        let (flag, inline) = match w.split_once('=') {
            Some((f, v)) if w.starts_with("--") => (f, Some(v.to_string())),
            _ => (w, None),
        };
        if flag == "-X" || flag == "--method" {
            method = inline
                .or_else(|| it.next().cloned())
                .map(|m| m.to_uppercase());
        } else if let Some(m) = flag.strip_prefix("-X").filter(|m| !m.is_empty()) {
            method = Some(m.to_uppercase());
        } else if FIELDS.contains(&flag) {
            let kv = inline.or_else(|| it.next().cloned()).unwrap_or_default();
            let (k, v) = kv.split_once('=').unwrap_or((kv.as_str(), ""));
            fields.push((k.to_lowercase(), v.to_string()));
        } else if flag == "--input" {
            input = true;
            if inline.is_none() {
                it.next();
            }
        } else if VALUED.contains(&flag) {
            if inline.is_none() {
                it.next();
            }
        } else if w.starts_with('-') {
            continue;
        } else if endpoint.is_none() {
            endpoint = Some(w.to_lowercase());
        }
    }
    let endpoint = endpoint?;
    let method = method.unwrap_or_else(|| {
        if fields.is_empty() && !input {
            "GET".into()
        } else {
            "POST".into()
        }
    });
    if endpoint.trim_start_matches('/') == "graphql" {
        let query = fields
            .iter()
            .filter(|(k, _)| k == "query")
            .map(|(_, v)| v.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        let mutation = query.contains("mutation") || (input && query.is_empty());
        if mutation && !rails.protected_git.is_empty() {
            return Some(RailHit {
                kind: RailKind::Git,
                what: "changing GitHub with a GraphQL mutation".into(),
            });
        }
        return None;
    }
    let path = endpoint
        .trim_start_matches('/')
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_string();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let rest: &[&str] = if segs.first() == Some(&"repos") && segs.len() >= 3 {
        &segs[3..]
    } else {
        &segs
    };
    let writes = !matches!(method.as_str(), "GET" | "HEAD");
    let field = |k: &str| {
        fields
            .iter()
            .find(|(f, _)| f == k)
            .map(|(_, v)| v.to_lowercase())
    };
    let main = |b: &str| ["main", "master"].contains(&b);
    match rest {
        ["pulls", _, "merge"] if writes => found.push(GitAction::MergePr),
        ["merges"] if writes => found.push(GitAction::MergePr),
        ["releases", ..] if writes => found.push(GitAction::Release),
        ["git", "refs", "heads", branch @ ..] if method == "DELETE" && !branch.is_empty() => {
            found.push(GitAction::DeleteBranch)
        }
        ["git", "refs", "heads", branch @ ..] if writes && main(&branch.join("/")) => {
            if field("force").as_deref() == Some("true") {
                found.push(GitAction::ForcePush);
            }
            found.push(GitAction::PushMain);
        }
        ["git", "refs", "tags", ..] | ["git", "tags", ..] if writes => found.push(GitAction::Tags),
        ["git", "refs"] if writes => match field("ref") {
            Some(r) if r.starts_with("refs/tags/") => found.push(GitAction::Tags),
            Some(r) if main(r.trim_start_matches("refs/heads/")) => found.push(GitAction::PushMain),
            _ => {}
        },
        ["contents", ..] if matches!(method.as_str(), "PUT" | "DELETE") => {
            found.push(GitAction::PushMain)
        }
        _ => {}
    }
    None
}

/// The protected GitHub action an MCP tool's name does (`merge_pull_request`,
/// `create_release`, `push_files`), if it is protected.
fn mcp_name(rails: &Guardrails, name: &str) -> Option<RailHit> {
    let words = name_words(name);
    let has = |w: &str| words.iter().any(|x| x == w);
    let merge = has("merge") && (has("pull") || has("pr") || has("request") || has("prs"));
    let auto_merge = has("auto") && has("merge");
    let release = (has("release") || has("releases"))
        && [
            "create", "publish", "delete", "update", "edit", "upload", "new",
        ]
        .iter()
        .any(|v| has(v));
    let commits = (has("push") && has("files"))
        || (has("file")
            && ["create", "update", "delete", "write"]
                .iter()
                .any(|v| has(v)))
        || (has("update") && has("ref"));
    let deletes_branch = has("delete") && (has("branch") || has("ref"));
    let tags = (has("create") && (has("tag") || has("ref"))) || (has("tag") && has("delete"));
    let repo = has("delete") && (has("repository") || has("repo"));
    if repo && !rails.protected_git.is_empty() {
        return Some(RailHit {
            kind: RailKind::Git,
            what: "deleting a repository".into(),
        });
    }
    [
        (merge || auto_merge, GitAction::MergePr),
        (release, GitAction::Release),
        (deletes_branch, GitAction::DeleteBranch),
        (tags, GitAction::Tags),
        (commits, GitAction::PushMain),
    ]
    .into_iter()
    .find(|(hit, action)| *hit && rails.protected_git.contains(action))
    .map(|(_, action)| RailHit::git(action))
}

/// A word of `text` naming something protected, for a line Dots can't read: a git action's
/// word, a protected command's name, or a protected path's literal part. For a Member, `git`
/// and `gh` themselves count.
fn mentions(rails: &Guardrails, role: GuardRole, text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || "-_.".contains(c)))
        .filter(|t| !t.is_empty())
        .collect();
    if !rails.protected_git.is_empty() {
        if let Some(w) = GIT_WORDS.iter().find(|w| tokens.contains(w)) {
            return Some(w.to_string());
        }
        if role == GuardRole::Member {
            if let Some(w) = ["git", "gh"].iter().find(|w| tokens.contains(w)) {
                return Some(w.to_string());
            }
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
    path_mention(rails, &lower)
}

/// A protected path's literal part that `lower` (lower case) contains.
fn path_mention(rails: &Guardrails, lower: &str) -> Option<String> {
    path_literals(rails)
        .into_iter()
        .filter(|p| p.len() >= 3)
        .find(|p| lower.contains(p.as_str()))
}

// ---- the words of a shell line ---------------------------------------------------------------

/// A word of a shell line with its quotes undone.
struct Word {
    text: String,
    /// It holds something the shell expands or escapes (`$`, a backtick, a glob, `{`, `~`, `%`,
    /// an escape), so its real name is unknown.
    special: bool,
    /// It holds a glob character outside quotes.
    glob: bool,
}

/// The words of `line`, redirection targets included, with their quotes undone.
fn line_words(line: &str) -> Vec<Word> {
    let chars: Vec<char> = line.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let (mut special, mut glob) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                started = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                started = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '$' || chars[i] == '`' {
                        special = true;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                started = true;
                let next = chars.get(i + 1).copied();
                if !next.is_some_and(|n| n.is_alphanumeric() || "._-$~".contains(n)) {
                    special = true;
                }
                cur.push('\\');
            }
            c if c.is_whitespace() || ";|&()<>".contains(c) => {
                if started {
                    words.push(Word {
                        text: std::mem::take(&mut cur),
                        special,
                        glob,
                    });
                }
                started = false;
                special = false;
                glob = false;
            }
            c => {
                started = true;
                if "$`{~%".contains(c) {
                    special = true;
                }
                if "*?[".contains(c) {
                    special = true;
                    glob = true;
                }
                cur.push(c);
            }
        }
        i += 1;
    }
    if started {
        words.push(Word {
            text: cur,
            special,
            glob,
        });
    }
    words
}

/// A protected path that a shell line names, unless the line only reads. A word whose name
/// Dots can't know (a glob, a variable) is a hit when it could be a protected name.
fn shell_path(rails: &Guardrails, command: &str, workspace: Option<&Path>) -> Option<RailHit> {
    if rails.protected_paths.is_empty() || reads_only(command, 0) {
        return None;
    }
    let path_hit = |shown: String| RailHit {
        kind: RailKind::Path,
        what: format!("the file {shown}"),
    };
    let has_git = split_line(command)
        .0
        .iter()
        .any(|w| w.first().is_some_and(|f| command_name(f) == "git"));
    let literals = path_literals(rails);
    for word in line_words(command) {
        let text = word.text.trim();
        if text.is_empty() {
            continue;
        }
        // An 8.3 name in a git line is more likely a revision (`HEAD~1`).
        let short = !has_git || text.contains(['/', '\\', '.']);
        let mut parts: Vec<&str> = vec![text];
        parts.extend(
            text.split(|c: char| c == '=' || c == ',' || c.is_whitespace())
                .filter(|p| !p.is_empty() && *p != text),
        );
        for part in parts {
            if is_flag(part) {
                continue;
            }
            if let Some(shown) = protected_path(rails, part, workspace, short) {
                return Some(path_hit(shown));
            }
        }
        if !word.special {
            continue;
        }
        let lower = text.to_lowercase().replace('\\', "/");
        if word.glob {
            let could_be = lower
                .split('/')
                .filter(|seg| seg.contains(['*', '?', '[']))
                .any(|seg| literals.iter().any(|lit| glob_matches(seg, lit)));
            if could_be {
                return Some(RailHit::unclear(format!(
                    "a file Dots can't name ({text}) that could be protected"
                )));
            }
        }
        if let Some(part) = path_mention(rails, &lower) {
            return Some(RailHit::unclear(format!(
                "a file Dots can't name ({text}) that mentions {part}"
            )));
        }
    }
    None
}

/// Whether every command of `line`, and of its substitutions, only reads, with no `>`.
fn reads_only(line: &str, depth: usize) -> bool {
    if depth > MAX_DEPTH || line.contains('>') {
        return false;
    }
    let (commands, subs) = split_line(line);
    commands.iter().all(|w| reads(w)) && subs.iter().all(|s| reads_only(s, depth + 1))
}

/// Whether the simple command only reads.
fn reads(words: &[String]) -> bool {
    let Some(first) = words.first() else {
        return true;
    };
    let name = command_name(first);
    if name == "git" {
        let writes = words.iter().any(|w| w.starts_with("--output") || w == "-o");
        let sub = words
            .iter()
            .skip(1)
            .find(|w| !w.starts_with('-'))
            .map(|w| w.to_lowercase());
        return !writes && sub.is_some_and(|s| READ_GIT.contains(&s.as_str()));
    }
    READ_COMMANDS.contains(&name.as_str())
}

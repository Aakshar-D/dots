//! The private-folder guard (approval guards spec section 3): the fixed list of places that hold
//! keys, tokens and credentials, and whether a call or a rule pattern touches one.
//!
//! Matching is lexical and fails closed: case-insensitive, `/` and `\` alike, the home folder's
//! entries matched as path parts anywhere in a path. A shell command can still build a path at
//! run time; as for deny rules, the worktree and the CLI lists stay the real boundary.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use super::{glob_matches, input_path, lexical_normalize, COMMAND_TOOLS, FILE_TOOLS};

/// Why no rule can allow a call that touches a private place.
pub const PRIVATE_REFUSAL: &str =
    "this touches a private folder, so it always asks: no rule can allow it";

/// Private places as path parts, in their usual case. Each matches wherever its parts appear in
/// a path, in any case.
const PLACES: &[&[&str]] = &[
    &[".ssh"],
    &[".aws"],
    &[".gnupg"],
    &[".kube"],
    &[".docker"],
    &[".config", "gh"],
    &[".azure"],
    &[".claude", ".credentials.json"],
    &[".netrc"],
    &[".npmrc"],
    &[".pypirc"],
    &[".git-credentials"],
    &["Microsoft", "Credentials"],
    &["Microsoft", "Protect"],
];

/// Names a wildcard part (`.env*`, `.ss?`) is tried against.
const HIDDEN_NAMES: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".credentials.json",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".env",
    ".env.local",
];

/// The folder variables expanded before matching. Only these: another variable (`PATH`) could
/// name anything.
const FOLDER_VARS: &[&str] = &[
    "userprofile",
    "home",
    "appdata",
    "localappdata",
    "dots_data_dir",
];

/// Characters that separate the paths of a command.
const SEPARATORS: &[char] = &[';', '|', '&', '(', ')', '{', '}', '<', '>', ',', '='];

fn vaults() -> &'static Mutex<Vec<String>> {
    static VAULTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    VAULTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Adds Dots' vault folder (`<data dir>/vault`) to the private places. A runtime registers its
/// own when it starts; a relative path is ignored.
pub fn register_vault(path: &Path) {
    let (abs, key) = lexical_normalize(&path.to_string_lossy());
    if abs {
        let mut v = vaults().lock().unwrap();
        if !v.contains(&key) {
            v.push(key);
        }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn home() -> Option<String> {
    if cfg!(windows) {
        var("USERPROFILE").or_else(|| var("HOME"))
    } else {
        var("HOME")
    }
}

fn folder_var(name: &str) -> Option<String> {
    match name {
        "home" if cfg!(windows) => home(),
        _ => var(&name.to_ascii_uppercase()),
    }
}

/// Lower-cases `text`, drops quotes and escape characters, and expands `~` and the folder
/// variables in their `%NAME%`, `$env:NAME`, `${NAME}` and `$NAME` forms.
fn expand(text: &str) -> String {
    let mut s: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '`' | '^'))
        .collect();
    for name in FOLDER_VARS {
        let Some(value) = folder_var(name) else {
            continue;
        };
        let value = value.to_lowercase();
        for form in [
            format!("%{name}%"),
            format!("$env:{name}"),
            format!("${{{name}}}"),
        ] {
            s = s.replace(&form, &value);
        }
        // `$name` only where the name ends: `$homepath` is another variable.
        let form = format!("${name}");
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(i) = rest.find(&form) {
            let after = &rest[i + form.len()..];
            let ends = !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
            out.push_str(&rest[..i]);
            out.push_str(if ends { &value } else { &form });
            rest = after;
        }
        out.push_str(rest);
        s = out;
    }
    if let Some(home) = home() {
        let home = home.to_lowercase();
        let mut out = String::with_capacity(s.len());
        let mut prev: Option<char> = None;
        let chars: Vec<char> = s.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            let starts = prev.is_none_or(|p| p.is_whitespace() || SEPARATORS.contains(&p));
            let ends = chars
                .get(i + 1)
                .is_none_or(|n| matches!(n, '/' | '\\') || n.is_whitespace());
            if c == '~' && starts && ends {
                out.push_str(&home);
            } else {
                out.push(c);
            }
            prev = Some(c);
        }
        s = out;
    }
    s
}

/// The parts of an expanded path: split at `/`, `\` and `:`, trailing dots and spaces dropped.
fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\', ':'])
        .map(|p| p.trim_end_matches(['.', ' ']))
        .filter(|p| !p.is_empty())
        .collect()
}

fn is_env_file(part: &str) -> bool {
    part == ".env" || part.starts_with(".env.")
}

/// True when the expanded, lower-case `path` names a private place or something inside one.
fn expanded_path_is_private(path: &str) -> bool {
    let parts = parts(path);
    let named = PLACES.iter().any(|place| {
        parts.windows(place.len()).any(|w| {
            w.iter()
                .zip(place.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        })
    });
    let wildcard = |p: &str| {
        p.starts_with('.')
            && p.contains(['*', '?', '['])
            && HIDDEN_NAMES.iter().any(|n| glob_matches(p, n))
    };
    if named || parts.iter().any(|p| is_env_file(p) || wildcard(p)) {
        return true;
    }
    let (abs, normalized) = lexical_normalize(path);
    abs && vaults()
        .lock()
        .unwrap()
        .iter()
        .any(|v| inside(&normalized, v))
}

/// `path` is `dir` or below it (both normalized).
fn inside(path: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// True when one path (a file tool's path, a Glob pattern, a rule's path) names a private place.
fn path_is_private(raw: &str) -> bool {
    expanded_path_is_private(&expand(raw))
}

/// True when a command's text mentions a private place.
fn text_is_private(text: &str) -> bool {
    let s = expand(text);
    if s.split(|c: char| c.is_whitespace() || SEPARATORS.contains(&c))
        .any(expanded_path_is_private)
    {
        return true;
    }
    // A vault path with a space in it is split above: look for it in the whole text too.
    let flat = s.replace('\\', "/");
    vaults().lock().unwrap().iter().any(|v| flat.contains(v.as_str()))
}

/// True when a call of `tool` touches a private place: a file tool's path (or its Glob/Grep
/// pattern) names one, or a shell command mentions one. A relative path is also judged inside
/// `workspace`, and a call without a path is judged at `workspace` itself. Other tools are
/// never judged.
pub fn touches_private(tool: &str, input: &Value, workspace: Option<&Path>) -> bool {
    let tool = tool.trim();
    if COMMAND_TOOLS.contains(&tool) {
        return input
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(text_is_private);
    }
    if !FILE_TOOLS.contains(&tool) {
        return false;
    }
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let mut paths: Vec<&str> = input_path(input).into_iter().collect();
    let extra = match tool {
        "Glob" => field("pattern"),
        "Grep" => field("glob"),
        _ => None,
    };
    let has_path = !paths.is_empty();
    paths.extend(extra);
    let ws = workspace.map(|w| w.to_string_lossy().into_owned());
    if !has_path {
        if let Some(ws) = &ws {
            paths.push(ws);
        }
    }
    paths.iter().any(|raw| {
        if path_is_private(raw) {
            return true;
        }
        let expanded = expand(raw);
        let relative = !lexical_normalize(&expanded).0;
        relative
            && ws
                .as_deref()
                .is_some_and(|ws| expanded_path_is_private(&expand(&format!("{ws}/{expanded}"))))
    })
}

/// The private folders on this machine as absolute, normalized paths: the home folder's
/// entries, the Windows credential folders and the registered vaults.
fn concrete_places() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |base: &str, rest: &str| {
        let (abs, p) = lexical_normalize(&format!("{base}/{rest}"));
        if abs {
            out.push(p);
        }
    };
    if let Some(home) = home() {
        for place in PLACES.iter().filter(|p| p[0].starts_with('.')) {
            add(&home, &place.join("/"));
        }
    }
    for base in ["APPDATA", "LOCALAPPDATA"].into_iter().filter_map(var) {
        for place in PLACES.iter().filter(|p| p[0] == "Microsoft") {
            add(&base, &place.join("/"));
        }
    }
    out.extend(vaults().lock().unwrap().iter().cloned());
    out
}

/// True when a rule pattern names a private place (`Read(~/.ssh/**)`, `Bash(cat .env)`) or
/// covers one with `dir/**` (`Read(C:/Users/me/**)` covers `~/.ssh`). A bare tool name never
/// does.
pub fn pattern_touches_private(pattern: &str) -> bool {
    let pattern = pattern.trim();
    let Some((tool, spec)) = pattern
        .strip_suffix(')')
        .and_then(|p| p.split_once('('))
    else {
        return false;
    };
    if COMMAND_TOOLS.contains(&tool) {
        return text_is_private(spec.strip_suffix(":*").unwrap_or(spec));
    }
    if path_is_private(spec) {
        return true;
    }
    let Some(dir) = spec.trim().strip_suffix("/**") else {
        return false;
    };
    let (abs, dir) = lexical_normalize(&expand(dir));
    abs && concrete_places().iter().any(|place| inside(place, &dir))
}

/// The CLI's ask rules for the private places (spec section 3.4): an ask rule beats an allow
/// rule in the CLI, which then asks Dots' `approve` tool, and `Policy::resolve_in` decides.
/// Broad on purpose: Dots' own matching is the precise one.
pub fn cli_private_ask_rules() -> Vec<String> {
    let mut paths: Vec<String> = PLACES.iter().map(|p| p.join("/")).collect();
    paths.extend([".env".to_string(), ".env.*".to_string()]);
    let vault_tails: Vec<String> = vaults()
        .lock()
        .unwrap()
        .iter()
        .filter_map(|v| {
            let mut it = v.rsplit('/');
            let (last, parent) = (it.next()?, it.next()?);
            Some(format!("{parent}/{last}"))
        })
        .collect();
    paths.extend(vault_tails.iter().cloned());
    let mut rules = Vec::new();
    // The CLI reads `Edit` rules for every file-editing tool and ignores `Write` ones.
    for tool in ["Read", "Edit"] {
        for p in &paths {
            rules.push(format!("{tool}(//**/{p})"));
            if !p.starts_with(".env") {
                rules.push(format!("{tool}(//**/{p}/**)"));
            }
        }
    }
    let mut words: Vec<String> = PLACES
        .iter()
        .flat_map(|p| [p.join("/"), p.join("\\")])
        .chain([".env".to_string()])
        .chain(
            vault_tails
                .iter()
                .flat_map(|t| [t.clone(), t.replace('/', "\\")]),
        )
        .collect();
    words.dedup();
    for tool in COMMAND_TOOLS {
        for w in &words {
            rules.push(format!("{tool}(*{w}*)"));
        }
    }
    rules
}

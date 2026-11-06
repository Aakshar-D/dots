//! The private-folder guard (approval guards spec section 3): the fixed list of places that hold
//! keys, tokens and credentials, and whether a call or a rule pattern touches one.
//!
//! Matching is lexical and fails closed: case-insensitive, `/` and `\` alike, the home folder's
//! entries matched as path parts anywhere in a path, brace alternations expanded, wildcards
//! tried against the private names. A shell command can still build a path at run time; as for
//! deny rules, the worktree and the CLI lists stay the real boundary.

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

/// Names a wildcard part (`*ssh`, `.env*`, `*.pem`) is tried against: the one-part places and
/// an example of each kind of secret file (`is_secret_file`). A part with no letter or digit
/// (`*`, `*.*`) is not tried, or every search would ask; where it sits decides instead
/// (`wildcard_reaches`).
const WILDCARD_NAMES: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".env",
    ".env.local",
    ".env-local",
    ".envrc",
    "prod.env",
    "server.pem",
    "server.key",
    "client.p12",
    "client.pfx",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
];

/// Secret files by name, wherever they are (spec section 3.1).
fn is_secret_file(part: &str) -> bool {
    part == ".env"
        || part == ".envrc"
        || part.starts_with(".env.")
        || part.starts_with(".env-")
        || (part.len() > 4 && part.ends_with(".env"))
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|ext| part.len() > ext.len() && part.ends_with(ext))
        || ["id_rsa", "id_ed25519", "id_ecdsa"]
            .iter()
            .any(|key| part.starts_with(key))
}

/// The folder variables expanded before matching. Only these: another variable (`PATH`) could
/// name anything.
const FOLDER_VARS: &[&str] = &[
    "userprofile",
    "home",
    "appdata",
    "localappdata",
    "dots_data_dir",
];

/// Characters that separate the words of a command.
const SEPARATORS: &[char] = &[';', '|', '&', '(', ')', '<', '>', '='];
/// Characters a word is also split at, after its brace alternations are expanded.
const SUB_SEPARATORS: &[char] = &['{', '}', ','];

fn vaults() -> &'static Mutex<Vec<String>> {
    static VAULTS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    VAULTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// The registered vault folders. A poisoned lock still holds a usable list.
fn vault_list() -> Vec<String> {
    vaults().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Adds Dots' vault folder (`<data dir>/vault`) to the private places. A runtime registers its
/// own when it starts; a relative path is ignored.
pub fn register_vault(path: &Path) {
    let (abs, key) = lexical_normalize(&path.to_string_lossy());
    if abs {
        let mut v = vaults().lock().unwrap_or_else(|e| e.into_inner());
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
/// variables in their `%NAME%`, `$env:NAME`, `${env:NAME}`, `${NAME}` and `$NAME` forms.
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
            format!("${{env:{name}}}"),
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
            let starts = prev.is_none_or(|p| {
                p.is_whitespace() || SEPARATORS.contains(&p) || SUB_SEPARATORS.contains(&p)
            });
            let ends = chars
                .get(i + 1)
                .is_none_or(|n| matches!(n, '/' | '\\' | ',' | '}') || n.is_whitespace());
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

/// Every reading of `word` with its brace alternations expanded (`.ss{h,}` is `.ssh` and
/// `.ss`), the word itself first. At most 64 readings.
fn brace_variants(word: &str) -> Vec<String> {
    let mut done: Vec<String> = vec![word.to_string()];
    let mut todo: Vec<String> = vec![word.to_string()];
    while let Some(w) = todo.pop() {
        // The first innermost group: a `{` with no `{` before its `}`.
        let Some(close) = w.find('}') else { continue };
        let Some(open) = w[..close].rfind('{') else {
            continue;
        };
        let inner = &w[open + 1..close];
        if !inner.contains(',') {
            continue;
        }
        for alt in inner.split(',') {
            let next = format!("{}{alt}{}", &w[..open], &w[close + 1..]);
            if done.len() >= 64 {
                return done;
            }
            if !done.contains(&next) {
                done.push(next.clone());
                todo.push(next);
            }
        }
    }
    done
}

/// The parts of an expanded path: split at `/`, `\` and `:`, trailing dots and spaces dropped.
fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\', ':'])
        .map(|p| p.trim_end_matches(['.', ' ']))
        .filter(|p| !p.is_empty())
        .collect()
}

fn is_wildcard(part: &str) -> bool {
    part.contains(['*', '?', '['])
}

/// A wildcard part as `glob_matches` reads it: a `[...]` class becomes `?`.
fn as_glob(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    let mut in_class = false;
    for c in part.chars() {
        match c {
            '[' if !in_class => in_class = true,
            ']' if in_class => {
                in_class = false;
                out.push('?');
            }
            _ if in_class => {}
            c => out.push(c),
        }
    }
    out
}

/// True when the parts name a private place, a secret file, or a wildcard that could match one.
fn names_private(parts: &[&str]) -> bool {
    let named = PLACES.iter().any(|place| {
        parts.windows(place.len()).any(|w| {
            w.iter()
                .zip(place.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        })
    });
    let wildcard = |p: &str| {
        is_wildcard(p)
            && p.chars().any(|c| c.is_ascii_alphanumeric())
            && WILDCARD_NAMES.iter().any(|n| glob_matches(&as_glob(p), n))
    };
    named || parts.iter().any(|p| is_secret_file(p) || wildcard(p))
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
    out.extend(vault_list());
    out
}

/// `path` is `dir` or below it (both normalized).
fn inside(path: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// True when a normalized, absolute path with wildcards could reach a private place: below the
/// folder before its first wildcard part lies a private place, and the pattern has `**` or its
/// first wildcard part matches the next part of that place (`~/*/id_rsa` reaches `~/.ssh`).
fn wildcard_reaches(normalized: &str) -> bool {
    let comps: Vec<&str> = normalized.split('/').collect();
    let Some(i) = comps.iter().position(|c| is_wildcard(c)) else {
        return false;
    };
    let base = comps[..i].join("/");
    let deep = comps[i..].iter().any(|c| c.contains("**"));
    let first = as_glob(comps[i]);
    concrete_places().iter().any(|place| {
        let Some(rest) = place.strip_prefix(&format!("{}/", base.trim_end_matches('/'))) else {
            return false;
        };
        let next = rest.split('/').next().unwrap_or("");
        deep || glob_matches(&first, next)
    })
}

/// True when the expanded, lower-case `path` names a private place or something inside one,
/// read as it is and with its `..` resolved.
fn expanded_path_is_private(path: &str) -> bool {
    if names_private(&parts(path)) {
        return true;
    }
    let (abs, normalized) = lexical_normalize(path);
    if names_private(&parts(&normalized)) {
        return true;
    }
    abs && (vault_list().iter().any(|v| inside(&normalized, v)) || wildcard_reaches(&normalized))
}

/// True when one path (a file tool's path, a Glob pattern, a rule's path) names a private place,
/// in any reading of its brace alternations.
fn path_is_private(raw: &str) -> bool {
    brace_variants(&expand(raw))
        .iter()
        .any(|p| expanded_path_is_private(p))
}

/// True when a command's text mentions a private place. Each word is read with its brace
/// alternations expanded, split at braces and commas, and with its backslash escapes removed
/// (`.s\sh`) as well as with `\` as a folder separator.
fn text_is_private(text: &str) -> bool {
    let s = expand(text);
    let words = s.split(|c: char| c.is_whitespace() || SEPARATORS.contains(&c));
    for word in words.filter(|w| !w.is_empty()) {
        let mut readings = brace_variants(word);
        readings.extend(word.split(SUB_SEPARATORS).map(str::to_string));
        for reading in readings {
            let unescaped = reading.replace('\\', "");
            if expanded_path_is_private(&reading) || expanded_path_is_private(&unescaped) {
                return true;
            }
        }
    }
    // A vault path with a space in it is split above: look for it in the whole text too.
    let flat = s.replace('\\', "/");
    vault_list().iter().any(|v| flat.contains(v.as_str()))
}

/// True when the folder a search or listing starts from holds a private place (or is one).
fn holds_private(dir: &str) -> bool {
    let (abs, normalized) = lexical_normalize(dir);
    abs && concrete_places()
        .iter()
        .any(|place| inside(place, &normalized))
}

/// True when a call of `tool` touches a private place: a file tool's path (or its Glob/Grep
/// pattern) names one, or a shell command mentions one. A relative path is also judged inside
/// `workspace`, and a call without a path is judged at `workspace` itself. A search or listing
/// (`Grep`, `Glob`, `LS`) also touches one when its folder holds one (`~`, `~/.config`). Other
/// tools are never judged.
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
    let ws = workspace.map(|w| w.to_string_lossy().into_owned());
    // An expanded path made absolute against the run's folder where it is relative.
    let resolved = |raw: &str| -> Option<String> {
        let expanded = expand(raw);
        if lexical_normalize(&expanded).0 {
            Some(expanded)
        } else {
            ws.as_deref().map(|ws| expand(&format!("{ws}/{expanded}")))
        }
    };
    let path = input_path(input);
    let mut paths: Vec<String> = path.iter().map(|p| p.to_string()).collect();
    match tool {
        // A Glob pattern is read inside its folder.
        "Glob" => paths.extend(field("pattern").map(|pattern| match path {
            Some(dir) if !lexical_normalize(&expand(pattern)).0 => format!("{dir}/{pattern}"),
            _ => pattern.to_string(),
        })),
        "Grep" => paths.extend(field("glob").map(str::to_string)),
        _ => {}
    }
    if path.is_none() {
        paths.extend(ws.clone());
    }
    let named = paths.iter().any(|raw| {
        path_is_private(raw)
            || resolved(raw)
                .filter(|r| r != &expand(raw))
                .is_some_and(|r| path_is_private(&r))
    });
    if named {
        return true;
    }
    let searches = matches!(tool, "Grep" | "Glob" | "LS");
    let dir = path.map(str::to_string).or_else(|| ws.clone());
    searches
        && dir
            .and_then(|d| resolved(&d))
            .is_some_and(|d| holds_private(&d))
}

/// True when a rule pattern names a private place (`Read(~/.ssh/**)`, `Bash(cat .env)`) or
/// covers one with `dir/**` (`Read(C:/Users/me/**)` covers `~/.ssh`). A bare tool name never
/// does.
pub fn pattern_touches_private(pattern: &str) -> bool {
    let pattern = pattern.trim();
    let Some((tool, spec)) = pattern.strip_suffix(')').and_then(|p| p.split_once('(')) else {
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
    let mut dirs: Vec<String> = PLACES.iter().map(|p| p.join("/")).collect();
    let vault_tails: Vec<String> = vault_list()
        .iter()
        .filter_map(|v| {
            let mut it = v.rsplit('/');
            let (last, parent) = (it.next()?, it.next()?);
            Some(format!("{parent}/{last}"))
        })
        .collect();
    dirs.extend(vault_tails.iter().cloned());
    let files = [
        ".env",
        ".env.*",
        ".env-*",
        ".envrc",
        "*.env",
        "*.pem",
        "*.key",
        "*.p12",
        "*.pfx",
        "id_rsa*",
        "id_ed25519*",
        "id_ecdsa*",
    ];
    let mut rules = Vec::new();
    // The CLI reads `Edit` rules for every file-editing tool and ignores `Write` ones.
    for tool in ["Read", "Edit"] {
        for d in &dirs {
            rules.push(format!("{tool}(//**/{d})"));
            rules.push(format!("{tool}(//**/{d}/**)"));
        }
        for f in files {
            rules.push(format!("{tool}(//**/{f})"));
        }
    }
    let mut words: Vec<String> = PLACES
        .iter()
        .flat_map(|p| [p.join("/"), p.join("\\")])
        .chain(
            [
                ".env",
                ".pem",
                ".key",
                ".p12",
                ".pfx",
                "id_rsa",
                "id_ed25519",
                "id_ecdsa",
            ]
            .map(String::from),
        )
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

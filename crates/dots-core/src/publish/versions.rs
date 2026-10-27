//! Release versions and the notes draft (updates UI spec sections 5.3, 5.4 and 5.6): the five
//! version fields edited as text, which version may be published next, and the notes drafted
//! from the merged pull requests.

use super::github::MergedPull;
use crate::{Error, Result};

/// The files that carry Dots' version (updates UI spec section 3).
pub const VERSION_FILES: [&str; 5] = [
    "apps/dots-app/tauri.conf.json",
    "apps/dots-app/Cargo.toml",
    "crates/dots-core/Cargo.toml",
    "ui/package.json",
    "Cargo.lock",
];

const FORMAT: &str = "A version is MAJOR.MINOR.PATCH, for example 0.3.0.";

/// `text` of `path` with its version set to `version`; every other byte kept.
pub fn set_version(path: &str, text: &str, version: &str) -> Result<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let mut changed = 0;
    let set = |line: &mut String, prefix: &str, quote_end: &str| -> bool {
        let Some(start) = line.find(prefix) else {
            return false;
        };
        let value_start = start + prefix.len();
        let Some(len) = line[value_start..].find(quote_end) else {
            return false;
        };
        line.replace_range(value_start..value_start + len, version);
        true
    };
    if path.ends_with(".json") {
        if let Some(line) = lines
            .iter_mut()
            .find(|l| l.trim_start().starts_with("\"version\":"))
        {
            changed += set(line, "\"version\": \"", "\"") as usize;
        }
    } else if path == "Cargo.lock" {
        for package in ["dots-app", "dots-core"] {
            let name = format!("name = \"{package}\"");
            if let Some(at) = lines.iter().position(|l| l.trim_end_matches('\r') == name) {
                if let Some(line) = lines.get_mut(at + 1) {
                    changed += set(line, "version = \"", "\"") as usize;
                }
            }
        }
    } else if let Some(line) = lines.iter_mut().find(|l| l.starts_with("version = \"")) {
        changed += set(line, "version = \"", "\"") as usize;
    }
    let wanted = if path == "Cargo.lock" { 2 } else { 1 };
    if changed != wanted {
        return Err(Error::Other(format!(
            "{path}: the version could not be found"
        )));
    }
    Ok(lines.join("\n"))
}

/// A plain `MAJOR.MINOR.PATCH`.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = v.split('.').collect();
    let [a, b, c] = parts.as_slice() else {
        return None;
    };
    let num = |p: &str| {
        let ok = !p.is_empty()
            && p.bytes().all(|b| b.is_ascii_digit())
            && (p == "0" || !p.starts_with('0'));
        ok.then(|| p.parse().ok()).flatten()
    };
    Some((num(a)?, num(b)?, num(c)?))
}

/// The next minor version after `live` ("0.2.0" when nothing is live).
pub fn next_minor(live: Option<&str>) -> String {
    match live.and_then(parse) {
        Some((major, minor, _)) => format!("{major}.{}.0", minor + 1),
        None => "0.2.0".to_string(),
    }
}

fn tagged(tags: &[String], version: &str) -> bool {
    tags.iter().any(|t| t == &format!("v{version}"))
}

/// The next minor version after `live` that was never tagged.
pub fn next_free_minor(live: Option<&str>, tags: &[String]) -> String {
    let mut next = next_minor(live);
    while tagged(tags, &next) {
        next = next_minor(Some(&next));
    }
    next
}

/// The next patch of `version` that was never tagged (after a failed promote, spec §5.7).
pub fn next_free_patch(version: &str, tags: &[String]) -> String {
    let Some((major, minor, mut patch)) = parse(version) else {
        return version.to_string();
    };
    loop {
        patch += 1;
        let next = format!("{major}.{minor}.{patch}");
        if !tagged(tags, &next) {
            return next;
        }
    }
}

/// Whether `version` may be published: plain, above the live one, and never tagged before.
/// A refusal names a version that may.
pub fn check_new(
    version: &str,
    live: Option<&str>,
    tags: &[String],
) -> std::result::Result<(), String> {
    let wanted = parse(version).ok_or_else(|| FORMAT.to_string())?;
    if let Some(live) = live {
        if parse(live).is_some_and(|l| wanted <= l) {
            return Err(format!(
                "{version} is not above the live {live}; use {}.",
                next_free_minor(Some(live), tags)
            ));
        }
    }
    if tagged(tags, version) {
        return Err(format!(
            "{version} was published before; use {}.",
            next_free_patch(version, tags)
        ));
    }
    Ok(())
}

/// The next trial number of `version`: one above its highest `-rc.<n>` tag.
pub fn next_rc(version: &str, tags: &[String]) -> u32 {
    let prefix = format!("v{version}-rc.");
    tags.iter()
        .filter_map(|t| t.strip_prefix(&prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// One line per pull: "- <title> (#<n>)", a leading "type(scope): " or "type: " removed and
/// the first letter upper-cased.
pub fn draft_notes(pulls: &[MergedPull]) -> String {
    pulls
        .iter()
        .map(|p| format!("- {} (#{})\n", clean_title(&p.title), p.number))
        .collect()
}

fn clean_title(title: &str) -> String {
    let rest = match title.split_once(": ") {
        Some((kind, rest))
            if !kind.is_empty()
                && kind.chars().all(|c| {
                    c.is_ascii_lowercase() || "()-_!".contains(c) || c.is_ascii_digit()
                }) =>
        {
            rest
        }
        _ => title,
    };
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

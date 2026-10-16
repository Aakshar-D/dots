//! The YAML frontmatter of a `SKILL.md` or a command file, read by hand: top-level `key: value`
//! pairs, quoted values, `>` and `|` block scalars. Lists, nested maps and flow collections are
//! skipped. As in Claude Code, a frontmatter that does not parse gives no fields at all.

use std::collections::BTreeMap;

/// A file split into its frontmatter fields and its body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    /// Top-level keys whose value is a scalar. Empty when there is no frontmatter or it is broken.
    pub fields: BTreeMap<String, String>,
    /// What follows the frontmatter; the whole text when there is none.
    pub body: String,
}

fn is_open(line: &str) -> bool {
    line.trim_end() == "---"
}

fn is_close(line: &str) -> bool {
    matches!(line.trim_end(), "---" | "...")
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// Splits `text` into its frontmatter and its body.
pub fn parse_frontmatter(text: &str) -> Frontmatter {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().is_none_or(|l| !is_open(l)) {
        return Frontmatter {
            fields: BTreeMap::new(),
            body: text.to_string(),
        };
    }
    let Some(close) = (1..lines.len()).find(|&i| is_close(lines[i])) else {
        // Never closed: no frontmatter, as in Claude Code.
        return Frontmatter {
            fields: BTreeMap::new(),
            body: lines[1..].join("\n"),
        };
    };
    Frontmatter {
        fields: parse_map(&lines[1..close]).unwrap_or_default(),
        body: lines[close + 1..].join("\n"),
    }
}

/// The key of a top-level line and the text after its colon. `None` when the line is not
/// `key: ...` (a plain key ends at the first `:` followed by a space or the end of the line).
fn split_key(line: &str) -> Option<(String, &str)> {
    if let Some(quote) = line.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let rest = &line[1..];
        let end = rest.find(quote)?;
        let after = rest[end + 1..].trim_start();
        let after = after.strip_prefix(':')?;
        return Some((rest[..end].to_string(), after));
    }
    let mut at = 0;
    while let Some(i) = line[at..].find(':') {
        let colon = at + i;
        let after = &line[colon + 1..];
        if after.is_empty() || after.starts_with([' ', '\t']) {
            let key = line[..colon].trim();
            return (!key.is_empty()).then(|| (key.to_string(), after));
        }
        at = colon + 1;
    }
    None
}

/// `s` without a trailing ` # comment`.
fn without_comment(s: &str) -> &str {
    let mut prev_space = true;
    for (i, c) in s.char_indices() {
        if c == '#' && prev_space {
            return s[..i].trim_end();
        }
        prev_space = c == ' ' || c == '\t';
    }
    s.trim_end()
}

/// A plain scalar may not hold `: ` or end with `:` (YAML refuses it).
fn plain_ok(s: &str) -> bool {
    !(s.contains(": ") || s.contains(":\t") || s.ends_with(':'))
}

fn parse_map(lines: &[&str]) -> Option<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    let mut seen = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if is_blank(line) || is_comment(line) {
            continue;
        }
        if indent(line) > 0 || line.starts_with("- ") || line == "-" {
            return None;
        }
        let (key, rest) = split_key(line)?;
        if seen.contains(&key) {
            return None;
        }
        seen.push(key.clone());
        let rest = rest.trim();
        // The lines after this one that belong to the value.
        let mut end = i;
        let mut value = None;
        match rest.chars().next() {
            None | Some('#') => {
                // A list or a nested map follows, or nothing.
                while end < lines.len()
                    && (is_blank(lines[end])
                        || indent(lines[end]) > 0
                        || lines[end].starts_with("- ")
                        || lines[end] == "-")
                {
                    end += 1;
                }
            }
            Some('|') | Some('>') => {
                while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
                    end += 1;
                }
                value = Some(block_scalar(rest, &lines[i..end]));
            }
            Some('[') | Some('{') => {
                let mut depth = bracket_depth(rest);
                while depth > 0 && end < lines.len() {
                    depth += bracket_depth(lines[end]);
                    end += 1;
                }
            }
            Some(q @ ('"' | '\'')) => {
                let (text, used) = quoted(q, rest, &lines[i..])?;
                end = i + used;
                value = Some(text);
            }
            Some('@') | Some('`') => return None,
            Some('&') | Some('*') | Some('!') => {
                while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
                    end += 1;
                }
            }
            Some(_) => {
                let mut text = without_comment(rest).to_string();
                if !plain_ok(&text) {
                    return None;
                }
                // Indented lines continue a plain scalar.
                while end < lines.len() && (is_blank(lines[end]) || indent(lines[end]) > 0) {
                    let more = lines[end].trim();
                    end += 1;
                    if more.is_empty() || more.starts_with('#') {
                        continue;
                    }
                    let more = without_comment(more);
                    if !plain_ok(more) {
                        return None;
                    }
                    text.push(' ');
                    text.push_str(more);
                }
                value = Some(text);
            }
        }
        if let Some(v) = value {
            fields.insert(key, v);
        }
        i = end;
    }
    Some(fields)
}

fn bracket_depth(s: &str) -> i32 {
    s.chars()
        .map(|c| match c {
            '[' | '{' => 1,
            ']' | '}' => -1,
            _ => 0,
        })
        .sum()
}

/// A `|` or `>` scalar. `>` joins lines with a space and a blank line becomes a line break.
fn block_scalar(header: &str, lines: &[&str]) -> String {
    let folded = header.starts_with('>');
    let base = lines
        .iter()
        .filter(|l| !is_blank(l))
        .map(|l| indent(l))
        .next()
        .unwrap_or(0);
    let mut out = String::new();
    let mut pending_breaks = 0;
    for line in lines {
        if is_blank(line) {
            pending_breaks += 1;
            continue;
        }
        let text = line.get(base.min(line.len())..).unwrap_or("").trim_end();
        if !out.is_empty() {
            if folded {
                out.push_str(&"\n".repeat(pending_breaks));
                if pending_breaks == 0 {
                    out.push(' ');
                }
            } else {
                out.push_str(&"\n".repeat(pending_breaks + 1));
            }
        }
        pending_breaks = 0;
        out.push_str(text);
    }
    out
}

/// A quoted scalar that starts in `first` and may go on in `more`. Returns the text and the
/// number of lines of `more` it used. `None` when the quote is not closed, or text follows it.
fn quoted(quote: char, first: &str, more: &[&str]) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut used = 0;
    let mut line = &first[1..];
    loop {
        let mut chars = line.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if quote == '"' && c == '\\' {
                match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    ' ' => out.push(' '),
                    'u' => {
                        let hex: String =
                            (0..4).filter_map(|_| chars.next().map(|p| p.1)).collect();
                        out.push(
                            u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)?,
                        );
                    }
                    _ => return None,
                }
            } else if c == quote {
                if quote == '\'' && chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                let tail = line[i + c.len_utf8()..].trim();
                return (tail.is_empty() || tail.starts_with('#')).then_some((out, used));
            } else {
                out.push(c);
            }
        }
        // The quote is still open: the next line continues it, joined by a space.
        let next = more.get(used)?;
        used += 1;
        if is_blank(next) {
            out.push('\n');
            line = "";
        } else {
            if !out.ends_with('\n') {
                out.push(' ');
            }
            line = next.trim_start();
        }
    }
}

/// `text` with the top-level field `key` taken out of its frontmatter, whatever its value looks
/// like: a scalar, a block scalar, or a list. It works on the lines, not on the parsed fields, so
/// that a frontmatter Dots reads as broken but Claude Code reads is cleaned too. A text without
/// a closed frontmatter is returned as it is.
pub fn remove_field(text: &str, key: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let opens = lines
        .first()
        .is_some_and(|l| is_open(l.strip_prefix('\u{feff}').unwrap_or(l)));
    let close = (1..lines.len()).find(|&i| is_close(lines[i]));
    let (true, Some(close)) = (opens, close) else {
        return text.to_string();
    };
    let mut out = String::new();
    out.push_str(lines[0]);
    let mut skipping = false;
    let mut blanks: Vec<&str> = Vec::new();
    for line in &lines[1..close] {
        if skipping {
            if is_blank(line) {
                blanks.push(line);
                continue;
            }
            if indent(line) > 0 || line.starts_with("- ") || line.trim_end() == "-" {
                blanks.clear();
                continue;
            }
            skipping = false;
            blanks.drain(..).for_each(|b| out.push_str(b));
        }
        let plain = line.trim_end_matches(['\r', '\n']);
        if indent(plain) == 0
            && !is_comment(plain)
            && split_key(plain).is_some_and(|(k, _)| k == key)
        {
            skipping = true;
            continue;
        }
        out.push_str(line);
    }
    blanks.drain(..).for_each(|b| out.push_str(b));
    for line in &lines[close..] {
        out.push_str(line);
    }
    out
}

/// Fail-closed cleaning: when the frontmatter of `text` is in a form this reader does not follow
/// (an indented key, a flow mapping or explicit key, a quoted key with an escape), YAML parsers
/// may still read a key that `remove_field` cannot find. Returns the text without its frontmatter
/// block then; `None` when the form is understood or there is no closed frontmatter.
pub(crate) fn drop_unsupported_frontmatter(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let opens = lines
        .first()
        .is_some_and(|l| is_open(l.strip_prefix('\u{feff}').unwrap_or(l)));
    let close = (1..lines.len()).find(|&i| is_close(lines[i]))?;
    if !opens {
        return None;
    }
    let mut in_value = false;
    let mut unsupported = false;
    for raw in &lines[1..close] {
        let line = raw.trim_end_matches(['\r', '\n']);
        if is_blank(line) || is_comment(line) {
            continue;
        }
        if in_value && (indent(line) > 0 || line.starts_with("- ") || line == "-") {
            continue;
        }
        if indent(line) > 0
            || line.starts_with(['{', '[', '?'])
            || (line.starts_with('"') && line.split("\":").next().is_some_and(|k| k.contains('\\')))
        {
            unsupported = true;
            break;
        }
        in_value = true;
    }
    unsupported.then(|| lines[close + 1..].concat())
}

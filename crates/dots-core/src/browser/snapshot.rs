//! What Dots reads in the results of Playwright MCP 0.0.83 (browser spec section 3.3): the page
//! a result reports as current, and the role, name and reference of each element of a page
//! snapshot. A result is markdown: `### <section>` lines, each followed by its lines; a
//! snapshot is a fenced `yaml` block of lines such as `- button "Pay now" [ref=e16]`. The
//! formats are pinned by tests on captured output (`tests/common/playwright.rs`).

use std::collections::HashMap;

use serde_json::Value;

/// An element of a snapshot: its role and its accessible name (`""` when it has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub role: String,
    pub name: String,
}

impl Element {
    pub fn new(role: &str, name: &str) -> Element {
        Element {
            role: role.to_string(),
            name: name.to_string(),
        }
    }

    /// What a card and the run view call it: its name, or its role when it has none.
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.role
        } else {
            &self.name
        }
    }
}

/// The elements of a snapshot by their reference (`e16`).
pub type RefMap = HashMap<String, Element>;

/// The tools whose results report the current page in Playwright's own words. The others
/// (console messages, found text, results of code, network requests, waiting for text) answer
/// with what the page or the model wrote, which could imitate a `### Page` section, so they
/// never move the current page.
pub const REPORTS_PAGE: [&str; 16] = [
    "browser_navigate",
    "browser_navigate_back",
    "browser_snapshot",
    "browser_click",
    "browser_type",
    "browser_fill_form",
    "browser_select_option",
    "browser_press_key",
    "browser_hover",
    "browser_drag",
    "browser_drop",
    "browser_handle_dialog",
    "browser_tabs",
    "browser_take_screenshot",
    "browser_resize",
    "browser_close",
];

/// How many pages' maps a run keeps: the newest.
pub const KEPT_PAGES: usize = 8;

/// The sections of a result, in order: each `### ` title with the lines after it.
fn sections(text: &str) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("### ") {
            out.push((title.trim(), Vec::new()));
        } else if let Some((_, lines)) = out.last_mut() {
            lines.push(line);
        }
    }
    out
}

/// The address of the page a result reports as current: the `- Page URL:` line of its first
/// `### Page` section or, without one, the `(current)` tab of its first tab list (`### Open
/// tabs`, or the `### Result` of `browser_tabs`).
pub fn current_url(text: &str) -> Option<String> {
    let sections = sections(text);
    if let Some((_, lines)) = sections.iter().find(|(title, _)| *title == "Page") {
        return lines
            .iter()
            .find_map(|line| line.strip_prefix("- Page URL: "))
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty());
    }
    sections
        .iter()
        .filter(|(title, _)| matches!(*title, "Open tabs" | "Result"))
        .find_map(|(_, lines)| lines.iter().find_map(|line| current_tab(line)))
}

/// The address of a tab line `- 2: (current) [title](url)`. The title may hold `]` or `(`, so
/// the address is what follows the last `](`.
fn current_tab(line: &str) -> Option<String> {
    let rest = line.trim_end().strip_prefix("- ")?;
    let (index, rest) = rest.split_once(": ")?;
    if index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = rest.strip_prefix("(current) [")?;
    let open = rest.rfind("](")?;
    let url = rest[open + 2..].strip_suffix(')')?;
    (!url.is_empty()).then(|| url.to_string())
}

/// The elements of the snapshot in a result's first `### Snapshot` section, a fenced `yaml`
/// block. `None` when the result has no snapshot inline (one saved to a file is a link).
pub fn read_snapshot(text: &str) -> Option<RefMap> {
    let sections = sections(text);
    let (_, lines) = sections.iter().find(|(title, _)| *title == "Snapshot")?;
    let mut lines = lines.iter().map(|line| line.trim_end());
    lines.by_ref().find(|line| *line == "```yaml")?;
    Some(
        lines
            .take_while(|line| *line != "```")
            .filter_map(read_line)
            .collect(),
    )
}

/// The elements `browser_find` reports in its `### Result` section.
pub fn read_found(text: &str) -> RefMap {
    sections(text)
        .into_iter()
        .filter(|(title, _)| *title == "Result")
        .flat_map(|(_, lines)| lines)
        .filter_map(|line| read_line(line.trim_end()))
        .collect()
}

/// One snapshot line, at any depth: `- <role>`, an optional quoted name, attributes in
/// brackets (`[level=1]`, `[active]`, `[ref=e2]`, `[cursor=pointer]`, `[box=...]`), and an
/// optional `:` with text. `None` for a line without a reference (`- text: Search`, `- /url:
/// /a`, `- option "Small" [selected]`) and for one Dots cannot read.
pub fn read_line(line: &str) -> Option<(String, Element)> {
    let rest = line.trim_start().strip_prefix("- ")?;
    let role_end = rest.find([' ', ':']).unwrap_or(rest.len());
    let role = &rest[..role_end];
    if role.is_empty() || !role.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let mut rest = &rest[role_end..];
    let mut name = String::new();
    if let Some(quoted) = rest.strip_prefix(" \"") {
        let (read, after) = read_quoted(quoted)?;
        name = read;
        rest = after;
    }
    let mut reference = None;
    while let Some(attribute) = rest.strip_prefix(" [") {
        let close = attribute.find(']')?;
        if let Some(r) = attribute[..close].strip_prefix("ref=") {
            reference = Some(r.to_string());
        }
        rest = &attribute[close + 1..];
    }
    Some((reference?, Element::new(role, &name)))
}

/// A quoted string after its opening `"`, its escapes read (`\"`, `\\`, `\n`, `\t`). Returns
/// it and what follows its closing quote; `None` when it is not closed.
fn read_quoted(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 1..])),
            '\\' => {
                let (_, next) = chars.next()?;
                out.push(match next {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
            }
            other => out.push(other),
        }
    }
    None
}

/// What a run knows of its pages from the results so far: the page Playwright reported as
/// current last, and the reference map of each page it took a snapshot of (the newest
/// `KEPT_PAGES`). A snapshot of a page replaces that page's map; a part of a page (`target`,
/// `depth`) and found elements add to it; a snapshot saved to a file adds nothing.
#[derive(Debug, Default)]
pub struct PageMemory {
    current: Option<String>,
    maps: Vec<(String, RefMap)>,
    /// The element an act last landed on, where a pressed key goes.
    focus: Option<Element>,
}

impl PageMemory {
    /// The page Playwright reported as current last.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// The element an act last landed on, while the run stays on its page.
    pub fn focus(&self) -> Option<&Element> {
        self.focus.as_ref()
    }

    /// The references known on the page `url`.
    pub fn refs(&self, url: &str) -> Option<&RefMap> {
        self.maps
            .iter()
            .find(|(page, _)| page == url)
            .map(|(_, map)| map)
    }

    /// Records `map` for the page `url`, in place of what it had: Playwright numbers references
    /// afresh for each document, so an older map may name other elements.
    pub fn store(&mut self, url: &str, map: RefMap) {
        self.forget(url);
        self.maps.push((url.to_string(), map));
        if self.maps.len() > KEPT_PAGES {
            self.maps.remove(0);
        }
    }

    /// Forgets the references of the page `url`.
    pub fn forget(&mut self, url: &str) {
        self.maps.retain(|(page, _)| page != url);
    }

    /// Learns from the `tools/call` result `result` of `tool` called with `input`: nothing from
    /// an error, whose text may be a page's dialog message with no real snapshot.
    pub fn learn_result(&mut self, tool: &str, input: &Value, result: &Value) {
        if result.get("isError") == Some(&Value::Bool(true)) {
            return;
        }
        let text = result["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join(
                        "
",
                    )
            })
            .unwrap_or_default();
        self.learn(tool, input, &text);
    }

    /// Learns from the result `text` of `tool` called with `input`. What follows `### Modal
    /// state` is a page's dialog message, written unescaped, and is not read.
    pub fn learn(&mut self, tool: &str, input: &Value, text: &str) {
        let text = match text.find("### Modal state") {
            Some(at) => &text[..at],
            None => text,
        };
        // The element the act lands on, by its reference on the page it was taken on.
        let target = match tool {
            "browser_click" | "browser_type" | "browser_select_option" => {
                input.get("target").and_then(Value::as_str)
            }
            "browser_fill_form" => input
                .get("fields")
                .and_then(Value::as_array)
                .and_then(|fields| fields.last())
                .and_then(|field| field.get("target"))
                .and_then(Value::as_str),
            "browser_press_key"
            | "browser_snapshot"
            | "browser_find"
            | "browser_take_screenshot"
            | "browser_wait_for"
            | "browser_hover" => None,
            _ => {
                self.focus = None;
                None
            }
        };
        if let Some(target) = target {
            self.focus = self
                .current
                .as_deref()
                .and_then(|url| self.refs(url))
                .and_then(|map| map.get(target))
                .cloned();
        }
        if REPORTS_PAGE.contains(&tool) {
            if let Some(url) = current_url(text) {
                self.current = Some(url);
            }
        }
        let to_file = input
            .get("filename")
            .and_then(Value::as_str)
            .is_some_and(|name| !name.trim().is_empty());
        let Some(url) = self.current.clone() else {
            return;
        };
        match tool {
            // A snapshot or a find saved to a file teaches nothing, and the page may have loaded
            // a new document since the last one: its references are forgotten.
            "browser_snapshot" | "browser_find" if to_file => self.forget(&url),
            "browser_snapshot" => match read_snapshot(text) {
                Some(map) => self.store(&url, map),
                None => self.forget(&url),
            },
            "browser_find" => self.store(&url, read_found(text)),
            _ => {}
        }
    }
}

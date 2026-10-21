//! The browser gate (browser spec section 3): pure code that decides each call of a `browser`
//! tool before the dot's other rules, from the call, the references of the page it acts on
//! (`snapshot::PageMemory`) and that page's address, which `RunBrowser` confirms in Edge. It
//! also writes what an approval card shows (`Ask`), the rules the card offers, and the line the
//! run view shows for a performed call.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sites::{check_site, place, rule_site, site_matches, site_of};
use super::snapshot::{Element, RefMap};
use crate::policy::{Action, Policy, RuleCandidate};

/// An act is risky when its target's name or role holds one of these as a whole word (section
/// 3.4).
pub const RISKY_WORDS: [&str; 23] = [
    "pay",
    "payment",
    "buy",
    "purchase",
    "checkout",
    "order",
    "send",
    "delete",
    "remove",
    "submit",
    "transfer",
    "confirm",
    "publish",
    "post",
    "sign",
    "subscribe",
    "unsubscribe",
    "cancel",
    "withdraw",
    "donate",
    "book",
    "reserve",
    "place",
];

/// Typing into an element whose name holds one of these as a whole word is refused (section
/// 3.5).
pub const SECRET_WORDS: [&str; 7] = [
    "password",
    "passcode",
    "pin",
    "one-time",
    "otp",
    "verification code",
    "security code",
];

/// The answer to typing a password or a code. No rule allows it.
/// The answer to `browser_run_code_unsafe`, which no rule allows.
pub const RUN_CODE_REFUSAL: &str = "Dots does not run code outside the page. Use the page's own controls, or browser_evaluate for code in the page.";

pub const PASSWORD_REFUSAL: &str = "Dots does not type passwords or codes. Ask the person to enter it in the browser window, with your ask tool.";

/// The key under which an approval's input carries its card (`Ask`). Dots writes it; a key of
/// this name in the model's own input is dropped before anything else reads the input.
pub const ASK_KEY: &str = "dots_browser";

/// The read tools of the table in section 3.1. `browser_tabs` is one unless it opens a URL,
/// and `browser_handle_dialog` is one when it dismisses the dialog.
pub const READ_TOOLS: [&str; 10] = [
    "browser_snapshot",
    "browser_take_screenshot",
    "browser_wait_for",
    "browser_navigate_back",
    "browser_console_messages",
    "browser_find",
    "browser_resize",
    "browser_emulate_media",
    "browser_close",
    "browser_tabs",
];

/// What an act does to the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Click,
    Type,
    Fill,
    Select,
    Press,
    Hover,
    Drag,
    Drop,
}

impl Act {
    /// The card's word for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Act::Click => "click",
            Act::Type => "type",
            Act::Fill => "fill",
            Act::Select => "select",
            Act::Press => "press",
            Act::Hover => "hover",
            Act::Drag => "drag",
            Act::Drop => "drop",
        }
    }
}

/// What a dangerous call does: no rule allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Danger {
    /// Runs code in the page: `browser_evaluate`, `browser_run_code_unsafe`, a `javascript:`
    /// address.
    RunCode,
    /// Sends files from the PC: `browser_file_upload`, `browser_drop` with `paths`.
    Upload,
    /// Reads the page's network traffic.
    Network,
    /// A tool the table does not name.
    Other,
}

impl Danger {
    /// The card's word for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Danger::RunCode => "run_code",
            Danger::Upload => "upload",
            Danger::Network => "network",
            Danger::Other => "other",
        }
    }
}

/// The kind of a call (section 3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Read,
    /// Goes to `url`: `browser_navigate`, and `browser_tabs` `new` with a URL.
    Open {
        url: String,
    },
    /// Acts on the current page: on the elements `targets` (references or selectors, as the
    /// model gave them), typing into `typed`; `key` for `browser_press_key`.
    Act {
        act: Act,
        targets: Vec<String>,
        typed: Vec<String>,
        key: Option<String>,
    },
    /// Accepts the page's dialog: always risky.
    AcceptDialog,
    Dangerous(Danger),
}

/// The kind of a call of `tool` (the name without `mcp__browser__`) with `input`.
pub fn classify(tool: &str, input: &Value) -> Kind {
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let act = |act: Act, targets: Vec<String>, typed: Vec<String>| Kind::Act {
        act,
        targets,
        typed,
        key: None,
    };
    match tool {
        "browser_navigate" => open(text("url")),
        "browser_tabs"
            if input.get("action").and_then(Value::as_str) == Some("new")
                && !text("url").trim().is_empty() =>
        {
            open(text("url"))
        }
        "browser_handle_dialog" => {
            if input.get("accept").and_then(Value::as_bool) == Some(true) {
                Kind::AcceptDialog
            } else {
                Kind::Read
            }
        }
        "browser_drop"
            if input
                .get("paths")
                .and_then(Value::as_array)
                .is_some_and(|paths| !paths.is_empty()) =>
        {
            Kind::Dangerous(Danger::Upload)
        }
        "browser_click" => act(Act::Click, vec![text("target")], Vec::new()),
        "browser_type" => act(Act::Type, vec![text("target")], vec![text("target")]),
        "browser_fill_form" => {
            let fields = input
                .get("fields")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let target = |field: &Value| {
                field
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            // Every field that can take text is typed into: Playwright fills a `slider` as it
            // fills a `textbox`. Only a checkbox or a radio takes true or false.
            let typed = fields
                .iter()
                .filter(|f| {
                    !matches!(
                        f.get("type").and_then(Value::as_str),
                        Some("checkbox" | "radio")
                    )
                })
                .map(target)
                .collect();
            act(Act::Fill, fields.iter().map(target).collect(), typed)
        }
        "browser_select_option" => act(Act::Select, vec![text("target")], Vec::new()),
        "browser_press_key" => Kind::Act {
            act: Act::Press,
            targets: Vec::new(),
            typed: Vec::new(),
            key: Some(text("key")),
        },
        "browser_hover" => act(Act::Hover, vec![text("target")], Vec::new()),
        "browser_drag" => act(
            Act::Drag,
            vec![text("startTarget"), text("endTarget")],
            Vec::new(),
        ),
        "browser_drop" => act(Act::Drop, vec![text("target")], Vec::new()),
        "browser_evaluate" | "browser_run_code_unsafe" => Kind::Dangerous(Danger::RunCode),
        "browser_file_upload" => Kind::Dangerous(Danger::Upload),
        "browser_network_request" | "browser_network_requests" => Kind::Dangerous(Danger::Network),
        tool if READ_TOOLS.contains(&tool) => Kind::Read,
        _ => Kind::Dangerous(Danger::Other),
    }
}

/// Going to `url`; a `javascript:` address runs code in the page instead.
fn open(url: String) -> Kind {
    if url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("javascript:")
    {
        Kind::Dangerous(Danger::RunCode)
    } else {
        Kind::Open { url }
    }
}

/// A dot's browser rules by what they say (section 6): which sites are allowed, asked,
/// denied, and where risky acts are allowed or denied. A rule whose site is not valid is
/// ignored (`Policy::validate` refuses it when the dot is saved).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteRules {
    allowed: Vec<String>,
    asked: Vec<String>,
    denied: Vec<String>,
    risky_allowed: Vec<String>,
    risky_denied: Vec<String>,
}

impl SiteRules {
    pub fn of(policy: &Policy) -> SiteRules {
        let mut rules = SiteRules::default();
        for rule in &policy.rules {
            let Some((risky, site)) = browser_rule(&rule.tool) else {
                continue;
            };
            let list = match (risky, rule.action) {
                (false, Action::Allow) => &mut rules.allowed,
                (false, Action::Ask) => &mut rules.asked,
                (false, Action::Deny) => &mut rules.denied,
                (true, Action::Allow) => &mut rules.risky_allowed,
                (true, Action::Deny) => &mut rules.risky_denied,
                (true, Action::Ask) => continue,
            };
            list.push(site);
        }
        rules
    }

    fn covers(list: &[String], site: &str) -> bool {
        list.iter().any(|rule| site_matches(rule, site))
    }

    /// A deny rule covers `site`: every call there is refused.
    pub fn denied(&self, site: &str) -> bool {
        Self::covers(&self.denied, site)
    }

    /// Going to `site` and acting there need no approval: an allow rule covers it (an allow
    /// `BrowserRisky` too), and no ask rule does.
    pub fn approved(&self, site: &str) -> bool {
        !Self::covers(&self.asked, site)
            && (Self::covers(&self.allowed, site) || Self::covers(&self.risky_allowed, site))
    }

    /// A risky act on `site` needs no approval.
    pub fn risky_allowed(&self, site: &str) -> bool {
        self.approved(site) && Self::covers(&self.risky_allowed, site) && !self.risky_denied(site)
    }

    /// A `BrowserRisky` deny rule covers `site`: its risky acts are refused.
    pub fn risky_denied(&self, site: &str) -> bool {
        Self::covers(&self.risky_denied, site)
    }
}

/// `(risky, site)` of a rule `Browser(<site>)` (`false`) or `BrowserRisky(<site>)` (`true`).
fn browser_rule(pattern: &str) -> Option<(bool, String)> {
    let (name, rest) = pattern.trim().split_once('(')?;
    let site = rest.strip_suffix(')')?;
    let risky = match name {
        "Browser" => false,
        "BrowserRisky" => true,
        _ => return None,
    };
    check_site(site).is_ok().then(|| (risky, rule_site(site)))
}

/// What an approval card shows about a browser call that asks (section 3.6), stored in the
/// approval's input under `ASK_KEY`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ask {
    /// `open`, an act (`click`, `type`, `fill`, `select`, `press`, `hover`, `drag`, `drop`),
    /// `dialog`, or a danger (`run_code`, `upload`, `network`, `other`).
    pub action: String,
    /// The site the call goes to or acts on; `None` for a page with no site.
    pub site: Option<String>,
    /// The page as the card shows it (`shop.com/cart`).
    pub page: Option<String>,
    /// The labels of the elements it acts on, in order; `None` for an element that is not a
    /// reference of the page's snapshot. For `press` the key, for `other` the tool.
    pub labels: Vec<Option<String>>,
    pub risky: bool,
    /// No rule can allow it.
    pub dangerous: bool,
}

/// The gate's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow,
    Ask(Ask),
    /// Not performed, for this reason.
    Refuse(String),
}

/// Decides a call of `tool` of `kind` on the page `page` (its address, as `RunBrowser`
/// confirmed it; `None` when it could not), whose snapshot references are `refs`, under the
/// dot's `rules`. Typing a password or a code is refused whatever the rules; then a deny rule
/// refuses; a dangerous call asks; reading is free; going to a site asks unless it is
/// approved; an act asks unless its page's site is approved and, for a risky act, risky acts
/// are allowed there. A target that is not a reference of `refs` is risky.
pub fn judge(
    tool: &str,
    kind: &Kind,
    page: Option<&str>,
    refs: Option<&RefMap>,
    rules: &SiteRules,
    dot_name: &str,
) -> Verdict {
    // It runs a program in Playwright's own process, outside the page and the workspace.
    if tool == "browser_run_code_unsafe" {
        return Verdict::Refuse(RUN_CODE_REFUSAL.to_string());
    }
    let current = page.and_then(site_of);
    let refused = |site: &str| {
        Verdict::Refuse(format!(
            "Denied by the policy of dot '{dot_name}': the browser may not be used on {site}."
        ))
    };
    let ask = |action: &str,
               site: Option<String>,
               labels: Vec<Option<String>>,
               risky: bool,
               dangerous: bool| {
        Verdict::Ask(Ask {
            action: action.to_string(),
            site,
            page: page.map(place),
            labels,
            risky,
            dangerous,
        })
    };
    let on_page = |action: &str, labels: Vec<Option<String>>, risky: bool| match current.as_deref()
    {
        None => ask(action, None, labels, risky, false),
        Some(site) if rules.denied(site) || (risky && rules.risky_denied(site)) => refused(site),
        Some(site) if rules.approved(site) && (!risky || rules.risky_allowed(site)) => {
            Verdict::Allow
        }
        Some(_) => ask(action, current.clone(), labels, risky, false),
    };
    match kind {
        Kind::Read => match current.as_deref() {
            Some(site) if rules.denied(site) => refused(site),
            _ => Verdict::Allow,
        },
        Kind::Open { url } => {
            let target = site_of(url);
            match target.as_deref() {
                Some(site) if rules.denied(site) => refused(site),
                Some(site) if rules.approved(site) => Verdict::Allow,
                _ => Verdict::Ask(Ask {
                    action: "open".to_string(),
                    site: target.clone(),
                    page: Some(place(url)),
                    labels: Vec::new(),
                    risky: false,
                    dangerous: false,
                }),
            }
        }
        Kind::Dangerous(danger) => match current.as_deref() {
            Some(site) if rules.denied(site) => refused(site),
            _ => {
                let labels = match danger {
                    Danger::Other => vec![Some(tool.to_string())],
                    _ => Vec::new(),
                };
                ask(danger.as_str(), current.clone(), labels, true, true)
            }
        },
        Kind::AcceptDialog => on_page("dialog", Vec::new(), true),
        Kind::Act {
            act,
            targets,
            typed,
            key,
        } => {
            let element = |target: &String| refs.and_then(|map| map.get(target.trim()));
            if typed.iter().filter_map(element).any(is_secret) {
                return Verdict::Refuse(PASSWORD_REFUSAL.to_string());
            }
            let labels = match act {
                Act::Press => vec![key.clone()],
                _ => targets
                    .iter()
                    .map(|t| element(t).map(|e| e.label().to_string()))
                    .collect(),
            };
            let risky = targets.iter().any(|t| element(t).is_none_or(is_risky));
            on_page(act.as_str(), labels, risky)
        }
    }
}

/// `text` as words: lower case, every run of characters that are not letters or digits one
/// space, with a space at each end.
fn words_of(text: &str) -> String {
    let mut out = String::from(" ");
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

/// Whether `text` holds one of `words` (a word or a phrase) as whole words, ignoring case.
pub fn has_word(text: &str, words: &[&str]) -> bool {
    let text = words_of(text);
    words.iter().any(|word| text.contains(&words_of(word)))
}

/// An act on `element` is risky (section 3.4).
pub fn is_risky(element: &Element) -> bool {
    has_word(&element.name, &RISKY_WORDS) || has_word(&element.role, &RISKY_WORDS)
}

/// `element` takes a password or a code (section 3.5).
pub fn is_secret(element: &Element) -> bool {
    has_word(&element.name, &SECRET_WORDS) || element.role.eq_ignore_ascii_case("password")
}

/// The rules a person may save while approving a browser call (section 3.6): "always allow on
/// <site>" and, for a risky act, "always allow risky actions on <site>". None for a dangerous
/// call, a page with no site, or an input without Dots' card. `None` for a tool that is not
/// the browser's.
pub fn rule_candidates(tool: &str, input: &Value) -> Option<Vec<RuleCandidate>> {
    if !super::is_browser_tool(tool) {
        return None;
    }
    let card = input
        .get(ASK_KEY)
        .cloned()
        .and_then(|v| serde_json::from_value::<Ask>(v).ok());
    let Some(Ask {
        site: Some(site),
        risky,
        dangerous: false,
        ..
    }) = card
    else {
        return Some(Vec::new());
    };
    if site == "*" || check_site(&site).is_err() {
        return Some(Vec::new());
    }
    let mut out = vec![RuleCandidate {
        pattern: format!("Browser({site})"),
        label: format!("always allow on {site}"),
    }];
    if risky {
        out.push(RuleCandidate {
            pattern: format!("BrowserRisky({site})"),
            label: format!("always allow risky actions on {site}"),
        });
    }
    Some(out)
}

/// The line the run view shows for a call Dots performed (section 7), from what the gate knew
/// when it decided: `Opened shop.com/cart`, `Clicked "Add to cart"`, `Typed into "Search"`,
/// `Took a screenshot`. An element that is not a reference shows as the model named it.
/// `None` for a call that needs no line of its own.
pub fn line(tool: &str, kind: &Kind, refs: Option<&RefMap>) -> Option<String> {
    let named = |target: &String| match refs.and_then(|map| map.get(target.trim())) {
        Some(element) => format!("\"{}\"", element.label()),
        None => target.trim().to_string(),
    };
    Some(match kind {
        Kind::Open { url } => format!("Opened {}", place(url)),
        Kind::AcceptDialog => "Accepted a dialog".to_string(),
        Kind::Dangerous(_) => format!("Used {tool}"),
        Kind::Read => match tool {
            "browser_snapshot" => "Read the page".to_string(),
            "browser_take_screenshot" => "Took a screenshot".to_string(),
            "browser_navigate_back" => "Went back".to_string(),
            _ => return None,
        },
        Kind::Act {
            act, targets, key, ..
        } => {
            let first = targets.first().map(named).unwrap_or_default();
            match act {
                Act::Click => format!("Clicked {first}"),
                Act::Type => format!("Typed into {first}"),
                Act::Fill if targets.len() == 1 => "Filled in 1 field".to_string(),
                Act::Fill => format!("Filled in {} fields", targets.len()),
                Act::Select => format!("Chose an option in {first}"),
                Act::Press => format!("Pressed {}", key.as_deref().unwrap_or_default()),
                Act::Hover => format!("Pointed at {first}"),
                Act::Drag => format!(
                    "Dragged {first} to {}",
                    targets.get(1).map(named).unwrap_or_default()
                ),
                Act::Drop => format!("Dropped onto {first}"),
            }
        }
    })
}

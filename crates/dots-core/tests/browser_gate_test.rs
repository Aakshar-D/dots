//! The browser gate (browser spec section 3), pure: each tool's kind, sites and rules, risky
//! words, targets that are not references of the current snapshot, refused passwords, deny
//! over allow, what a card stores and offers, and the run view's line, on the references of a
//! captured snapshot of a cart page served as `https://shop.com/cart`.

mod common;

use common::playwright::{on, SNAPSHOT};
use dots_core::browser::gate::{
    classify, has_word, is_risky, is_secret, judge, line, rule_candidates, Act, Ask, Danger, Kind,
    SiteRules, Verdict, ASK_KEY, PASSWORD_REFUSAL, RISKY_WORDS, RUN_CODE_REFUSAL, SECRET_WORDS,
};
use dots_core::browser::snapshot::{read_snapshot, Element, RefMap};
use dots_core::policy::{allow_candidates, Action, Policy, Preset, Rule, RuleCandidate};
use serde_json::{json, Value};

const CART: &str = "https://shop.com/cart";
const SHOP: (&str, Action) = ("Browser(shop.com)", Action::Allow);
const RISKY: (&str, Action) = ("BrowserRisky(shop.com)", Action::Allow);
const EVERYTHING: [(&str, Action); 2] = [
    ("Browser(*)", Action::Allow),
    ("BrowserRisky(*)", Action::Allow),
];

fn refs() -> RefMap {
    read_snapshot(&on(SNAPSHOT, CART)).unwrap()
}

fn rules(list: &[(&str, Action)]) -> SiteRules {
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules
        .extend(list.iter().map(|(tool, action)| Rule::new(tool, *action)));
    SiteRules::of(&p)
}

/// The verdict on `tool` with `input` on the cart page, under the rules `list`.
fn verdict(tool: &str, input: Value, list: &[(&str, Action)]) -> Verdict {
    judge(
        tool,
        &classify(tool, &input),
        Some(CART),
        Some(&refs()),
        &rules(list),
        "shopper",
    )
}

fn asks(v: &Verdict) -> &Ask {
    match v {
        Verdict::Ask(ask) => ask,
        other => panic!("expected an ask, got {other:?}"),
    }
}

fn denied(site: &str) -> Verdict {
    Verdict::Refuse(format!(
        "Denied by the policy of dot 'shopper': the browser may not be used on {site}."
    ))
}

fn s(text: &str) -> String {
    text.to_string()
}

#[test]
fn every_tool_of_the_pinned_server_has_its_kind() {
    for tool in [
        "browser_snapshot",
        "browser_take_screenshot",
        "browser_wait_for",
        "browser_navigate_back",
        "browser_console_messages",
        "browser_find",
        "browser_resize",
        "browser_emulate_media",
        "browser_close",
    ] {
        assert_eq!(classify(tool, &json!({})), Kind::Read, "{tool}");
    }
    let open = |url: &str| Kind::Open { url: s(url) };
    assert_eq!(
        classify("browser_tabs", &json!({"action": "list"})),
        Kind::Read
    );
    assert_eq!(
        classify("browser_tabs", &json!({"action": "new"})),
        Kind::Read
    );
    assert_eq!(
        classify(
            "browser_tabs",
            &json!({"action": "new", "url": "https://a.com/"})
        ),
        open("https://a.com/")
    );
    assert_eq!(
        classify(
            "browser_tabs",
            &json!({"action": "select", "index": 0, "url": "https://a.com/"})
        ),
        Kind::Read
    );
    assert_eq!(
        classify("browser_navigate", &json!({"url": "https://a.com/"})),
        open("https://a.com/")
    );
    assert_eq!(
        classify("browser_navigate", &json!({"url": " JavaScript:alert(1)"})),
        Kind::Dangerous(Danger::RunCode)
    );
    assert_eq!(
        classify("browser_handle_dialog", &json!({"accept": true})),
        Kind::AcceptDialog
    );
    assert_eq!(
        classify("browser_handle_dialog", &json!({"accept": false})),
        Kind::Read
    );
    let act = |act: Act, targets: &[&str], typed: &[&str], key: Option<&str>| Kind::Act {
        act,
        targets: targets.iter().map(|t| s(t)).collect(),
        typed: typed.iter().map(|t| s(t)).collect(),
        key: key.map(s),
    };
    assert_eq!(
        classify("browser_click", &json!({"target": "e16", "element": "Pay"})),
        act(Act::Click, &["e16"], &[], None)
    );
    assert_eq!(
        classify("browser_type", &json!({"target": "e11", "text": "shoes"})),
        act(Act::Type, &["e11"], &["e11"], None)
    );
    assert_eq!(
        classify(
            "browser_fill_form",
            &json!({"fields": [
                {"target": "e11", "name": "Search", "type": "textbox", "value": "x"},
                {"target": "e20", "name": "Remember", "type": "checkbox", "value": "true"},
                {"target": "e15", "name": "Code", "value": "1"}
            ]})
        ),
        act(Act::Fill, &["e11", "e20", "e15"], &["e11", "e15"], None)
    );
    assert_eq!(
        classify(
            "browser_select_option",
            &json!({"target": "e19", "values": ["Large"]})
        ),
        act(Act::Select, &["e19"], &[], None)
    );
    assert_eq!(
        classify("browser_press_key", &json!({"key": "Enter"})),
        act(Act::Press, &[], &[], Some("Enter"))
    );
    assert_eq!(
        classify("browser_hover", &json!({"target": "e6"})),
        act(Act::Hover, &["e6"], &[], None)
    );
    assert_eq!(
        classify(
            "browser_drag",
            &json!({"startTarget": "e6", "endTarget": "e21"})
        ),
        act(Act::Drag, &["e6", "e21"], &[], None)
    );
    assert_eq!(
        classify(
            "browser_drop",
            &json!({"target": "e1", "data": {"text/plain": "hi"}})
        ),
        act(Act::Drop, &["e1"], &[], None)
    );
    assert_eq!(
        classify("browser_drop", &json!({"target": "e1", "paths": []})),
        act(Act::Drop, &["e1"], &[], None)
    );
    // Dropping files sends files from the PC, as an upload does.
    assert_eq!(
        classify(
            "browser_drop",
            &json!({"target": "e1", "paths": ["C:/a.pdf"]})
        ),
        Kind::Dangerous(Danger::Upload)
    );
    for (tool, danger) in [
        ("browser_evaluate", Danger::RunCode),
        ("browser_run_code_unsafe", Danger::RunCode),
        ("browser_file_upload", Danger::Upload),
        ("browser_network_request", Danger::Network),
        ("browser_network_requests", Danger::Network),
        ("browser_install", Danger::Other),
        ("checkout_now", Danger::Other),
    ] {
        assert_eq!(
            classify(tool, &json!({})),
            Kind::Dangerous(danger),
            "{tool}"
        );
    }
}

#[test]
fn reading_is_free_unless_the_site_is_denied() {
    assert_eq!(verdict("browser_snapshot", json!({}), &[]), Verdict::Allow);
    assert_eq!(
        verdict("browser_take_screenshot", json!({"scale": "css"}), &[]),
        Verdict::Allow
    );
    assert_eq!(
        verdict(
            "browser_snapshot",
            json!({}),
            &[SHOP, ("Browser(shop.com)", Action::Deny)]
        ),
        denied("shop.com")
    );
    // A page with no site is not covered by `*`.
    assert_eq!(
        judge(
            "browser_snapshot",
            &Kind::Read,
            Some("about:blank"),
            None,
            &rules(&[("Browser(*)", Action::Deny)]),
            "shopper"
        ),
        Verdict::Allow
    );
}

#[test]
fn opening_a_page_asks_unless_its_site_is_approved() {
    let open =
        |url: &str, list: &[(&str, Action)]| verdict("browser_navigate", json!({"url": url}), list);
    assert_eq!(open("https://shop.com/cart", &[SHOP]), Verdict::Allow);
    assert_eq!(open("https://www.SHOP.com:443/x", &[SHOP]), Verdict::Allow);
    assert_eq!(open("https://app.shop.com/", &[SHOP]), Verdict::Allow);
    assert_eq!(open("https://shop.com/", &[RISKY]), Verdict::Allow);
    assert_eq!(open("https://any.example/", &EVERYTHING), Verdict::Allow);
    assert_eq!(
        asks(&open("https://new.example/a?b=1", &[SHOP])),
        &Ask {
            action: s("open"),
            site: Some(s("new.example")),
            page: Some(s("new.example/a")),
            labels: vec![],
            risky: false,
            dangerous: false,
        }
    );
    assert_eq!(
        asks(&open("https://evilshop.com/", &[SHOP]))
            .site
            .as_deref(),
        Some("evilshop.com")
    );
    for odd in [
        "https://shop.com@evil.com/",
        "https://evil.com\\@shop.com/",
        "https://shop%2ecom/",
        "shop.com/cart",
        "data:text/html,hi",
        "file:///C:/a.html",
        "about:blank",
        "edge://settings/",
    ] {
        assert_eq!(asks(&open(odd, &EVERYTHING)).site, None, "{odd}");
    }
    // A `javascript:` address runs code in the page: no rule allows it.
    let script = open("javascript:document.forms[0].submit()", &EVERYTHING);
    assert!(asks(&script).dangerous);
    // Deny wins over allow, and ask over allow.
    assert_eq!(
        open(
            "https://shop.com/",
            &[SHOP, ("Browser(shop.com)", Action::Deny)]
        ),
        denied("shop.com")
    );
    let bank = [
        ("Browser(*)", Action::Allow),
        ("Browser(bank.com)", Action::Ask),
    ];
    assert!(matches!(open("https://bank.com/", &bank), Verdict::Ask(_)));
    assert_eq!(open("https://shop.com/", &bank), Verdict::Allow);
}

#[test]
fn acting_needs_an_approved_site_and_a_risky_act_asks_there() {
    assert_eq!(
        verdict("browser_click", json!({"target": "e6"}), &[SHOP]),
        Verdict::Allow
    );
    assert_eq!(
        verdict(
            "browser_type",
            json!({"target": "e11", "text": "shoes"}),
            &[SHOP]
        ),
        Verdict::Allow
    );
    // `Enter` is not risky on its own (spec section 11).
    assert_eq!(
        verdict("browser_press_key", json!({"key": "Enter"}), &[SHOP]),
        Verdict::Allow
    );
    // The model's own description of the element plays no part.
    assert_eq!(
        asks(&verdict(
            "browser_click",
            json!({"target": "e6", "element": "Pay now!"}),
            &[]
        )),
        &Ask {
            action: s("click"),
            site: Some(s("shop.com")),
            page: Some(s("shop.com/cart")),
            labels: vec![Some(s("Home"))],
            risky: false,
            dangerous: false,
        }
    );
    assert_eq!(
        asks(&verdict(
            "browser_click",
            json!({"target": "e16", "element": "Home link"}),
            &[SHOP]
        )),
        &Ask {
            action: s("click"),
            site: Some(s("shop.com")),
            page: Some(s("shop.com/cart")),
            labels: vec![Some(s("Pay now"))],
            risky: true,
            dangerous: false,
        }
    );
    assert_eq!(
        verdict("browser_click", json!({"target": "e16"}), &[SHOP, RISKY]),
        Verdict::Allow
    );
    // An allow `BrowserRisky` approves the site as well.
    assert_eq!(
        verdict("browser_click", json!({"target": "e16"}), &[RISKY]),
        Verdict::Allow
    );
    // "My orders" is a plain act: `orders` is not the whole word `order`.
    assert_eq!(
        verdict("browser_click", json!({"target": "e8"}), &[SHOP]),
        Verdict::Allow
    );
    // "Delete account" is risky.
    assert!(asks(&verdict("browser_click", json!({"target": "e18"}), &[SHOP])).risky);
    // A `BrowserRisky` deny refuses the risky acts; an ask rule keeps the site asking.
    assert!(matches!(
        verdict(
            "browser_click",
            json!({"target": "e16"}),
            &[SHOP, RISKY, ("BrowserRisky(shop.com)", Action::Deny)]
        ),
        Verdict::Refuse(_)
    ));
    assert!(matches!(
        verdict(
            "browser_click",
            json!({"target": "e6"}),
            &[
                ("Browser(*)", Action::Allow),
                ("Browser(shop.com)", Action::Ask)
            ]
        ),
        Verdict::Ask(_)
    ));
    // Accepting a dialog is always risky; dismissing it is a read.
    let dialog = verdict("browser_handle_dialog", json!({"accept": true}), &[SHOP]);
    assert_eq!(
        (asks(&dialog).action.as_str(), asks(&dialog).risky),
        ("dialog", true)
    );
    assert_eq!(
        verdict(
            "browser_handle_dialog",
            json!({"accept": true}),
            &[SHOP, RISKY]
        ),
        Verdict::Allow
    );
    assert_eq!(
        verdict("browser_handle_dialog", json!({"accept": false}), &[]),
        Verdict::Allow
    );
    // A drag is risky when either end is.
    assert!(
        asks(&verdict(
            "browser_drag",
            json!({"startTarget": "e6", "endTarget": "e18"}),
            &[SHOP]
        ))
        .risky
    );
    // An act on a page with no site, or on a page Dots could not confirm, asks.
    let click_home = classify("browser_click", &json!({"target": "e6"}));
    for page in [Some("about:blank"), None] {
        let v = judge(
            "browser_click",
            &click_home,
            page,
            Some(&refs()),
            &rules(&EVERYTHING),
            "shopper",
        );
        assert_eq!(asks(&v).site, None, "{page:?}");
    }
}

#[test]
fn risky_words_are_whole_words_in_any_case_and_spacing() {
    for risky in [
        "PAY NOW",
        "Pay-now",
        "Buy\u{a0}now",
        "Checkout →",
        "Confirm & pay",
        "Place order",
        "Sign in",
        "Post",
        "submit",
        "Delete",
        "Book a table",
        "Remove item (2)",
        "unsubscribe",
        "Step 2: transfer",
    ] {
        assert!(has_word(risky, &RISKY_WORDS), "{risky}");
    }
    for plain in [
        "Paypal",
        "Placeholder",
        "Ordering",
        "Postcode",
        "Display",
        "Sender",
        "Bookmarks",
        "Cancellation policy",
        "",
    ] {
        assert!(!has_word(plain, &RISKY_WORDS), "{plain}");
    }
    // The role counts as well as the name.
    assert!(is_risky(&Element::new("link", "Checkout")));
    assert!(is_risky(&Element::new("menuitem-delete", "")));
    assert!(!is_risky(&Element::new("button", "Continue")));
}

#[test]
fn a_target_not_in_the_current_snapshot_is_risky() {
    for target in [
        "e999",
        "E16",
        "text=Pay",
        "role=button[name=\"Pay\"]",
        "#pay",
        "",
        "e6x",
    ] {
        let v = verdict("browser_click", json!({"target": target}), &[SHOP]);
        assert!(asks(&v).risky, "{target:?}");
        assert_eq!(asks(&v).labels, vec![None], "{target:?}");
    }
    // A known reference with spaces around it is that reference.
    assert_eq!(
        verdict("browser_click", json!({"target": " e6 "}), &[SHOP]),
        Verdict::Allow
    );
    let click_home = classify("browser_click", &json!({"target": "e6"}));
    // Before any snapshot of the page, and with another tab's references, nothing is known.
    for map in [None, Some(&RefMap::new())] {
        let v = judge(
            "browser_click",
            &click_home,
            Some(CART),
            map,
            &rules(&[SHOP]),
            "shopper",
        );
        assert!(asks(&v).risky);
    }
    // With risky actions allowed there, an unknown target passes too.
    assert_eq!(
        verdict(
            "browser_click",
            json!({"target": "text=Pay"}),
            &[SHOP, RISKY]
        ),
        Verdict::Allow
    );
}

#[test]
fn typing_a_password_or_code_is_refused_whatever_the_rules() {
    let refusal = Verdict::Refuse(s(PASSWORD_REFUSAL));
    assert_eq!(
        PASSWORD_REFUSAL,
        "Dots does not type passwords or codes. Ask the person to enter it in the browser window, with your ask tool."
    );
    assert_eq!(
        verdict(
            "browser_type",
            json!({"target": "e13", "text": "hunter2"}),
            &EVERYTHING
        ),
        refusal
    );
    assert_eq!(
        verdict(
            "browser_type",
            json!({"target": "e15", "text": "123456"}),
            &EVERYTHING
        ),
        refusal
    );
    assert_eq!(
        verdict(
            "browser_fill_form",
            json!({"fields": [
                {"target": "e11", "name": "Search", "type": "textbox", "value": "x"},
                {"target": "e13", "name": "Your name", "type": "textbox", "value": "y"}
            ]}),
            &EVERYTHING
        ),
        refusal
    );
    let field = |role: &str, name: &str| {
        let mut map = RefMap::new();
        map.insert(s("e1"), Element::new(role, name));
        let typing = classify("browser_type", &json!({"target": "e1", "text": "x"}));
        judge(
            "browser_type",
            &typing,
            Some(CART),
            Some(&map),
            &rules(&EVERYTHING),
            "shopper",
        )
    };
    for secret in [
        "Password",
        "Confirm password",
        "PIN",
        "Enter your PIN",
        "Passcode",
        "OTP",
        "One-time code",
        "one time password",
        "Verification code",
        "Security Code",
        "CVV / security code",
    ] {
        assert_eq!(field("textbox", secret), refusal, "{secret}");
    }
    assert_eq!(field("password", ""), refusal);
    for plain in [
        "Pinterest handle",
        "Spin speed",
        "Shipping",
        "Passport number",
        "Verification",
        "Code",
    ] {
        assert_eq!(field("textbox", plain), Verdict::Allow, "{plain}");
    }
    assert!(has_word("Your one-time passcode", &SECRET_WORDS));
    assert!(is_secret(&Element::new("textbox", "PIN code")));
    // A secret field ticked, not typed into, is an ordinary act.
    assert_eq!(
        verdict(
            "browser_fill_form",
            json!({"fields": [{"target": "e13", "name": "x", "type": "checkbox", "value": "true"}]}),
            &EVERYTHING
        ),
        Verdict::Allow
    );
}

#[test]
fn a_dangerous_tool_always_asks_and_offers_no_rule() {
    for (tool, action) in [
        ("browser_evaluate", "run_code"),
        ("browser_file_upload", "upload"),
        ("browser_network_requests", "network"),
        ("browser_network_request", "network"),
    ] {
        let ask = asks(&verdict(tool, json!({}), &EVERYTHING)).clone();
        assert_eq!(ask.action, action, "{tool}");
        assert!(ask.dangerous && ask.risky, "{tool}");
        assert_eq!(ask.site.as_deref(), Some("shop.com"), "{tool}");
        let input = json!({ ASK_KEY: ask });
        assert_eq!(
            rule_candidates(&format!("mcp__browser__{tool}"), &input),
            Some(vec![]),
            "{tool}"
        );
    }
    let other = asks(&verdict("checkout_now", json!({}), &EVERYTHING)).clone();
    assert_eq!(other.action, "other");
    assert_eq!(other.labels, vec![Some(s("checkout_now"))]);
    assert_eq!(
        verdict(
            "browser_evaluate",
            json!({}),
            &[("Browser(shop.com)", Action::Deny)]
        ),
        denied("shop.com")
    );
}

#[test]
fn a_card_offers_the_rules_of_its_site() {
    let pay = asks(&verdict("browser_click", json!({"target": "e16"}), &[SHOP])).clone();
    let both = vec![
        RuleCandidate {
            pattern: s("Browser(shop.com)"),
            label: s("always allow on shop.com"),
        },
        RuleCandidate {
            pattern: s("BrowserRisky(shop.com)"),
            label: s("always allow risky actions on shop.com"),
        },
    ];
    let input = json!({"target": "e16", ASK_KEY: pay});
    assert_eq!(
        rule_candidates("mcp__browser__browser_click", &input),
        Some(both.clone())
    );
    assert_eq!(
        allow_candidates("mcp__browser__browser_click", &input, None),
        both
    );
    let home = asks(&verdict("browser_click", json!({"target": "e6"}), &[])).clone();
    assert_eq!(
        allow_candidates(
            "mcp__browser__browser_click",
            &json!({ ASK_KEY: home.clone() }),
            None
        ),
        both[..1].to_vec()
    );
    // No site, `*`, a site Dots would not save, or no card: nothing to save, and never a rule
    // for the whole server.
    for site in [None, Some(s("*")), Some(s("Shop.com"))] {
        let card = Ask {
            site,
            ..home.clone()
        };
        assert!(allow_candidates(
            "mcp__browser__browser_click",
            &json!({ ASK_KEY: card }),
            None
        )
        .is_empty());
    }
    assert!(allow_candidates(
        "mcp__browser__browser_click",
        &json!({"target": "e6"}),
        None
    )
    .is_empty());
    // Other tools keep their own rules.
    assert_eq!(rule_candidates("mcp__files__read", &json!({})), None);
    assert_eq!(
        allow_candidates("mcp__files__read", &json!({}), None).len(),
        2
    );
}

#[test]
fn the_run_view_line_says_what_was_done() {
    let map = refs();
    let line_of = |tool: &str, input: Value| line(tool, &classify(tool, &input), Some(&map));
    let cases: [(&str, Value, Option<&str>); 12] = [
        (
            "browser_navigate",
            json!({"url": "https://www.shop.com/cart?x=1"}),
            Some("Opened shop.com/cart"),
        ),
        (
            "browser_click",
            json!({"target": "e16"}),
            Some("Clicked \"Pay now\""),
        ),
        (
            "browser_type",
            json!({"target": "e11", "text": "shoes"}),
            Some("Typed into \"Search\""),
        ),
        (
            "browser_click",
            json!({"target": "text=Pay"}),
            Some("Clicked text=Pay"),
        ),
        (
            "browser_take_screenshot",
            json!({"scale": "css"}),
            Some("Took a screenshot"),
        ),
        ("browser_snapshot", json!({}), Some("Read the page")),
        (
            "browser_press_key",
            json!({"key": "Enter"}),
            Some("Pressed Enter"),
        ),
        (
            "browser_fill_form",
            json!({"fields": [{"target": "e11"}, {"target": "e20"}]}),
            Some("Filled in 2 fields"),
        ),
        (
            "browser_drag",
            json!({"startTarget": "e6", "endTarget": "e21"}),
            Some("Dragged \"Home\" to \"Download statement\""),
        ),
        (
            "browser_handle_dialog",
            json!({"accept": true}),
            Some("Accepted a dialog"),
        ),
        (
            "browser_evaluate",
            json!({"function": "() => 1"}),
            Some("Used browser_evaluate"),
        ),
        ("browser_tabs", json!({"action": "list"}), None),
    ];
    for (tool, input, want) in cases {
        assert_eq!(line_of(tool, input).as_deref(), want, "{tool}");
    }
}

/// Review fix: Playwright fills `textbox` and `slider` fields alike, so every form field that
/// can take text is checked for a secret; only a checkbox or a radio, which takes true or
/// false, is not.
#[test]
fn a_password_is_refused_in_any_form_field_that_takes_text() {
    for kind in ["slider", "combobox", "textbox", "", "something"] {
        let input =
            json!({"fields": [{"target": "e13", "name": "x", "type": kind, "value": "hunter2"}]});
        assert_eq!(
            verdict("browser_fill_form", input, &EVERYTHING),
            Verdict::Refuse(PASSWORD_REFUSAL.to_string()),
            "{kind}"
        );
    }
    for kind in ["checkbox", "radio"] {
        let input =
            json!({"fields": [{"target": "e13", "name": "x", "type": kind, "value": "true"}]});
        assert_eq!(
            verdict("browser_fill_form", input, &EVERYTHING),
            Verdict::Allow,
            "{kind}"
        );
    }
}

/// Review fix: `browser_run_code_unsafe` runs a program in Playwright's own process on this
/// computer, outside the page and the workspace, so no approval can allow it.
#[test]
fn running_code_outside_the_page_is_refused() {
    assert_eq!(
        verdict("browser_run_code_unsafe", json!({"code": "1"}), &EVERYTHING),
        Verdict::Refuse(RUN_CODE_REFUSAL.to_string())
    );
    // Code in the page still asks.
    assert_eq!(
        asks(&verdict("browser_evaluate", json!({}), &EVERYTHING)).action,
        "run_code"
    );
}

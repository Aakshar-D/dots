//! The browser's rules in a dot's policy (browser spec section 6): `Browser(<site>)` and
//! `BrowserRisky(<site>)` validate as `Context(<name>)` rules do, never reach the CLI's
//! `--allowedTools` / `--disallowedTools`, decide no other tool, and no preset carries them.

use dots_core::browser::sites::{BARE_RULE, SITE_RULE};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::Error;
use serde_json::json;

fn with(rules: &[(&str, Action)]) -> Policy {
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules
        .extend(rules.iter().map(|(tool, action)| Rule::new(tool, *action)));
    p
}

#[test]
fn a_browser_rule_names_one_site_or_every_site() {
    for ok in [
        "Browser(shop.com)",
        "Browser(*)",
        "BrowserRisky(app.shop.com)",
        "BrowserRisky(*)",
        "Browser(127.0.0.1)",
    ] {
        for action in [Action::Allow, Action::Ask, Action::Deny] {
            assert!(with(&[(ok, action)]).validate().is_ok(), "{ok} {action:?}");
        }
    }
    for (bad, why) in [
        ("Browser", BARE_RULE),
        ("BrowserRisky", BARE_RULE),
        ("Browser(Shop.com)", SITE_RULE),
        ("Browser(shop.com/cart)", SITE_RULE),
        ("BrowserRisky(https://shop.com)", SITE_RULE),
        ("Browser(*.shop.com)", SITE_RULE),
        ("Browser( )", "empty spec"),
    ] {
        let err = with(&[(bad, Action::Allow)]).validate().unwrap_err();
        let want = format!("policy rule {bad:?}: {why}");
        assert!(
            matches!(&err, Error::Invalid(m) if *m == want),
            "{bad}: {err}"
        );
    }
}

#[test]
fn browser_rules_never_reach_the_cli() {
    let p = with(&[
        ("Browser(shop.com)", Action::Allow),
        ("BrowserRisky(shop.com)", Action::Allow),
        ("Browser(bank.com)", Action::Deny),
        ("BrowserRisky(*)", Action::Deny),
        ("Bash(git log:*)", Action::Allow),
    ]);
    let (allow, deny) = p.cli_lists();
    assert!(allow.contains(&"Bash(git log:*)".to_string()), "{allow:?}");
    assert!(
        allow.iter().chain(&deny).all(|t| !t.starts_with("Browser")),
        "{allow:?} {deny:?}"
    );
}

#[test]
fn no_preset_carries_a_browser_rule() {
    for preset in Preset::ALL {
        assert!(
            Policy::preset(*preset)
                .rules
                .iter()
                .all(|r| !r.tool.starts_with("Browser")),
            "{preset:?}"
        );
    }
}

#[test]
fn a_browser_rule_decides_no_other_tool() {
    // The browser gate reads these rules itself (Task 4); `resolve` decides the CLI's and the
    // local engine's own tools, which they never name.
    let p = with(&[
        ("Browser(*)", Action::Deny),
        ("BrowserRisky(*)", Action::Deny),
    ]);
    assert_eq!(
        p.resolve("Read", &json!({"file_path": "a.txt"})),
        Action::Allow
    );
    assert_eq!(p.resolve("Skill", &json!({"skill": "pdf"})), Action::Allow);
}

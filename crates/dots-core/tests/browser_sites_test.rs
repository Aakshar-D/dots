//! Sites (browser spec section 3.2): the host of an address without `www.` and port, the
//! addresses that have none, which rules cover a site, and how a page is shown.

use dots_core::browser::sites::{check_site, place, rule_site, site_matches, site_of, SITE_RULE};

#[test]
fn a_site_is_the_host_without_www_or_port_and_odd_addresses_have_none() {
    let cases: &[(&str, Option<&str>)] = &[
        ("https://shop.com/cart", Some("shop.com")),
        ("https://www.shop.com/", Some("shop.com")),
        ("HTTPS://WWW.Shop.COM.:443/cart?x=1#top", Some("shop.com")),
        ("http://app.shop.com:8080", Some("app.shop.com")),
        ("http://127.0.0.1:54428/cart", Some("127.0.0.1")),
        ("http://[::1]:8080/", Some("[::1]")),
        // `www.` goes only in front of a name that has a dot after it.
        ("https://www.com/", Some("www.com")),
        // A user name, an escape or a backslash in the host part: browsers and Dots could read
        // another host there.
        ("https://shop.com@evil.com/", None),
        ("https://evil.com\\@shop.com/", None),
        ("https://shop%2ecom/", None),
        // Not plain ASCII: Edge reports such a host in its punycode form.
        ("https://bücher.de/", None),
        ("about:blank", None),
        ("data:text/html,<button>Second tab</button>", None),
        ("file:///C:/Users/me/a.html", None),
        ("javascript:alert(1)", None),
        ("edge://downloads-hub/", None),
        ("chrome-extension://abc/page.html", None),
        ("ftp://files.shop.com/", None),
        ("shop.com/cart", None),
        ("", None),
        ("https:///path", None),
    ];
    for (url, site) in cases {
        assert_eq!(site_of(url).as_deref(), *site, "{url}");
    }
}

#[test]
fn a_rule_covers_its_site_and_its_subdomains_only() {
    assert!(site_matches("shop.com", "shop.com"));
    assert!(site_matches("shop.com", "app.shop.com"));
    assert!(site_matches("shop.com", "a.b.shop.com"));
    assert!(!site_matches("shop.com", "evilshop.com"));
    assert!(!site_matches("shop.com", "shop.com.evil.net"));
    assert!(!site_matches("app.shop.com", "shop.com"));
    assert!(site_matches("*", "anything.example"));
    assert_eq!(rule_site(" www.shop.com "), "shop.com");
    assert_eq!(rule_site("www.com"), "www.com");
    assert_eq!(rule_site("*"), "*");
}

#[test]
fn a_rule_site_is_a_lower_case_host_or_a_star() {
    for good in [
        "shop.com",
        "*",
        "app.shop-1.co.uk",
        "127.0.0.1",
        "localhost",
        " shop.com ",
    ] {
        assert_eq!(check_site(good), Ok(()), "{good:?}");
    }
    for bad in [
        "",
        " ",
        "Shop.com",
        "shop.com/cart",
        "https://shop.com",
        ".shop.com",
        "shop.com.",
        "shop..com",
        "*.shop.com",
        "shop com",
        "[::1]",
    ] {
        assert_eq!(check_site(bad), Err(SITE_RULE.to_string()), "{bad:?}");
    }
}

#[test]
fn a_page_shows_as_its_site_and_path() {
    assert_eq!(
        place("https://www.shop.com:8443/cart/?step=2#pay"),
        "shop.com/cart"
    );
    assert_eq!(place("https://example.com/"), "example.com");
    assert_eq!(place("http://127.0.0.1:54428/cart"), "127.0.0.1/cart");
    assert_eq!(place("about:blank"), "about:blank");
    let long = format!("https://shop.com/{}", "a".repeat(200));
    assert_eq!(place(&long).chars().count(), 80);
    assert!(place(&long).ends_with('…'));
}

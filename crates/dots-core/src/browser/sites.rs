//! Sites (browser spec section 3.2): the host of an address, lower-cased, without a port and
//! without a leading `www.`. A rule `Browser(shop.com)` covers `shop.com` and its subdomains,
//! and `Browser(*)` covers every site. An address with no host, or one Dots cannot read
//! safely, is no site: an act there asks.

/// Why a rule's site is refused (`Policy::validate` adds the rule in front).
pub const SITE_RULE: &str =
    "a Browser rule names one site: a host in lower case, such as shop.com, or *";
/// Why a bare `Browser` or `BrowserRisky` rule is refused.
pub const BARE_RULE: &str = "a Browser rule names one site: Browser(<site>) or Browser(*)";

/// The longest page shown on an approval card or a run-view line, in characters.
const PLACE_CHARS: usize = 80;

/// The site of `url`: for `http` and `https` only, its host without port and `www.`. `None`
/// for any other address (`about:blank`, `data:`, `file:`, `edge:`, one without `://`), for a
/// host that is not plain ASCII, and for a host part with `@`, `%` or `\`, which a browser and
/// Dots could read as different hosts.
pub fn site_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return None;
    }
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.is_empty() || authority.contains(['@', '%', '\\']) {
        return None;
    }
    let host = match authority.strip_prefix('[') {
        // An IPv6 address keeps its brackets: `[::1]`.
        Some(v6) => format!("[{}]", &v6[..v6.find(']')?]).to_ascii_lowercase(),
        None => authority
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase(),
    };
    let host = host.trim_end_matches('.');
    let host = without_www(host);
    let plain = host.starts_with('[')
        || (!host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'));
    plain.then(|| host.to_string())
}

/// `host` without a leading `www.`, when a name with a dot follows it.
fn without_www(host: &str) -> &str {
    host.strip_prefix("www.")
        .filter(|rest| rest.contains('.'))
        .unwrap_or(host)
}

/// Checks the site of a rule `Browser(<site>)` or `BrowserRisky(<site>)`: `*`, or a host in
/// lower case of letters, digits, `.` and `-`, without a leading, trailing or doubled `.`.
pub fn check_site(site: &str) -> Result<(), String> {
    let s = site.trim();
    let host = !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if s == "*" || host {
        Ok(())
    } else {
        Err(SITE_RULE.to_string())
    }
}

/// The site a rule names, as `site_of` gives sites: trimmed, without a leading `www.`.
pub fn rule_site(spec: &str) -> String {
    without_www(spec.trim()).to_string()
}

/// Whether the rule's site `rule` (from `rule_site`) covers `site`: `*`, the same site, or a
/// subdomain of it.
pub fn site_matches(rule: &str, site: &str) -> bool {
    rule == "*"
        || site == rule
        || site
            .strip_suffix(rule)
            .is_some_and(|sub| sub.ends_with('.'))
}

/// A page as a card and the run view show it: its site and path, without the scheme, `www.`,
/// port, query or fragment (`shop.com/cart`); any other address as it is. At most 80
/// characters.
pub fn place(url: &str) -> String {
    let url = url.trim();
    let shown = match (site_of(url), url.split_once("://")) {
        (Some(site), Some((_, rest))) => {
            let path = &rest[rest.find(['/', '?', '#']).unwrap_or(rest.len())..];
            let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
            format!("{site}{}", path.trim_end_matches('/'))
        }
        _ => url.to_string(),
    };
    if shown.chars().count() <= PLACE_CHARS {
        shown
    } else {
        let cut: String = shown.chars().take(PLACE_CHARS - 1).collect();
        format!("{cut}…")
    }
}

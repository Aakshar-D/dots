//! HTTP origin pinning per scope (env vault spec §8): an http MCP server whose URL reads a
//! `${VAR}` starts in a run only when the origin its URL reaches with the run's values was
//! confirmed for the dot's scope. A Personal value or an org's that moves the host leaves the
//! server out with a notice naming the server and the origin, never a value. A server whose URL
//! reads no variable has one origin and is never pinned, whatever its headers send.

mod common;

use std::collections::BTreeMap;

use common::fake_mcp::{tool, FakeHttp, FakeMcp};
use common::orgs::{policy, sign, OrgsHarness, ACME, TOKEN};
use dots_core::events::new_bus;
use dots_core::library::{
    HttpOrigin, LibraryConfig, LibraryItem, McpServerConfig, SkillConfig, Uses, PERSONAL_SCOPE,
};
use dots_core::library_service::LibraryService;
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::vault::{DotScope, EntryKind, Grants, Vault};
use dots_core::Error;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const ORG_KEY: &str = "org-secret-api-key-5d93a1";
const PERSONAL_KEY: &str = "personal-secret-api-key-e2c47b";
const LITERAL: &str = "ghp_literalTokenForTheOriginTest0042";

/// Every secret a test serves or stores, which no notice or error may hold.
const SECRETS: &[&str] = &[ORG_KEY, PERSONAL_KEY, LITERAL, TOKEN];

fn assert_clean(text: &str) {
    for value in SECRETS {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

fn acme() -> DotScope {
    DotScope::Org(ACME.into())
}

/// The notice of a run whose server `name` reaches the unconfirmed `origin`.
fn not_confirmed(name: &str, origin: &str) -> String {
    format!(
        "{name}: {origin} is not a confirmed origin; open the server in the Library to confirm it"
    )
}

/// An http `fake-mcp`, the `host:port` it listens on, and its origin.
struct Served {
    fake: FakeMcp,
    http: FakeHttp,
    port: u16,
}

impl Served {
    async fn start() -> Served {
        let fake = FakeMcp::new(json!({"tools": [tool("query", true)]}));
        let http = fake.serve_http().await;
        let port = reqwest::Url::parse(&http.url).unwrap().port().unwrap();
        Served { fake, http, port }
    }

    /// `host:<port>`, a value of `MCP_HOST`.
    fn at(&self, host: &str) -> String {
        format!("{host}:{}", self.port)
    }

    /// `http://host:<port>`, the origin of a URL on `host`.
    fn origin(&self, host: &str) -> String {
        format!("http://{}", self.at(host))
    }

    /// The `Authorization` each initialize request had, in order.
    fn auths(&self) -> Vec<String> {
        self.fake
            .log()
            .into_iter()
            .filter(|r| r["msg"]["method"] == "initialize")
            .map(|r| r["auth"].as_str().unwrap_or("").to_string())
            .collect()
    }
}

/// The pinned server: its host from `${MCP_HOST}`, its token from `${API_KEY}`.
fn pinned_config() -> LibraryConfig {
    LibraryConfig::McpServer(McpServerConfig::Http {
        url: "http://${MCP_HOST}/mcp".into(),
        headers: BTreeMap::from([("Authorization".into(), "Bearer ${API_KEY}".into())]),
    })
}

/// Acme joined with `MCP_HOST` (plain) and `API_KEY` from `entries`, its policy `personal:
/// deny`; Personal holding `MCP_HOST` on 127.0.0.1 and its own `API_KEY`; the server `gh`
/// added as the store adds it, with nothing confirmed.
async fn setup(
    served: &Served,
    entries: Value,
) -> (OrgsHarness, Vault, LibraryService, LibraryItem) {
    let h = OrgsHarness::new().await;
    h.join_acme(entries, json!({"personal": "deny", "plain": ["MCP_HOST"]}))
        .await;
    let vault = h.vault();
    vault
        .add("MCP_HOST", EntryKind::Plain, &served.at("127.0.0.1"))
        .await
        .unwrap();
    vault
        .add("API_KEY", EntryKind::Secret, PERSONAL_KEY)
        .await
        .unwrap();
    let lib = LibraryService::new(h.store.clone(), new_bus()).with_vault(vault.clone());
    let item = h
        .store
        .add_library_item("gh", &pinned_config(), None)
        .await
        .unwrap();
    (h, vault, lib, item)
}

fn org_entries(served: &Served, host: &str) -> Value {
    json!({"MCP_HOST": served.at(host), "API_KEY": ORG_KEY})
}

/// Opens a run of a dot in `scope` that uses the server `name` and grants it both variables.
/// Returns how many servers it opened and its notices, but for the one that holds a server
/// whose tool list is not accepted yet (no test here accepts one); each is checked for values.
async fn run(h: &OrgsHarness, vault: &Vault, scope: &DotScope, name: &str) -> (usize, Vec<String>) {
    let uses = Uses {
        mcp_servers: vec![name.to_string()],
        ..Default::default()
    };
    let grants = Grants(
        [(
            format!("mcp:{name}"),
            vec!["MCP_HOST".to_string(), "API_KEY".to_string()],
        )]
        .into(),
    );
    let (tools, notices) = RunTools::open(OpenRequest {
        store: &h.store,
        bus: &new_bus(),
        run_id: "01RUN",
        uses: &uses,
        workspace: h.data.path(),
        plugins_dir: None,
        cancel: &CancellationToken::new(),
        vault,
        scope,
        grants: &grants,
        dot_name: "triage",
    })
    .await
    .unwrap();
    for notice in &notices {
        assert_clean(notice);
    }
    let opened = tools.servers().len();
    tools.close().await;
    let notices = notices
        .into_iter()
        .filter(|n| !n.contains("its tool list has not been accepted yet"))
        .collect();
    (opened, notices)
}

fn origins(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
        .collect()
}

// Which servers are pinned

#[test]
fn only_an_http_server_whose_url_reads_a_variable_is_pinned() {
    let http = |url: &str, header: &str| McpServerConfig::Http {
        url: url.into(),
        headers: BTreeMap::from([("X-Key".into(), header.into())]),
    };
    assert!(http("https://${HOST}/mcp", "plain").origin_pinned());
    assert!(http("https://api.example/${PATH}", "Bearer ${TOKEN}").origin_pinned());
    assert!(http("https://api.example/mcp?k=${KEY}", "plain").origin_pinned());
    // A fixed URL has one origin: what its headers send needs no pin.
    assert!(!http("https://api.example/mcp", "Bearer ${TOKEN}").origin_pinned());
    assert!(!http("https://api.example/mcp", "plain").origin_pinned());
    assert!(!http("https://api.example/${", "a ${not a name}").origin_pinned());
    let stdio = McpServerConfig::Stdio {
        command: "node".into(),
        args: vec!["${ARG}".into()],
        cwd: None,
        env: BTreeMap::from([("TOKEN".into(), "${TOKEN}".into())]),
    };
    assert!(!stdio.origin_pinned());
}

/// A server that sends no variables starts with nothing confirmed, and has no origin to
/// confirm or compute.
#[tokio::test]
async fn a_server_that_sends_no_variables_is_never_pinned() {
    let served = Served::start().await;
    let (h, vault, lib, _) = setup(&served, org_entries(&served, "localhost")).await;
    let plain = LibraryConfig::McpServer(served.http.config(BTreeMap::new()));
    let item = h
        .store
        .add_library_item("plain", &plain, None)
        .await
        .unwrap();
    for scope in [DotScope::Personal, acme()] {
        let (opened, notices) = run(&h, &vault, &scope, "plain").await;
        assert_eq!((opened, notices), (1, Vec::<String>::new()));
    }
    assert_eq!(served.auths(), ["", ""]);
    let origin = served.origin("127.0.0.1");
    match h
        .store
        .add_confirmed_origin(&item.id, PERSONAL_SCOPE, &origin)
        .await
    {
        Err(Error::Invalid(why)) => assert_eq!(
            why,
            "the MCP server \"plain\" has no variable in its http URL: its origin needs no \
             confirmation"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert!(matches!(
        lib.origin_in(&item.id, &DotScope::Personal).await,
        Err(Error::Invalid(_))
    ));
    assert!(h
        .store
        .confirmed_origins(&item.id)
        .await
        .unwrap()
        .is_empty());
}

/// A fixed URL whose header reads `${API_KEY}` is not pinned: the add and change dialog named
/// its one origin with that variable. It starts in Personal and in the org's scope with
/// nothing confirmed, each with its scope's value, and has no origin to confirm.
#[tokio::test]
async fn a_fixed_url_with_a_header_variable_starts_in_an_org_without_confirmation() {
    let served = Served::start().await;
    let (h, vault, lib, _) = setup(&served, org_entries(&served, "localhost")).await;
    let fixed = LibraryConfig::McpServer(served.http.config(BTreeMap::from([(
        "Authorization".into(),
        "Bearer ${API_KEY}".into(),
    )])));
    let item = h
        .store
        .add_library_item("fixed", &fixed, None)
        .await
        .unwrap();
    let LibraryConfig::McpServer(config) = &item.config else {
        unreachable!()
    };
    assert!(!config.origin_pinned());
    assert_eq!(run(&h, &vault, &acme(), "fixed").await, (1, Vec::new()));
    assert_eq!(
        run(&h, &vault, &DotScope::Personal, "fixed").await,
        (1, Vec::new())
    );
    assert_eq!(
        served.auths(),
        [
            format!("Bearer {ORG_KEY}"),
            format!("Bearer {PERSONAL_KEY}")
        ]
    );
    assert!(h
        .store
        .confirmed_origins(&item.id)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        h.store
            .add_confirmed_origin(&item.id, ACME, &served.origin("127.0.0.1"))
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        lib.origin_in(&item.id, &acme()).await,
        Err(Error::Invalid(_))
    ));
}

// The store's confirmed origins

#[tokio::test]
async fn confirmed_origins_are_kept_per_scope_once_each() {
    let served = Served::start().await;
    let (h, _vault, _lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    let store = &h.store;
    assert!(store.confirmed_origins(&item.id).await.unwrap().is_empty());
    for (scope, origin) in [
        (PERSONAL_SCOPE, "https://api.github.com"),
        (ACME, "https://ghe.acme.example:8443"),
        (PERSONAL_SCOPE, "http://127.0.0.1:9"),
        (PERSONAL_SCOPE, "https://api.github.com"),
        (ACME, "https://ghe.acme.example:8443"),
    ] {
        store
            .add_confirmed_origin(&item.id, scope, origin)
            .await
            .unwrap();
    }
    assert_eq!(
        store.confirmed_origins(&item.id).await.unwrap(),
        origins(&[
            (ACME, &["https://ghe.acme.example:8443"]),
            (
                PERSONAL_SCOPE,
                &["https://api.github.com", "http://127.0.0.1:9"]
            ),
        ])
    );
    // A scope is checked as a catalog's is; an origin is one `http_origin` gives.
    for scope in ["Acme", "", "acme_eng"] {
        let err = store
            .add_confirmed_origin(&item.id, scope, "https://api.github.com")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{scope}: {err:?}");
    }
    for origin in [
        "https://api.github.com/",
        "https://API.github.com",
        "https://api.github.com:443",
        "https://api.github.com/mcp",
        "ftp://files.example",
        "api.github.com",
        "",
    ] {
        match store
            .add_confirmed_origin(&item.id, PERSONAL_SCOPE, origin)
            .await
        {
            Err(Error::Invalid(why)) => assert_eq!(
                why,
                format!(
                    "{origin:?} is not an origin: it reads like https://host or \
                     https://host:8443"
                )
            ),
            other => panic!("{origin:?}: expected Invalid, got {other:?}"),
        }
    }
    // Only an MCP server has origins.
    let skill = store
        .add_library_item(
            "pdf",
            &LibraryConfig::Skill(SkillConfig {
                path: "C:/skills/pdf".into(),
            }),
            None,
        )
        .await
        .unwrap();
    for id in [skill.id.as_str(), "01NOSUCHSERVER"] {
        assert!(matches!(
            store
                .add_confirmed_origin(id, PERSONAL_SCOPE, "https://api.github.com")
                .await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            store.confirmed_origins(id).await,
            Err(Error::NotFound(_))
        ));
    }
}

/// Deleting a server deletes its origins: one added again under its name starts with none.
#[tokio::test]
async fn deleting_a_server_deletes_its_confirmed_origins() {
    let served = Served::start().await;
    let (h, _vault, _lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    h.store
        .add_confirmed_origin(&item.id, ACME, "https://ghe.acme.example")
        .await
        .unwrap();
    h.store.remove_library_item(&item.id).await.unwrap();
    assert!(matches!(
        h.store.confirmed_origins(&item.id).await,
        Err(Error::NotFound(_))
    ));
    let again = h
        .store
        .add_library_item("gh", &pinned_config(), None)
        .await
        .unwrap();
    assert!(h
        .store
        .confirmed_origins(&again.id)
        .await
        .unwrap()
        .is_empty());
}

// At spawn

#[tokio::test]
async fn a_personal_dots_server_starts_only_where_its_origin_was_confirmed() {
    let served = Served::start().await;
    let (h, vault, lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    let origin = served.origin("127.0.0.1");

    let (opened, notices) = run(&h, &vault, &DotScope::Personal, "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(notices, [not_confirmed("gh", &origin)]);
    assert!(served.auths().is_empty(), "nothing was sent");

    // The origin a Personal run reaches, as the server form shows it.
    assert_eq!(
        lib.origin_in(&item.id, &DotScope::Personal).await.unwrap(),
        Ok(HttpOrigin {
            ascii: origin.clone(),
            unicode: None
        })
    );
    lib.add_confirmed_origin(&item.id, &DotScope::Personal, &origin)
        .await
        .unwrap();
    let (opened, notices) = run(&h, &vault, &DotScope::Personal, "gh").await;
    assert_eq!((opened, notices), (1, Vec::<String>::new()));
    assert_eq!(served.auths(), [format!("Bearer {PERSONAL_KEY}")]);
}

#[tokio::test]
async fn a_personal_value_that_moves_the_host_is_refused() {
    let served = Served::start().await;
    let (h, vault, lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    lib.add_confirmed_origin(&item.id, &DotScope::Personal, &served.origin("127.0.0.1"))
        .await
        .unwrap();
    vault
        .set_value("MCP_HOST", &served.at("localhost"))
        .await
        .unwrap();

    let (opened, notices) = run(&h, &vault, &DotScope::Personal, "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(notices, [not_confirmed("gh", &served.origin("localhost"))]);
    assert!(served.auths().is_empty(), "nothing was sent");
    // Moved back, it starts again.
    vault
        .set_value("MCP_HOST", &served.at("127.0.0.1"))
        .await
        .unwrap();
    assert_eq!(run(&h, &vault, &DotScope::Personal, "gh").await.0, 1);
}

#[tokio::test]
async fn an_org_dots_server_starts_once_the_orgs_origin_is_confirmed() {
    let served = Served::start().await;
    let (h, vault, lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    // Confirmed for Personal only: the org's value reaches another origin.
    lib.add_confirmed_origin(&item.id, &DotScope::Personal, &served.origin("127.0.0.1"))
        .await
        .unwrap();
    let in_org = served.origin("localhost");
    let (opened, notices) = run(&h, &vault, &acme(), "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(notices, [not_confirmed("gh", &in_org)]);
    assert!(served.auths().is_empty(), "nothing was sent");

    // The origin the org's values reach, as if all were granted, as its Check would.
    let shown = lib.origin_in(&item.id, &acme()).await.unwrap().unwrap();
    assert_eq!(shown.ascii, in_org);
    lib.add_confirmed_origin(&item.id, &acme(), &shown.ascii)
        .await
        .unwrap();
    assert_eq!(
        h.store.confirmed_origins(&item.id).await.unwrap(),
        origins(&[
            (ACME, &[in_org.as_str()]),
            (PERSONAL_SCOPE, &[served.origin("127.0.0.1").as_str()]),
        ])
    );
    assert_eq!(run(&h, &vault, &acme(), "gh").await, (1, Vec::new()));
    assert_eq!(
        run(&h, &vault, &DotScope::Personal, "gh").await,
        (1, Vec::new())
    );
    assert_eq!(
        served.auths(),
        [
            format!("Bearer {ORG_KEY}"),
            format!("Bearer {PERSONAL_KEY}")
        ]
    );
}

#[tokio::test]
async fn the_orgs_value_moving_the_host_is_refused_while_personal_stays_confirmed() {
    let served = Served::start().await;
    let (h, vault, lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    lib.add_confirmed_origin(&item.id, &DotScope::Personal, &served.origin("127.0.0.1"))
        .await
        .unwrap();
    lib.add_confirmed_origin(&item.id, &acme(), &served.origin("localhost"))
        .await
        .unwrap();
    assert_eq!(run(&h, &vault, &acme(), "gh").await.0, 1);

    // The org's next fetch moves its host.
    let extra = json!({"personal": "deny", "plain": ["MCP_HOST"]});
    let signed = sign(&h.keys, &policy(ACME, "2026-10-22T08:30:00Z", extra));
    h.serve("dots", org_entries(&served, "127.0.0.2"), &signed);
    h.orgs.refresh(ACME).await.unwrap();

    let (opened, notices) = run(&h, &vault, &acme(), "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(notices, [not_confirmed("gh", &served.origin("127.0.0.2"))]);
    assert_eq!(
        run(&h, &vault, &DotScope::Personal, "gh").await,
        (1, Vec::new())
    );
    assert_eq!(
        served.auths(),
        [
            format!("Bearer {ORG_KEY}"),
            format!("Bearer {PERSONAL_KEY}")
        ]
    );
}

/// A change keeps the origins: one that keeps the origin still starts, one that moves it no
/// longer matches, and the old origin stays confirmed for when the URL comes back to it.
#[tokio::test]
async fn a_changed_server_keeps_its_origins_and_a_moved_one_does_not_match() {
    let served = Served::start().await;
    let (h, vault, lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    let here = served.origin("127.0.0.1");
    lib.add_confirmed_origin(&item.id, &DotScope::Personal, &here)
        .await
        .unwrap();
    let with = |url: &str, extra: &str| {
        LibraryConfig::McpServer(McpServerConfig::Http {
            url: url.into(),
            headers: BTreeMap::from([
                ("Authorization".into(), "Bearer ${API_KEY}".into()),
                ("X-Client".into(), extra.into()),
            ]),
        })
    };
    // A header changed, the origin the same.
    h.store
        .update_library_item(&item.id, &with("http://${MCP_HOST}/mcp", "dots"))
        .await
        .unwrap();
    assert_eq!(run(&h, &vault, &DotScope::Personal, "gh").await.0, 1);
    // Moved to another origin, its URL still reading a variable: the stored one does not match.
    let moved = format!("{}/mcp?via=${{MCP_HOST}}", served.origin("localhost"));
    h.store
        .update_library_item(&item.id, &with(&moved, "dots"))
        .await
        .unwrap();
    assert_eq!(
        h.store.confirmed_origins(&item.id).await.unwrap(),
        origins(&[(PERSONAL_SCOPE, &[here.as_str()])])
    );
    let (opened, notices) = run(&h, &vault, &DotScope::Personal, "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(notices, [not_confirmed("gh", &served.origin("localhost"))]);
    // Back on the first origin, it starts without a new confirmation.
    h.store
        .update_library_item(&item.id, &with("http://${MCP_HOST}/mcp", "dots-2"))
        .await
        .unwrap();
    assert_eq!(run(&h, &vault, &DotScope::Personal, "gh").await.0, 1);
}

/// `origin_in` says why there is no origin, naming the variable and the scope, never a value.
#[tokio::test]
async fn an_origin_that_cannot_be_computed_says_why_without_a_value() {
    let served = Served::start().await;
    // The org lacks `MCP_HOST`, and `personal: deny` takes nothing from Personal.
    let (h, vault, lib, item) = setup(&served, json!({"API_KEY": ORG_KEY})).await;
    let why = lib.origin_in(&item.id, &acme()).await.unwrap().unwrap_err();
    assert_eq!(why, "MCP_HOST: not in Acme Engineering");
    vault.remove("MCP_HOST").await.unwrap();
    let why = lib
        .origin_in(&item.id, &DotScope::Personal)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(why, "MCP_HOST is not set");
    // A run in either scope leaves it out for the variable, as before.
    let (opened, notices) = run(&h, &vault, &DotScope::Personal, "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(
        notices,
        ["MCP_HOST: not in Personal (dot 'triage', mcp:gh)"]
    );
    let (opened, notices) = run(&h, &vault, &acme(), "gh").await;
    assert_eq!(opened, 0);
    assert_eq!(
        notices,
        ["MCP_HOST: not in Acme Engineering (dot 'triage', mcp:gh)"]
    );
}

// Values Dots moves itself

/// A server added with a secret typed into a header sends a variable once Dots moves the value
/// into Personal. Its URL reads none, so its origin cannot move: it is not pinned, nothing is
/// confirmed, and the server starts as before. A URL that reads a variable is pinned, and
/// confirming it is the form's question.
#[tokio::test]
async fn a_typed_secret_that_dots_moves_leaves_a_fixed_url_unpinned() {
    let served = Served::start().await;
    let (h, vault, lib, _) = setup(&served, org_entries(&served, "localhost")).await;
    let typed = LibraryConfig::McpServer(served.http.config(BTreeMap::from([(
        "Authorization".into(),
        format!("Bearer {LITERAL}"),
    )])));
    let item = lib.add("typed", typed, None).await.unwrap();
    let LibraryConfig::McpServer(saved) = &item.config else {
        unreachable!()
    };
    assert!(!saved.variables().is_empty(), "{saved:?}");
    assert!(!saved.origin_pinned(), "{saved:?}");
    assert_clean(&serde_json::to_string(saved).unwrap());
    assert!(h
        .store
        .confirmed_origins(&item.id)
        .await
        .unwrap()
        .is_empty());
    let grants = Grants(
        [(
            "mcp:typed".to_string(),
            saved.variables().into_iter().collect(),
        )]
        .into(),
    );
    let uses = Uses {
        mcp_servers: vec!["typed".into()],
        ..Default::default()
    };
    let (tools, notices) = RunTools::open(OpenRequest {
        store: &h.store,
        bus: &new_bus(),
        run_id: "01RUN",
        uses: &uses,
        workspace: h.data.path(),
        plugins_dir: None,
        cancel: &CancellationToken::new(),
        vault: &vault,
        scope: &DotScope::Personal,
        grants: &grants,
        dot_name: "triage",
    })
    .await
    .unwrap();
    assert_eq!(tools.servers().len(), 1, "{notices:?}");
    tools.close().await;
    assert_eq!(served.auths(), [format!("Bearer {LITERAL}")]);

    // With a variable in the URL it is pinned, and nothing is confirmed by the move.
    let mixed = LibraryConfig::McpServer(McpServerConfig::Http {
        url: "http://${MCP_HOST}/mcp".into(),
        headers: BTreeMap::from([("Authorization".into(), format!("Bearer {LITERAL}"))]),
    });
    let item = lib.add("mixed", mixed, None).await.unwrap();
    assert!(h
        .store
        .confirmed_origins(&item.id)
        .await
        .unwrap()
        .is_empty());
}

// The Library's Check

/// The refusal of a Check whose origin is not confirmed in the scope named `scope_name`.
fn check_refused(origin: &str, scope_name: &str) -> String {
    format!("gh: {origin} is not a confirmed origin in {scope_name}")
}

/// A Check sends a scope's values only to an origin confirmed for that scope: otherwise it is
/// refused with the server, the origin and the scope's name, nothing is sent, and the catalog
/// is not touched. `origin_to_confirm` says what to ask, and nothing once it is confirmed.
#[tokio::test]
async fn a_check_is_refused_until_the_origin_is_confirmed_in_its_scope() {
    let served = Served::start().await;
    let (h, _vault, lib, item) = setup(&served, org_entries(&served, "localhost")).await;
    let (here, in_org) = (served.origin("127.0.0.1"), served.origin("localhost"));
    for (scope, origin, name) in [
        (DotScope::Personal, &here, "Personal"),
        (acme(), &in_org, "Acme Engineering"),
    ] {
        match lib.check(&item.id, &scope).await {
            Err(Error::Conflict(why)) => assert_eq!(why, check_refused(origin, name)),
            other => panic!("expected Conflict, got {other:?}"),
        }
        match lib.check_and_accept(&item.id, &scope, "fp").await {
            Err(Error::Conflict(why)) => assert_eq!(why, check_refused(origin, name)),
            other => panic!("expected Conflict, got {other:?}"),
        }
        let asked = lib.origin_to_confirm(&item.id, &scope).await.unwrap();
        assert_eq!(
            asked,
            Some(dots_core::library_service::OriginToConfirm {
                server: "gh".into(),
                origin: HttpOrigin {
                    ascii: origin.clone(),
                    unicode: None
                },
                variables: vec!["API_KEY".into(), "MCP_HOST".into()],
                scope_name: name.into(),
            })
        );
    }
    assert!(served.auths().is_empty(), "nothing was sent");
    for scope in [PERSONAL_SCOPE, ACME] {
        let catalog = h.store.get_catalog(&item.id, scope).await.unwrap();
        assert_eq!(catalog.checked_at, None, "{scope}: {catalog:?}");
    }

    // Confirmed in Personal: Personal's Check sends Personal's token; the org's is still
    // refused.
    lib.add_confirmed_origin(&item.id, &DotScope::Personal, &here)
        .await
        .unwrap();
    assert_eq!(
        lib.origin_to_confirm(&item.id, &DotScope::Personal)
            .await
            .unwrap(),
        None
    );
    let checked = lib.check(&item.id, &DotScope::Personal).await.unwrap();
    assert!(checked.checked_at.is_some(), "{checked:?}");
    assert!(matches!(
        lib.check(&item.id, &acme()).await,
        Err(Error::Conflict(_))
    ));
    lib.add_confirmed_origin(&item.id, &acme(), &in_org)
        .await
        .unwrap();
    let fp = lib
        .check(&item.id, &acme())
        .await
        .unwrap()
        .seen_fingerprint
        .unwrap();
    let accepted = lib.check_and_accept(&item.id, &acme(), &fp).await.unwrap();
    assert_eq!(accepted.accepted_fingerprint.as_deref(), Some(fp.as_str()));
    assert_eq!(
        served.auths(),
        [
            format!("Bearer {PERSONAL_KEY}"),
            format!("Bearer {ORG_KEY}"),
            format!("Bearer {ORG_KEY}")
        ]
    );
    // The listing shows what is confirmed, by scope.
    let entry = lib
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.item.id == item.id)
        .unwrap();
    assert_eq!(
        entry.confirmed_origins,
        origins(&[
            (ACME, &[in_org.as_str()]),
            (PERSONAL_SCOPE, &[here.as_str()])
        ])
    );
}

/// With no origin to confirm, `origin_to_confirm` says why, naming the scope and the variable,
/// and the Check fails on the variable as before, sending nothing.
#[tokio::test]
async fn an_origin_that_cannot_be_computed_is_not_asked_about() {
    let served = Served::start().await;
    let (h, _vault, lib, item) = setup(&served, json!({"API_KEY": ORG_KEY})).await;
    match lib.origin_to_confirm(&item.id, &acme()).await {
        Err(Error::Invalid(why)) => assert_eq!(
            why,
            "gh: there is no origin to confirm in Acme Engineering: MCP_HOST: not in Acme \
             Engineering"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }
    let catalog = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(
        catalog.error.as_deref(),
        Some("MCP_HOST: not in Acme Engineering")
    );
    assert!(served.auths().is_empty());
    // A server that is not pinned has nothing to confirm.
    let plain = LibraryConfig::McpServer(served.http.config(BTreeMap::new()));
    let plain = h
        .store
        .add_library_item("plain", &plain, None)
        .await
        .unwrap();
    assert!(matches!(
        lib.origin_to_confirm(&plain.id, &DotScope::Personal).await,
        Err(Error::Invalid(_))
    ));
}

/// A pinned server whose resolved URL gives no http origin is not checked: the failure is
/// recorded for the scope, no session opens, and nothing is sent.
#[tokio::test]
async fn a_check_whose_url_gives_no_origin_records_a_failure_and_sends_nothing() {
    let served = Served::start().await;
    let (h, _vault, lib, item) = setup(&served, org_entries(&served, "bad host")).await;
    let catalog = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(
        catalog.status,
        dots_core::library::CatalogStatus::Unreachable
    );
    assert_eq!(catalog.error.as_deref(), Some("not a valid URL"));
    assert!(served.auths().is_empty(), "no session was opened");
    let stored = h.store.get_catalog(&item.id, ACME).await.unwrap();
    assert_eq!(stored.error.as_deref(), Some("not a valid URL"));
}

/// `add` with a fingerprint to accept checks the server first. A pinned one whose origin is not
/// confirmed is saved unchecked, and nothing is sent.
#[tokio::test]
async fn adding_a_pinned_server_with_accept_sends_nothing_to_an_unconfirmed_origin() {
    let served = Served::start().await;
    let (h, _vault, lib, _) = setup(&served, org_entries(&served, "localhost")).await;
    let item = lib.add("gh2", pinned_config(), Some("fp")).await.unwrap();
    let catalog = h.store.get_catalog(&item.id, PERSONAL_SCOPE).await.unwrap();
    assert_eq!(catalog.checked_at, None, "{catalog:?}");
    assert!(served.auths().is_empty());
}

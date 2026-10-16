//! The Library's model: item names, configs and a dot's uses (MCP/skills spec sections 2 and
//! 10). `${VAR}` references are tested with the MCP client (`mcp_test.rs`).

use std::collections::BTreeMap;

use dots_core::library::{
    check_name, looks_secret, mcp_server_name, plugin_server_name, CatalogStatus, LibraryConfig,
    LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig, SkillConfig, Uses,
};
use dots_core::Error;
use serde_json::json;

fn invalid(r: dots_core::Result<()>) -> bool {
    matches!(r, Err(Error::Invalid(_)))
}

fn stdio(command: &str) -> McpServerConfig {
    McpServerConfig::Stdio {
        command: command.into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
    }
}

#[test]
fn an_mcp_server_name_is_its_tool_prefix() {
    for good in ["sf", "my-server_2", "a", &"x".repeat(40)] {
        assert!(check_name(LibraryKind::McpServer, good).is_ok(), "{good}");
    }
    // `__` would make `mcp__<server>__<tool>` ambiguous; `dots` is Dots' own server.
    for bad in [
        "",
        "Sf",
        "a.b",
        "a b",
        "a__b",
        "_a",
        "files_",
        "dots",
        &"x".repeat(41),
    ] {
        assert!(invalid(check_name(LibraryKind::McpServer, bad)), "{bad:?}");
    }
}

#[test]
fn a_skill_or_plugin_name_is_a_folder_name() {
    for kind in [LibraryKind::Skill, LibraryKind::Plugin] {
        for good in ["pdf", "pdf-tools", "Fill_Form.2", &"x".repeat(64)] {
            assert!(check_name(kind, good).is_ok(), "{good}");
        }
        for bad in [
            "",
            ".",
            "..",
            "-x",
            "a/b",
            "a\\b",
            "a:b",
            "a b",
            &"x".repeat(65),
        ] {
            assert!(invalid(check_name(kind, bad)), "{bad:?}");
        }
    }
}

#[test]
fn a_generated_server_name_is_always_a_valid_one() {
    assert_eq!(mcp_server_name("GitHub"), "github");
    assert_eq!(mcp_server_name("my.server"), "my_server");
    assert_eq!(mcp_server_name("a__b"), "a_b");
    assert_eq!(mcp_server_name("a. b"), "a_b");
    assert_eq!(mcp_server_name("_x_"), "x");
    assert_eq!(mcp_server_name("..."), "server");
    assert_eq!(mcp_server_name("dots"), "dots-server");
    assert_eq!(
        plugin_server_name("pdf-tools", "files"),
        "plugin_pdf-tools_files"
    );
    assert_eq!(plugin_server_name("my__plug", "_x"), "plugin_my_plug_x");
    let long = mcp_server_name(&"Long.Name".repeat(10));
    assert_eq!(long.len(), 40);
    assert_ne!(long, mcp_server_name(&"Long.Name".repeat(11)));
    for raw in [
        "GitHub",
        "a__b",
        "...",
        "dots",
        &"Long.Name".repeat(10),
        "é_ü__x",
    ] {
        let name = mcp_server_name(raw);
        assert!(
            check_name(LibraryKind::McpServer, &name).is_ok(),
            "{raw} -> {name}"
        );
    }
}

#[test]
fn secret_looking_keys() {
    for key in [
        "SF_TOKEN",
        "api_key",
        "CLIENT_SECRET",
        "Password",
        "Authorization",
        "X-Auth",
    ] {
        assert!(looks_secret(key), "{key}");
    }
    for key in ["MODE", "PATH", "Accept", "region"] {
        assert!(!looks_secret(key), "{key}");
    }
}

#[test]
fn server_configs_read_as_the_spec_writes_them() {
    let s: McpServerConfig = serde_json::from_value(json!({
        "transport": "stdio", "command": "npx", "args": ["-y", "some-server"], "cwd": null,
        "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly" }
    }))
    .unwrap();
    let McpServerConfig::Stdio {
        command, args, env, ..
    } = &s
    else {
        panic!("{s:?}")
    };
    assert_eq!(command, "npx");
    assert_eq!(args, &["-y", "some-server"]);
    assert_eq!(env["MODE"], "readonly");
    let h: McpServerConfig = serde_json::from_value(json!({
        "transport": "http", "url": "https://example.com/mcp",
        "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" }
    }))
    .unwrap();
    assert!(matches!(h, McpServerConfig::Http { .. }));
    // Optional parts may be left out.
    let bare: McpServerConfig =
        serde_json::from_value(json!({"transport": "stdio", "command": "x"})).unwrap();
    assert_eq!(bare, stdio("x"));
    let round = serde_json::to_value(&s).unwrap();
    assert_eq!(serde_json::from_value::<McpServerConfig>(round).unwrap(), s);
}

#[test]
fn an_item_carries_its_kind_next_to_its_config() {
    let item = LibraryItem {
        id: "01J0LIB".into(),
        name: "pdf".into(),
        config: LibraryConfig::Skill(SkillConfig {
            path: "C:/skills/pdf".into(),
        }),
        owner_id: None,
        created_at: "t".into(),
        updated_at: "t".into(),
    };
    let v = serde_json::to_value(&item).unwrap();
    assert_eq!(
        v,
        json!({"id": "01J0LIB", "name": "pdf", "kind": "skill",
               "config": {"path": "C:/skills/pdf"}, "owner_id": null,
               "created_at": "t", "updated_at": "t"})
    );
    assert_eq!(serde_json::from_value::<LibraryItem>(v).unwrap(), item);
    assert_eq!(item.kind(), LibraryKind::Skill);
    let plugin: LibraryConfig = serde_json::from_value(
        json!({"kind": "plugin", "config": {"path": "C:/p", "installed_id": null}}),
    )
    .unwrap();
    assert_eq!(
        plugin,
        LibraryConfig::Plugin(PluginConfig {
            path: "C:/p".into(),
            installed_id: None,
            run_hooks: false,
        })
    );
}

#[test]
fn configs_are_checked() {
    assert!(LibraryConfig::McpServer(stdio("npx")).validate().is_ok());
    assert!(invalid(LibraryConfig::McpServer(stdio(" ")).validate()));
    let http = |url: &str| {
        LibraryConfig::McpServer(McpServerConfig::Http {
            url: url.into(),
            headers: BTreeMap::new(),
        })
    };
    assert!(http("https://example.com/mcp").validate().is_ok());
    assert!(http("http://127.0.0.1:9000/mcp").validate().is_ok());
    assert!(invalid(http("example.com/mcp").validate()));
    assert!(invalid(
        LibraryConfig::Skill(SkillConfig { path: "".into() }).validate()
    ));
    assert!(invalid(
        LibraryConfig::Plugin(PluginConfig {
            path: " ".into(),
            installed_id: None,
            run_hooks: false
        })
        .validate()
    ));
}

#[test]
fn uses_default_to_nothing() {
    let uses: Uses = serde_json::from_value(json!({})).unwrap();
    assert_eq!(uses, Uses::default());
    assert!(uses.is_empty());
    assert_eq!(
        serde_json::to_value(&uses).unwrap(),
        json!({"mcp_servers": [], "skills": [], "plugins": []})
    );
}

#[test]
fn a_new_catalog_is_unchecked() {
    let c = McpCatalog::unchecked("01J0SRV");
    assert_eq!(c.server_id, "01J0SRV");
    assert_eq!(c.status, CatalogStatus::Unchecked);
    assert_eq!(c.accepted_fingerprint, None);
    assert_eq!(c.seen_tools, None);
    assert_eq!(
        CatalogStatus::parse("changed").unwrap(),
        CatalogStatus::Changed
    );
}

#[test]
fn control_and_direction_characters_are_refused_in_what_a_dialog_shows() {
    let with = |args: &[&str], cwd: Option<&str>, env: &[(&str, &str)]| {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: "node".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: cwd.map(String::from),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        })
    };
    assert!(with(&["a b", "--x=\u{e9}"], Some("C:/d"), &[("A", "${A}")])
        .validate()
        .is_ok());
    for bad in [
        "\n", "\r", "\t", "\u{7f}", "\u{85}", "\u{202e}", "\u{2066}", "\u{2069}", "\u{200f}",
        "\u{061c}",
    ] {
        let text = format!("x{bad}y");
        assert!(invalid(with(&[&text], None, &[]).validate()), "arg {bad:?}");
        assert!(
            invalid(with(&[], Some(&text), &[]).validate()),
            "cwd {bad:?}"
        );
        assert!(
            invalid(with(&[], None, &[(&text, "v")]).validate()),
            "key {bad:?}"
        );
        let url = LibraryConfig::McpServer(McpServerConfig::Http {
            url: format!("https://h/{text}"),
            headers: BTreeMap::new(),
        });
        assert!(invalid(url.validate()), "url {bad:?}");
        let header = LibraryConfig::McpServer(McpServerConfig::Http {
            url: "https://h/".into(),
            headers: BTreeMap::from([(text, "v".to_string())]),
        });
        assert!(invalid(header.validate()), "header {bad:?}");
    }
    assert_eq!(
        dots_core::library::http_origin("HTTPS://user:p@Host.example:8443/a?b#c").as_deref(),
        Some("https://host.example:8443")
    );
    assert_eq!(dots_core::library::http_origin("ftp://h/"), None);
}

#[test]
fn the_origin_is_the_one_the_http_client_uses() {
    let o = |u: &str| dots_core::library::http_origin(u);
    // A backslash ends the authority for http(s): the host is evil.example.
    assert_eq!(
        o(r"https://evil.example\@api.github.com/mcp").as_deref(),
        Some("https://evil.example")
    );
    // Default port, host case and a trailing dot's absence do not make another origin.
    assert_eq!(o("https://h.example:443/a"), o("HTTPS://H.Example/b"));
    assert_ne!(o("https://h.example:444/"), o("https://h.example/"));
    assert_eq!(o("http://h.example:80/"), o("http://h.example/"));
    // A variable in the host is only known at start: no origin, the raw url is compared.
    assert_eq!(o("https://${HOST}/mcp"), None);
    assert_eq!(o("https://x.${Domain}:8443/mcp"), None);
    // A variable in the path leaves the origin.
    assert_eq!(
        o("https://h.example/${P}").as_deref(),
        Some("https://h.example")
    );
}

#[test]
fn an_env_value_may_hold_any_text_but_a_key_may_not() {
    let env = |k: &str, v: &str| {
        LibraryConfig::McpServer(McpServerConfig::Stdio {
            command: "node".into(),
            args: vec![],
            cwd: None,
            env: BTreeMap::from([(k.to_string(), v.to_string())]),
        })
    };
    assert!(env("CERT", "-----BEGIN-----\n{\"a\":\t1}\n\u{202e}")
        .validate()
        .is_ok());
    assert!(invalid(env("A\nB", "v").validate()));
}

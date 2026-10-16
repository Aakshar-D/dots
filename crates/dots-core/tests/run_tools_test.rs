//! A run's tools (MCP/skills spec sections 5 to 7): the MCP sessions of the servers a dot uses,
//! its plugins' servers included, checked against their catalogs and held when their list
//! changed; the skills it reads; the plugin folders of a Claude-engine run; the items left out;
//! and the close at run end.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::fake_mcp::{tool, FakeMcp};
use common::temp_store;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, PluginConfig, SkillConfig, Uses,
};
use dots_core::library_service::LibraryService;
use dots_core::policy::McpTrust;
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::store::Store;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn skill_dir(root: &Path, name: &str) -> String {
    let dir = root.join(name);
    write(
        &dir.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: The {name} skill.\n---\nBody.\n"),
    );
    dir.to_string_lossy().into_owned()
}

fn plugin_dir(root: &Path, name: &str, servers: &[(&str, &FakeMcp)]) -> String {
    let dir = root.join(name);
    write(
        &dir.join(".claude-plugin").join("plugin.json"),
        &json!({ "name": name }).to_string(),
    );
    write(
        &dir.join("skills").join("fill").join("SKILL.md"),
        "---\nname: fill\ndescription: Fills forms.\n---\nBody.\n",
    );
    let mut map = serde_json::Map::new();
    for (server, fake) in servers {
        map.insert(
            server.to_string(),
            json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [fake.script()]}),
        );
    }
    write(
        &dir.join(".mcp.json"),
        &json!({ "mcpServers": map }).to_string(),
    );
    dir.to_string_lossy().into_owned()
}

fn plugin(path: &str, installed_id: Option<&str>) -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: path.into(),
        installed_id: installed_id.map(str::to_string),
        run_hooks: false,
    })
}

struct Fixture {
    dir: TempDir,
    store: Store,
    bus: Bus,
    lib: LibraryService,
}

async fn fixture() -> Fixture {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = LibraryService::new(store.clone(), bus.clone());
    Fixture {
        dir,
        store,
        bus,
        lib,
    }
}

impl Fixture {
    /// Adds the server `name` on `fake`'s script with its current list accepted.
    async fn accepted_server(&self, name: &str, fake: &FakeMcp) -> String {
        let probe = self.lib.probe(&fake.stdio()).await.unwrap();
        self.lib
            .add(
                name,
                LibraryConfig::McpServer(fake.stdio()),
                Some(&probe.fingerprint),
            )
            .await
            .unwrap()
            .id
    }

    async fn open(&self, uses: &Uses, plugins_dir: Option<&Path>) -> (RunTools, Vec<String>) {
        RunTools::open(OpenRequest {
            store: &self.store,
            bus: &self.bus,
            run_id: "01RUN",
            uses,
            workspace: self.dir.path(),
            plugins_dir,
            cancel: &CancellationToken::new(),
        })
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_run_opens_its_servers_and_its_plugins_servers() {
    let f = fixture().await;
    let files = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let files_id = f.accepted_server("files", &files).await;
    let alpha = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let path = plugin_dir(f.dir.path(), "forms", &[("alpha", &alpha)]);
    f.lib.add("forms", plugin(&path, None), None).await.unwrap();
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        skills: vec![],
        plugins: vec!["forms".into()],
    };
    let (tools, notices) = f.open(&uses, None).await;

    let names: Vec<&str> = tools.servers().iter().map(|s| s.name()).collect();
    assert_eq!(names, ["files", "plugin-forms-alpha"]);
    let srv = tools.server("files").unwrap();
    assert_eq!(srv.item_id(), files_id);
    assert!(!srv.held());
    let listed: Vec<String> = srv.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(listed, ["read", "write"]);
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: false,
            read_only: true
        }
    );
    assert_eq!(
        tools.trust("files", "write"),
        Some(McpTrust {
            held: false,
            read_only: false
        })
    );
    // A tool the accepted list does not have asks (spec section 4, step 2).
    assert!(srv.trust("other").held);
    assert_eq!(tools.trust("nope", "read"), None);
    assert_eq!(
        srv.accepted_fingerprint(),
        f.store
            .get_catalog(&files_id)
            .await
            .unwrap()
            .accepted_fingerprint
    );

    // The plugin's server was never accepted: it is held, and the run says so.
    let owned = tools.server("plugin-forms-alpha").unwrap();
    assert!(owned.held());
    assert!(owned.trust("a").held);
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(
        notices[0].starts_with("plugin-forms-alpha: "),
        "{notices:?}"
    );

    // The session works.
    let result = srv
        .session()
        .call_tool(
            "read",
            json!({"q": "x"}),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("read"));

    // The local engine reads skills from the Library folders; no plugin folders are made.
    let skills: Vec<&str> = tools.skills().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skills, ["forms:fill"]);
    assert_eq!(tools.skills()[0].plugin.as_deref(), Some("forms"));
    assert!(tools.plugin_dirs().is_empty());

    tools.close().await;
    assert!(files.gone_within(Duration::from_secs(10)).await);
    assert!(alpha.gone_within(Duration::from_secs(10)).await);
    tools.close().await;
}

#[tokio::test]
async fn a_list_that_changed_since_it_was_accepted_holds_the_server() {
    let f = fixture().await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let id = f.accepted_server("files", &fake).await;
    fake.rewrite(json!({"tools": [tool("read", true), tool("drop", false)]}));
    let mut rx = f.bus.subscribe();
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    let srv = tools.server("files").unwrap();
    assert!(srv.held());
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: true,
            read_only: true
        }
    );
    let catalog = f.store.get_catalog(&id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Changed);
    assert_eq!(notices.len(), 1);
    assert!(notices[0].starts_with("files: "), "{notices:?}");
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::LibraryUpdated { item_id } = ev {
            ids.push(item_id);
        }
    }
    assert_eq!(ids, vec![id]);
    tools.close().await;
}

#[tokio::test]
async fn a_server_that_cannot_start_is_left_out_with_a_run_event() {
    let f = fixture().await;
    let good = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    f.accepted_server("good", &good).await;
    let bad = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let bad_cfg = LibraryConfig::McpServer(
        bad.stdio_with(vec!["${DOTS_T5_NEVER_SET_RUN}".into()], Default::default()),
    );
    let bad_id = f.lib.add("bad", bad_cfg, None).await.unwrap().id;
    let uses = Uses {
        mcp_servers: vec!["bad".into(), "good".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    let names: Vec<&str> = tools.servers().iter().map(|s| s.name()).collect();
    assert_eq!(names, ["good"]);
    assert_eq!(
        notices,
        ["bad: could not start: DOTS_T5_NEVER_SET_RUN is not set"]
    );
    let catalog = f.store.get_catalog(&bad_id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unreachable);
    assert_eq!(
        catalog.error.as_deref(),
        Some("could not start: DOTS_T5_NEVER_SET_RUN is not set")
    );
    tools.close().await;
}

#[tokio::test]
async fn a_claude_run_gets_plugin_folders_that_go_at_close() {
    let f = fixture().await;
    let pdf = skill_dir(f.dir.path(), "pdf");
    let csv = skill_dir(f.dir.path(), "csv");
    f.lib
        .add("pdf", LibraryConfig::Skill(SkillConfig { path: pdf }), None)
        .await
        .unwrap();
    f.lib
        .add(
            "csv",
            LibraryConfig::Skill(SkillConfig { path: csv.clone() }),
            None,
        )
        .await
        .unwrap();
    std::fs::remove_dir_all(&csv).unwrap();
    let path = plugin_dir(f.dir.path(), "forms", &[]);
    f.lib.add("forms", plugin(&path, None), None).await.unwrap();
    let uses = Uses {
        mcp_servers: vec![],
        skills: vec!["csv".into(), "pdf".into()],
        plugins: vec!["forms".into()],
    };
    let plugins_dir = f.dir.path().join("plugins");
    let (tools, notices) = f.open(&uses, Some(&plugins_dir)).await;
    assert_eq!(notices, ["skill csv: folder not found"]);
    let run_root = plugins_dir.join("run-01RUN");
    assert_eq!(
        tools.plugin_dirs(),
        [run_root.join("dots-skills"), run_root.join("forms")]
    );
    assert!(run_root.join("dots-skills/skills/pdf/SKILL.md").is_file());
    let skills: Vec<&str> = tools.skills().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(skills, ["pdf", "forms:fill"]);
    assert_eq!(tools.skill("pdf").unwrap().plugin, None);
    tools.close().await;
    assert!(!run_root.exists());
}

#[tokio::test]
async fn a_list_change_during_the_run_holds_the_server_at_once() {
    let f = fixture().await;
    let mut flip = tool("flip", false);
    flip["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "tools": [tool("read", true), flip],
        "after_switch": [tool("read", true), tool("new", true)],
    }));
    let id = f.accepted_server("files", &fake).await;
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    assert!(notices.is_empty(), "{notices:?}");
    let srv = tools.server("files").unwrap();
    assert!(!srv.held());
    srv.session()
        .call_tool(
            "flip",
            json!({}),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !srv.held() {
        assert!(Instant::now() < deadline, "the server was not held");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let listed: Vec<String> = srv.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(listed, ["read", "new"]);
    // `read` keeps the accepted list's read-only hint; `new` is not in it.
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: true,
            read_only: true
        }
    );
    assert_eq!(
        f.store.get_catalog(&id).await.unwrap().status,
        CatalogStatus::Changed
    );
    tools.close().await;
}

#[tokio::test]
async fn a_server_whose_tools_cannot_be_listed_after_a_change_is_held() {
    let f = fixture().await;
    let mut flip = tool("flip", false);
    flip["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "tools": [tool("read", true), flip],
        "after_switch": [tool("read", true), tool("new", true)],
        "fail_list_after_switch": true,
    }));
    f.accepted_server("files", &fake).await;
    let uses = Uses {
        mcp_servers: vec!["files".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    assert!(notices.is_empty(), "{notices:?}");
    let srv = tools.server("files").unwrap();
    assert!(!srv.held());
    srv.session()
        .call_tool(
            "flip",
            json!({}),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    // `settled` returns once the watcher has dealt with the announced change, and says so.
    assert!(srv.settled().await);
    assert!(srv.held(), "the server stayed trusted on the old list");
    // The old list stays, but a read-only hint no longer lets a call through.
    let listed: Vec<String> = srv.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(listed, ["read", "flip"]);
    assert_eq!(
        srv.trust("read"),
        McpTrust {
            held: true,
            read_only: true
        }
    );
    tools.close().await;
}

#[tokio::test]
async fn an_installed_plugin_that_moved_is_found_again() {
    let f = fixture().await;
    let old = plugin_dir(f.dir.path(), "forms-1.0", &[]);
    f.lib
        .add("forms", plugin(&old, Some("forms@mkt")), None)
        .await
        .unwrap();
    std::fs::remove_dir_all(&old).unwrap();
    let new = plugin_dir(f.dir.path(), "forms-1.1", &[]);
    let cache = f.dir.path().join("cache");
    write(
        &cache.join("installed_plugins.json"),
        &json!({"plugins": {"forms@mkt": [{"scope": "user", "installPath": new}]}}).to_string(),
    );
    std::env::set_var("CLAUDE_CODE_PLUGIN_CACHE_DIR", &cache);
    let uses = Uses {
        plugins: vec!["forms".into()],
        ..Default::default()
    };
    let (tools, notices) = f.open(&uses, None).await;
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(tools.skills().len(), 1);
    let item = f
        .store
        .find_library_item(LibraryKind::Plugin, "forms")
        .await
        .unwrap()
        .unwrap();
    match item.config {
        LibraryConfig::Plugin(c) => assert_eq!(c.path, new),
        other => panic!("{other:?}"),
    }
    tools.close().await;

    // Gone from Claude Code too: left out.
    std::fs::remove_dir_all(&new).unwrap();
    let (tools, notices) = f.open(&uses, None).await;
    std::env::remove_var("CLAUDE_CODE_PLUGIN_CACHE_DIR");
    assert_eq!(notices, ["plugin forms: folder not found"]);
    assert!(tools.skills().is_empty());
    tools.close().await;
}

#[tokio::test]
async fn a_cancelled_run_stops_opening_its_servers() {
    let f = fixture().await;
    let slow = FakeMcp::new(json!({"tools": [tool("read", true)], "init_delay_ms": 20000}));
    f.lib
        .add("slow", LibraryConfig::McpServer(slow.stdio()), None)
        .await
        .unwrap();
    let uses = Uses {
        mcp_servers: vec!["slow".into()],
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let (tools, _) = RunTools::open(OpenRequest {
        store: &f.store,
        bus: &f.bus,
        run_id: "01RUN",
        uses: &uses,
        workspace: f.dir.path(),
        plugins_dir: None,
        cancel: &cancel,
    })
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(tools.servers().is_empty());
    assert!(slow.gone_within(Duration::from_secs(10)).await);
}

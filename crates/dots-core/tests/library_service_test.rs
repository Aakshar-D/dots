//! The Library's runtime service (MCP/skills spec section 9): add, update and remove with the
//! confirm rule, probe, check and accept with the fingerprint guard, a plugin's own MCP
//! servers kept in line, scans, installed plugins, variables, and `LibraryUpdated`.

mod common;

use std::path::Path;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::library::{
    CatalogStatus, ContextConfig, LibraryConfig, LibraryKind, McpServerConfig, PluginConfig,
    SkillConfig,
};
use dots_core::library_service::LibraryService;
use dots_core::mcp::fingerprint;
use dots_core::store::Store;
use dots_core::vault::{DotScope, EntryKind, Grants, Vault};
use dots_core::Error;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast;

/// Held by the tests that set `CLAUDE_CODE_PLUGIN_CACHE_DIR`, which the process shares.
static CACHE_DIR_VAR: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn service(store: &Store, bus: &Bus) -> LibraryService {
    LibraryService::new(store.clone(), bus.clone())
}

fn server(fake: &FakeMcp) -> LibraryConfig {
    LibraryConfig::McpServer(fake.stdio())
}

/// The item ids of the `LibraryUpdated` events waiting on `rx`.
fn updated(rx: &mut broadcast::Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::LibraryUpdated { item_id } = ev {
            ids.push(item_id);
        }
    }
    ids
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn skill_dir(root: &Path, name: &str, description: &str) -> String {
    let dir = root.join(name);
    write(
        &dir.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: {description}\n---\nBody.\n"),
    );
    dir.to_string_lossy().into_owned()
}

/// A plugin folder named `name` with one skill and the MCP servers `servers` (name to script).
fn plugin_dir(root: &Path, name: &str, servers: &[(&str, &FakeMcp)]) -> String {
    let dir = root.join(name);
    write(
        &dir.join(".claude-plugin").join("plugin.json"),
        &json!({"name": name, "version": "1.0.0"}).to_string(),
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

fn plugin(path: &str) -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: path.into(),
        installed_id: None,
        run_hooks: false,
    })
}

#[tokio::test]
async fn probe_lists_the_tools_with_their_fingerprint_and_starts_nothing_that_stays() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true), tool("write", false)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    let names: Vec<&str> = probe.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["read", "write"]);
    assert_eq!(probe.fingerprint, fingerprint(&probe.tools));
    assert!(fake.gone_within(std::time::Duration::from_secs(10)).await);
    assert!(store.list_library_items().await.unwrap().is_empty());

    let missing = McpServerConfig::Stdio {
        command: env!("CARGO_BIN_EXE_fake-mcp").into(),
        args: vec!["${DOTS_T5_NEVER_SET_PROBE}".into()],
        cwd: None,
        env: Default::default(),
    };
    let err = lib.probe(&missing).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "could not start: DOTS_T5_NEVER_SET_PROBE is not set"
    );
}

#[tokio::test]
async fn adding_a_server_with_the_probed_fingerprint_accepts_that_list() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let mut rx = bus.subscribe();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    let item = lib
        .add("files", server(&fake), Some(&probe.fingerprint))
        .await
        .unwrap();
    let catalog = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Accepted);
    assert_eq!(
        catalog.accepted_fingerprint.as_deref(),
        Some(probe.fingerprint.as_str())
    );
    assert!(catalog.checked_at.is_some());
    assert!(updated(&mut rx).contains(&item.id));
}

#[tokio::test]
async fn a_server_added_without_a_fingerprint_stays_unchecked_and_is_not_started() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let item = lib.add("files", server(&fake), None).await.unwrap();
    assert_eq!(
        store.get_catalog(&item.id).await.unwrap().status,
        CatalogStatus::Unchecked
    );
    assert!(fake.received("initialize").is_empty());
}

#[tokio::test]
async fn adding_with_a_fingerprint_that_moved_leaves_the_list_to_review() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let item = lib
        .add("files", server(&fake), Some("not-what-it-lists"))
        .await
        .unwrap();
    let catalog = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Changed);
    assert_eq!(catalog.accepted_fingerprint, None);
    assert!(catalog.seen_fingerprint.is_some());

    let skill = LibraryConfig::Skill(SkillConfig {
        path: "C:/x".into(),
    });
    let err = lib.add("pdf", skill, Some("fp")).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
}

#[tokio::test]
async fn check_records_a_changed_list_and_accept_guards_the_fingerprint() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let first = lib.probe(&fake.stdio()).await.unwrap();
    let item = lib
        .add("files", server(&fake), Some(&first.fingerprint))
        .await
        .unwrap();

    // The same list: still accepted.
    let same = lib.check(&item.id).await.unwrap();
    assert_eq!(same.status, CatalogStatus::Accepted);

    // The server now lists another tool.
    fake.rewrite(json!({"tools": [tool("read", true), tool("drop", false)]}));
    let mut rx = bus.subscribe();
    let changed = lib.check(&item.id).await.unwrap();
    assert_eq!(changed.status, CatalogStatus::Changed);
    assert_eq!(
        changed.accepted_fingerprint.as_deref(),
        Some(first.fingerprint.as_str())
    );
    let seen = changed.seen_fingerprint.clone().unwrap();
    assert_ne!(seen, first.fingerprint);
    assert_eq!(updated(&mut rx), vec![item.id.clone()]);

    let err = lib.accept(&item.id, &first.fingerprint).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err}");
    let accepted = lib.accept(&item.id, &seen).await.unwrap();
    assert_eq!(accepted.status, CatalogStatus::Accepted);
    assert_eq!(accepted.accepted_fingerprint, Some(seen));
    assert_eq!(updated(&mut rx), vec![item.id.clone()]);
}

#[tokio::test]
async fn a_failed_check_is_unreachable_and_keeps_the_accepted_list() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let probe = lib.probe(&fake.stdio()).await.unwrap();
    let item = lib
        .add("files", server(&fake), Some(&probe.fingerprint))
        .await
        .unwrap();
    let broken = LibraryConfig::McpServer(fake.stdio_with(
        vec!["${DOTS_T5_NEVER_SET_CHECK}".into()],
        Default::default(),
    ));
    lib.update(&item.id, broken).await.unwrap();
    assert_eq!(
        store.get_catalog(&item.id).await.unwrap().status,
        CatalogStatus::Unchecked
    );
    let catalog = lib.check(&item.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unreachable);
    assert_eq!(
        catalog.error.as_deref(),
        Some("could not start: DOTS_T5_NEVER_SET_CHECK is not set")
    );
    assert_eq!(
        catalog.accepted_fingerprint.as_deref(),
        Some(probe.fingerprint.as_str())
    );
}

#[tokio::test]
async fn only_an_mcp_server_is_checked_or_accepted() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add("pdf", LibraryConfig::Skill(SkillConfig { path }), None)
        .await
        .unwrap();
    assert!(matches!(
        lib.check(&skill.id).await.unwrap_err(),
        Error::Invalid(_)
    ));
    assert!(matches!(
        lib.accept(&skill.id, "fp").await.unwrap_err(),
        Error::Invalid(_)
    ));
}

#[tokio::test]
async fn removing_an_item_that_dots_use_asks_first_and_names_them() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add("pdf", LibraryConfig::Skill(SkillConfig { path }), None)
        .await
        .unwrap();
    for name in ["nightly", "audit"] {
        let mut s = spec(name);
        s.uses.skills = vec!["pdf".into()];
        store.create_dot(&s, None).await.unwrap();
    }
    let err = lib.remove(&skill.id, false).await.unwrap_err();
    match &err {
        Error::Conflict(text) => {
            assert!(text.contains("audit") && text.contains("nightly"), "{text}")
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert!(store.get_library_item(&skill.id).await.is_ok());

    let mut rx = bus.subscribe();
    lib.remove(&skill.id, true).await.unwrap();
    assert!(store.get_library_item(&skill.id).await.is_err());
    for dot in store.list_dots().await.unwrap() {
        assert!(dot.spec.uses.skills.is_empty());
    }
    assert_eq!(updated(&mut rx), vec![skill.id.clone()]);

    // An item no dot uses goes without asking.
    let other = skill_dir(dir.path(), "csv", "Reads CSV.");
    let csv = lib
        .add(
            "csv",
            LibraryConfig::Skill(SkillConfig { path: other }),
            None,
        )
        .await
        .unwrap();
    lib.remove(&csv.id, false).await.unwrap();
}

#[tokio::test]
async fn a_skill_or_plugin_must_be_readable_to_be_added() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let missing = dir.path().join("nope").to_string_lossy().into_owned();
    let err = lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig {
                path: missing.clone(),
            }),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
    let err = lib.add("tools", plugin(&missing), None).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
    assert!(store.list_library_items().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_plugins_mcp_servers_are_kept_in_line_with_its_folder() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let a = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let b = FakeMcp::new(json!({"tools": [tool("b", true)]}));
    let path = plugin_dir(dir.path(), "forms", &[("alpha", &a), ("beta", &b)]);
    let item = lib.add("forms", plugin(&path), None).await.unwrap();
    let owned = |items: Vec<dots_core::library::LibraryItem>| -> Vec<String> {
        items
            .into_iter()
            .filter(|i| i.owner_id.as_deref() == Some(item.id.as_str()))
            .map(|i| i.name)
            .collect()
    };
    assert_eq!(
        owned(store.list_library_items().await.unwrap()),
        ["plugin-forms-alpha", "plugin-forms-beta"]
    );
    let beta = store
        .find_library_item(LibraryKind::McpServer, "plugin-forms-beta")
        .await
        .unwrap()
        .unwrap();
    let alpha = store
        .find_library_item(LibraryKind::McpServer, "plugin-forms-alpha")
        .await
        .unwrap()
        .unwrap();
    // Accept beta's list, so that a change of its config shows as a reset.
    let probe = lib.probe(&b.stdio()).await.unwrap();
    lib.check(&beta.id).await.unwrap();
    lib.accept(&beta.id, &probe.fingerprint).await.unwrap();

    // The folder changes: alpha goes, beta gets an argument, gamma comes.
    let c = FakeMcp::new(json!({"tools": [tool("c", true)]}));
    let mut servers = serde_json::Map::new();
    servers.insert(
        "beta".into(),
        json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [b.script(), "--more"]}),
    );
    servers.insert(
        "gamma".into(),
        json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": [c.script()]}),
    );
    write(
        &Path::new(&path).join(".mcp.json"),
        &json!({ "mcpServers": servers }).to_string(),
    );
    let mut rx = bus.subscribe();
    let switched = LibraryConfig::Plugin(PluginConfig {
        path: path.clone(),
        installed_id: None,
        run_hooks: true,
    });
    lib.update(&item.id, switched).await.unwrap();
    assert_eq!(
        owned(store.list_library_items().await.unwrap()),
        ["plugin-forms-beta", "plugin-forms-gamma"]
    );
    assert!(store.get_library_item(&alpha.id).await.is_err());
    let beta_now = store.get_library_item(&beta.id).await.unwrap();
    assert_eq!(beta_now.id, beta.id, "a server that stays keeps its item");
    assert_eq!(
        store.get_catalog(&beta.id).await.unwrap().status,
        CatalogStatus::Unchecked
    );
    let ids = updated(&mut rx);
    assert!(ids.contains(&item.id) && ids.contains(&alpha.id) && ids.contains(&beta.id));

    // A plugin's server is changed through its plugin only.
    let err = lib.update(&beta.id, server(&b)).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    let err = lib.remove(&beta.id, true).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");

    // Removing the plugin removes its servers.
    lib.remove(&item.id, false).await.unwrap();
    assert!(owned(store.list_library_items().await.unwrap()).is_empty());
}

#[tokio::test]
async fn the_list_says_who_uses_an_item_and_what_it_holds() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let needs = LibraryConfig::McpServer(fake.stdio_with(
        Vec::new(),
        [("TOKEN".to_string(), "${DOTS_T5_NEVER_SET_LIST}".to_string())].into(),
    ));
    lib.add("files", needs, None).await.unwrap();
    let skill_path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig {
                path: skill_path.clone(),
            }),
            None,
        )
        .await
        .unwrap();
    let pfake = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let ppath = plugin_dir(dir.path(), "forms", &[("alpha", &pfake)]);
    lib.add("forms", plugin(&ppath), None).await.unwrap();
    let mut s = spec("nightly");
    s.uses.mcp_servers = vec!["files".into()];
    s.uses.plugins = vec!["forms".into()];
    let dot = store.create_dot(&s, None).await.unwrap();
    // The skill's folder goes away after it was added.
    std::fs::remove_dir_all(&skill_path).unwrap();

    let list = lib.list().await.unwrap();
    let entry = |name: &str| list.iter().find(|e| e.item.name == name).unwrap();
    let files = entry("files");
    assert_eq!(files.used_by.len(), 1);
    assert_eq!(files.used_by[0].id, dot.id);
    assert_eq!(files.used_by[0].name, "nightly");
    assert_eq!(files.missing_variables, ["DOTS_T5_NEVER_SET_LIST"]);
    assert_eq!(
        files.catalog.as_ref().unwrap().status,
        CatalogStatus::Unchecked
    );
    let owned = entry("plugin-forms-alpha");
    assert_eq!(owned.used_by[0].name, "nightly", "used through its plugin");
    let forms = entry("forms");
    let parts = forms.plugin.as_ref().unwrap();
    assert_eq!(parts.skills, ["forms:fill"]);
    assert_eq!(parts.servers, ["plugin-forms-alpha"]);
    assert!(!parts.has_hooks);
    assert_eq!(parts.version.as_deref(), Some("1.0.0"));
    assert!(forms.problem.is_none());
    let pdf = entry("pdf");
    assert_eq!(pdf.item.id, skill.id);
    assert!(pdf.used_by.is_empty());
    assert_eq!(pdf.problem.as_deref(), Some("folder not found"));
    assert!(pdf.skill.is_none());
    // Serialized flat, as the UI reads it.
    let v: Value = serde_json::to_value(files).unwrap();
    assert_eq!(v["kind"], "mcp_server");
    assert_eq!(v["name"], "files");
    assert_eq!(v["catalog"]["status"], "unchecked");
}

#[tokio::test]
async fn a_skill_entry_carries_its_description() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    lib.add("pdf", LibraryConfig::Skill(SkillConfig { path }), None)
        .await
        .unwrap();
    let list = lib.list().await.unwrap();
    assert_eq!(list[0].skill.as_ref().unwrap().description, "Reads PDFs.");
}

#[tokio::test]
async fn scanning_a_folder_lists_its_skills_and_flags_bad_names() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    skill_dir(dir.path(), "pdf", "Reads PDFs.");
    write(
        &dir.path().join("odd").join("SKILL.md"),
        "---\nname: \"has space\"\ndescription: Odd.\n---\n",
    );
    let found = lib
        .scan_skills(&dir.path().to_string_lossy())
        .await
        .unwrap();
    let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["has space", "pdf"]);
    assert!(found[0].name_error.is_some());
    assert_eq!(found[1].name_error, None);
    assert_eq!(found[1].description, "Reads PDFs.");
    assert!(found[1].path.ends_with("pdf"));
}

#[tokio::test]
async fn environment_variables_say_which_personal_has_and_never_their_values() {
    let (_dir, store) = temp_store().await;
    let bus = new_bus();
    let vault = Vault::in_memory(store.clone());
    vault
        .add("DOTS_T5_SET_VAR", EntryKind::Secret, "secret-value")
        .await
        .unwrap();
    // Dots' own environment no longer counts (env vault spec §8).
    std::env::set_var("DOTS_T5_NEVER_SET_ENV", "from-the-environment");
    let lib = service(&store, &bus).with_vault(vault);
    let vars = lib
        .environment_variables(&[
            "DOTS_T5_SET_VAR".to_string(),
            "DOTS_T5_NEVER_SET_ENV".to_string(),
        ])
        .await
        .unwrap();
    let v = serde_json::to_value(&vars).unwrap();
    assert_eq!(
        v,
        json!([{"name": "DOTS_T5_SET_VAR", "set": true},
               {"name": "DOTS_T5_NEVER_SET_ENV", "set": false}])
    );
    assert!(!v.to_string().contains("secret-value"));
}

#[tokio::test]
async fn installed_plugins_are_read_from_the_cache_folder() {
    let _var = CACHE_DIR_VAR.lock().await;
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let install = dir.path().join("cache").join("forms");
    std::fs::create_dir_all(&install).unwrap();
    write(
        &dir.path().join("installed_plugins.json"),
        &json!({"plugins": {"forms@mkt": [{"scope": "user", "installPath": install,
                                           "version": "1.0.0"}]}})
        .to_string(),
    );
    std::env::set_var("CLAUDE_CODE_PLUGIN_CACHE_DIR", dir.path());
    let found = lib.installed_plugins();
    std::env::remove_var("CLAUDE_CODE_PLUGIN_CACHE_DIR");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "forms@mkt");
    assert!(found[0].exists);
    let v = serde_json::to_value(&found[0]).unwrap();
    assert_eq!(v["install_path"], json!(install));
}

async fn runtime(dir: &TempDir) -> dots_core::Runtime {
    let mut cfg = dots_core::Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(env!("CARGO_BIN_EXE_fake-claude").into());
    dots_core::Runtime::start(cfg).await.unwrap()
}

#[tokio::test]
async fn the_inbox_lists_servers_whose_tools_changed_and_counts_them() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(&dir).await;
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let item = rt
        .library()
        .add("files", server(&fake), Some("an-old-list"))
        .await
        .unwrap();
    let mut s = spec("nightly");
    s.uses.mcp_servers = vec!["files".into()];
    rt.create_dot(s, None).await.unwrap();
    let inbox = rt.inbox().await.unwrap();
    assert_eq!(inbox.tools_changed.len(), 1);
    let changed = &inbox.tools_changed[0];
    assert_eq!(changed.server_id, item.id);
    assert_eq!(changed.name, "files");
    assert_eq!(changed.dots.len(), 1);
    assert_eq!(changed.dots[0].name, "nightly");
    assert_eq!(rt.inbox_count().await.unwrap(), 1);

    let seen = rt
        .store()
        .get_catalog(&item.id)
        .await
        .unwrap()
        .seen_fingerprint
        .unwrap();
    rt.library().accept(&item.id, &seen).await.unwrap();
    assert!(rt.inbox().await.unwrap().tools_changed.is_empty());
    assert_eq!(rt.inbox_count().await.unwrap(), 0);
    rt.shutdown();
}

#[tokio::test]
async fn starting_dots_removes_plugin_folders_a_crash_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let left = dir.path().join("data").join("plugins").join("run-01OLD");
    write(&left.join("dots-skills").join("x.txt"), "x");
    let keep = dir.path().join("data").join("plugins").join("other");
    std::fs::create_dir_all(&keep).unwrap();
    let rt = runtime(&dir).await;
    assert!(!left.exists());
    assert!(keep.exists());
    rt.shutdown();
}

#[tokio::test]
async fn runs_opening_one_plugin_together_after_its_servers_changed_all_succeed() {
    use dots_core::library::Uses;
    use dots_core::run_tools::{OpenRequest, RunTools};
    use tokio_util::sync::CancellationToken;

    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let a = FakeMcp::new(json!({"tools": [tool("a", true)]}));
    let b = FakeMcp::new(json!({"tools": [tool("b", true)]}));
    let path = plugin_dir(dir.path(), "forms", &[("alpha", &a), ("beta", &b)]);
    let item = lib.add("forms", plugin(&path), None).await.unwrap();

    // The servers cannot start (a second copy of one fake server would wait for the first), which
    // leaves them out of the run with their own notice, never a "left out" one from the sync.
    // The folder now lists beta (changed) and gamma (new) but no alpha.
    let mut servers = serde_json::Map::new();
    servers.insert(
        "beta".into(),
        json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": ["${DOTS_T5_NEVER_SET_SYNC}"]}),
    );
    servers.insert(
        "gamma".into(),
        json!({"command": env!("CARGO_BIN_EXE_fake-mcp"), "args": ["${DOTS_T5_NEVER_SET_SYNC}", "gamma"]}),
    );
    write(
        &Path::new(&path).join(".mcp.json"),
        &json!({ "mcpServers": servers }).to_string(),
    );

    let uses = Uses {
        plugins: vec!["forms".into()],
        ..Default::default()
    };
    let open = || {
        let (store, bus, uses, ws) = (&store, &bus, &uses, dir.path());
        async move {
            RunTools::open(OpenRequest {
                store,
                bus,
                run_id: "01RUN",
                uses,
                workspace: ws,
                plugins_dir: None,
                cancel: &CancellationToken::new(),
                vault: &Vault::in_memory(store.clone()),
                scope: &DotScope::Personal,
                grants: &Grants::default(),
                dot_name: "forms",
            })
            .await
        }
    };
    let (one, two) = tokio::join!(open(), open());
    let (one, notices_one) = one.unwrap();
    let (two, notices_two) = two.unwrap();
    for notice in notices_one.iter().chain(&notices_two) {
        assert!(!notice.contains("left out"), "{notice}");
    }
    one.close().await;
    two.close().await;
    let owned: Vec<String> = store
        .owned_library_items(&item.id)
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert_eq!(owned, ["plugin-forms-beta", "plugin-forms-gamma"]);
}

/// A run that finds an installed plugin at a new folder stores the folder and switches its hooks
/// off: the hooks the person allowed were those of the old folder. The run copies the plugin
/// without them and says so; a later run says nothing more.
#[tokio::test]
async fn an_installed_plugin_that_moved_has_its_hooks_switched_off() {
    use dots_core::library::Uses;
    use dots_core::run_tools::{OpenRequest, RunTools};
    use tokio_util::sync::CancellationToken;

    let _var = CACHE_DIR_VAR.lock().await;
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let hooky = |name: &str| {
        let path = plugin_dir(dir.path(), name, &[]);
        write(&Path::new(&path).join("hooks").join("hooks.json"), "{}");
        path
    };
    let old = hooky("forms-1.0");
    lib.add(
        "forms",
        LibraryConfig::Plugin(PluginConfig {
            path: old.clone(),
            installed_id: Some("forms@mkt".into()),
            run_hooks: true,
        }),
        None,
    )
    .await
    .unwrap();
    std::fs::remove_dir_all(&old).unwrap();
    let new = hooky("forms-1.1");
    let cache = dir.path().join("cache");
    write(
        &cache.join("installed_plugins.json"),
        &json!({"plugins": {"forms@mkt": [{"scope": "user", "installPath": new}]}}).to_string(),
    );
    std::env::set_var("CLAUDE_CODE_PLUGIN_CACHE_DIR", &cache);
    let uses = Uses {
        plugins: vec!["forms".into()],
        ..Default::default()
    };
    let plugins_dir = dir.path().join("plugins");
    let open = |run_id: &'static str| {
        let (store, bus, uses, ws, plugins_dir) = (&store, &bus, &uses, dir.path(), &plugins_dir);
        async move {
            RunTools::open(OpenRequest {
                store,
                bus,
                run_id,
                uses,
                workspace: ws,
                plugins_dir: Some(plugins_dir),
                cancel: &CancellationToken::new(),
                vault: &Vault::in_memory(store.clone()),
                scope: &DotScope::Personal,
                grants: &Grants::default(),
                dot_name: "forms",
            })
            .await
            .unwrap()
        }
    };
    let (tools, notices) = open("01RUN").await;
    assert_eq!(
        notices,
        [
            "the plugin forms moved, so its hooks were switched off; switch them on again in the \
          Library"
        ]
    );
    let copy = plugins_dir.join("run-01RUN").join("forms");
    assert!(copy.join("skills").join("fill").join("SKILL.md").is_file());
    assert!(!copy.join("hooks").exists());
    let stored = store
        .find_library_item(LibraryKind::Plugin, "forms")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.config,
        LibraryConfig::Plugin(PluginConfig {
            path: new,
            installed_id: Some("forms@mkt".into()),
            run_hooks: false,
        })
    );
    tools.close().await;

    let (tools, notices) = open("01RUN2").await;
    std::env::remove_var("CLAUDE_CODE_PLUGIN_CACHE_DIR");
    assert!(notices.is_empty(), "{notices:?}");
    tools.close().await;
}

fn collection(path: &str, every_dot: bool) -> LibraryConfig {
    LibraryConfig::Context(ContextConfig {
        path: path.into(),
        description: "How we work".into(),
        every_dot,
    })
}

#[tokio::test]
async fn a_collection_needs_a_folder_and_lists_its_document_count() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let missing = dir.path().join("nope").to_string_lossy().into_owned();
    let err = lib
        .add("company", collection(&missing, false), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
    let kb = dir.path().join("kb");
    write(
        &kb.join("style.md"),
        "# Style
Short sentences.",
    );
    write(&kb.join("people/ana.md"), "# Ana");
    write(&kb.join("logo.png"), "png");
    let path = kb.to_string_lossy().into_owned();
    let item = lib
        .add("company", collection(&path, false), None)
        .await
        .unwrap();
    let mut s = spec("nightly");
    s.uses.context = vec!["company".into()];
    store.create_dot(&s, None).await.unwrap();
    let entry = lib
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.item.id == item.id)
        .unwrap();
    assert_eq!(entry.context.unwrap().documents, 2);
    assert_eq!(entry.used_by.len(), 1);
    assert_eq!(entry.problem, None);
    // The folder goes away: the list says so.
    std::fs::remove_dir_all(&kb).unwrap();
    let entry = lib
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.item.id == item.id)
        .unwrap();
    assert_eq!(entry.context, None);
    assert_eq!(entry.problem.as_deref(), Some("folder not found"));
    // An update to a folder that does not exist is refused.
    let err = lib
        .update(&item.id, collection(&missing, true))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
}

#[tokio::test]
async fn removing_a_collection_takes_it_out_of_the_dots_that_tick_it() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let kb = dir.path().join("kb");
    std::fs::create_dir_all(&kb).unwrap();
    let item = lib
        .add("company", collection(&kb.to_string_lossy(), false), None)
        .await
        .unwrap();
    let mut s = spec("nightly");
    s.uses.context = vec!["company".into()];
    s.uses.skills = Vec::new();
    store.create_dot(&s, None).await.unwrap();
    let mut unknown = spec("audit");
    unknown.uses.context = vec!["clients".into()];
    let err = store.create_dot(&unknown, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("the Library has no context collection named \"clients\""),
        "{err}"
    );
    assert!(matches!(
        lib.remove(&item.id, false).await,
        Err(Error::Conflict(_))
    ));
    lib.remove(&item.id, true).await.unwrap();
    let dots = store.list_dots().await.unwrap();
    assert!(dots[0].spec.uses.context.is_empty());
}

#[tokio::test]
async fn the_documents_of_a_collection_are_described_up_to_200() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let lib = service(&store, &bus);
    let kb = dir.path().join("kb");
    for i in 0..205 {
        write(
            &kb.join(format!("d{i:03}.md")),
            &format!(
                "---
title: Doc {i}
---
About {i}."
            ),
        );
    }
    let item = lib
        .add("company", collection(&kb.to_string_lossy(), false), None)
        .await
        .unwrap();
    let docs = lib.context_documents(&item.id).await.unwrap();
    assert_eq!(docs.documents.len(), 200);
    assert_eq!(docs.total, 205);
    assert!(docs.truncated);
    assert_eq!(docs.documents[0].path, "d000.md");
    assert_eq!(docs.documents[0].title, "Doc 0");
    assert_eq!(docs.documents[0].summary, "About 0.");
    let skill_path = skill_dir(dir.path(), "pdf", "Reads PDFs.");
    let skill = lib
        .add(
            "pdf",
            LibraryConfig::Skill(SkillConfig { path: skill_path }),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        lib.context_documents(&skill.id).await,
        Err(Error::Invalid(_))
    ));
    std::fs::remove_dir_all(&kb).unwrap();
    assert!(matches!(
        lib.context_documents(&item.id).await,
        Err(Error::NotFound(_))
    ));
}

//! Skills and plugins on disk: the frontmatter reader, skill and plugin reading, the installed
//! plugins (MCP/skills spec sections 2 and 7).

use std::fs;
use std::path::Path;

use dots_core::kit::{
    find_installed, installed_plugins, parse_frontmatter, plugins_root, read_installed,
    read_plugin, read_skill, remove_field, scan_skills, LISTING_MAX_CHARS,
};
use dots_core::library::McpServerConfig;
use dots_core::Error;
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn field(text: &str, key: &str) -> Option<String> {
    parse_frontmatter(text).fields.get(key).cloned()
}

// ---- frontmatter ----

#[test]
fn plain_and_quoted_values_are_read() {
    let text = "---\nname: pdf\ndescription: \"Extract: text, \\\"tables\\\"\"\nwhen_to_use: 'it''s for PDFs'\nmodel: sonnet # a comment\n---\n# Body\n";
    let fm = parse_frontmatter(text);
    assert_eq!(fm.fields["name"], "pdf");
    assert_eq!(fm.fields["description"], "Extract: text, \"tables\"");
    assert_eq!(fm.fields["when_to_use"], "it's for PDFs");
    assert_eq!(fm.fields["model"], "sonnet");
    assert_eq!(fm.body, "# Body");
}

#[test]
fn block_scalars_fold_or_keep_their_lines() {
    let text = "---\ndescription: >\n  First line\n  second line\n\n  new paragraph\nwhen_to_use: |-\n  keep\n  lines\nname: x\n---\n";
    assert_eq!(
        field(text, "description").unwrap(),
        "First line second line\nnew paragraph"
    );
    assert_eq!(field(text, "when_to_use").unwrap(), "keep\nlines");
    assert_eq!(field(text, "name").unwrap(), "x");
}

#[test]
fn a_plain_value_goes_on_over_indented_lines() {
    let text = "---\ndescription: one\n  two\n  three\nname: x\n---\n";
    assert_eq!(field(text, "description").unwrap(), "one two three");
}

#[test]
fn lists_and_maps_are_ignored_and_the_rest_is_kept() {
    let text = "---\nname: pdf\nallowed-tools:\n  - Bash(git *)\n  - Read\ntools: [a, b,\n  c]\nmeta:\n  nested: yes\nlist2:\n- a\n- b\ndescription: after\n---\n";
    let fm = parse_frontmatter(text);
    assert_eq!(fm.fields.len(), 2, "{:?}", fm.fields);
    assert_eq!(fm.fields["name"], "pdf");
    assert_eq!(fm.fields["description"], "after");
}

#[test]
fn allowed_tools_as_a_string_is_a_field() {
    let text = "---\nallowed-tools: Bash(git *) Read\n---\n";
    assert_eq!(field(text, "allowed-tools").unwrap(), "Bash(git *) Read");
}

#[test]
fn a_broken_frontmatter_gives_no_fields() {
    for text in [
        // `: ` inside a plain value
        "---\nname: pdf\ndescription: Use when: you read a PDF\n---\nbody\n",
        // the same key twice
        "---\nname: a\nname: b\n---\n",
        // an unclosed quote
        "---\nname: \"pdf\ndescription: x\n---\n",
        // a line that is no key
        "---\nname: pdf\njust text\n---\n",
        // a list at the top
        "---\n- a\n- b\n---\n",
    ] {
        let fm = parse_frontmatter(text);
        assert!(fm.fields.is_empty(), "{text:?} gave {:?}", fm.fields);
    }
}

#[test]
fn a_file_without_a_closed_frontmatter_has_none() {
    let fm = parse_frontmatter("# Title\nname: not-a-field\n");
    assert!(fm.fields.is_empty());
    assert_eq!(fm.body, "# Title\nname: not-a-field\n");
    let open = parse_frontmatter("---\nname: x\nno end\n");
    assert!(open.fields.is_empty());
}

#[test]
fn crlf_and_a_byte_order_mark_are_fine() {
    let fm = parse_frontmatter("\u{feff}---\r\nname: pdf\r\ndescription: d\r\n---\r\nbody\r\n");
    assert_eq!(fm.fields["name"], "pdf");
    assert_eq!(fm.fields["description"], "d");
}

#[test]
fn allowed_tools_is_removed_in_every_form() {
    let scalar = "---\nname: a\nallowed-tools: Bash(rm *)\ndescription: d\n---\nbody\n";
    assert_eq!(
        remove_field(scalar, "allowed-tools"),
        "---\nname: a\ndescription: d\n---\nbody\n"
    );
    let list =
        "---\nname: a\nallowed-tools:\n  - Bash\n  - Read\n\n- Write\ndescription: d\n---\nbody\n";
    assert_eq!(
        remove_field(list, "allowed-tools"),
        "---\nname: a\ndescription: d\n---\nbody\n"
    );
    let block = "---\nallowed-tools: >\n  Bash\n  Read\n---\nbody allowed-tools: x\n";
    assert_eq!(
        remove_field(block, "allowed-tools"),
        "---\n---\nbody allowed-tools: x\n"
    );
    let quoted = "---\r\n\"allowed-tools\": Bash\r\nname: a\r\n---\r\n";
    assert_eq!(
        remove_field(quoted, "allowed-tools"),
        "---\r\nname: a\r\n---\r\n"
    );
}

#[test]
fn allowed_tools_is_removed_from_a_frontmatter_dots_reads_as_broken() {
    // Claude Code's YAML may accept what the hand reader refuses; the line-based removal still
    // takes the field out.
    let text = "---\ndescription: Use when: x\nallowed-tools: Bash\n---\nbody\n";
    assert!(parse_frontmatter(text).fields.is_empty());
    assert_eq!(
        remove_field(text, "allowed-tools"),
        "---\ndescription: Use when: x\n---\nbody\n"
    );
}

#[test]
fn removal_leaves_text_without_a_closed_frontmatter_alone() {
    for text in [
        "allowed-tools: Bash\nbody\n",
        "---\nallowed-tools: Bash\nno end\n",
        "",
    ] {
        assert_eq!(remove_field(text, "allowed-tools"), text);
    }
}

// ---- skills ----

#[test]
fn a_skill_takes_its_name_from_the_folder_and_its_description_from_the_body() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("pdf");
    write(&skill.join("SKILL.md"), "\n# Reading PDFs\n\nMore.\n");
    let info = read_skill(&skill).unwrap();
    assert_eq!(info.name, "pdf");
    assert_eq!(info.description, "# Reading PDFs");
    assert_eq!(info.when_to_use, None);
    assert_eq!(info.dir, skill);
}

#[test]
fn a_skill_with_broken_frontmatter_still_loads_with_defaults() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("pdf");
    write(
        &skill.join("SKILL.md"),
        "---\nname: other\ndescription: Use when: x\n---\nFirst body line\n",
    );
    let info = read_skill(&skill).unwrap();
    assert_eq!(info.name, "pdf");
    assert_eq!(info.description, "First body line");
    assert!(info.fields.is_empty());
}

#[test]
fn a_skill_reads_its_fields() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("folder");
    write(
        &skill.join("SKILL.md"),
        "---\nname: pdf\ndescription: Extract text\nwhen_to_use: When a PDF is attached\nallowed-tools: Bash\n---\nbody\n",
    );
    let info = read_skill(&skill).unwrap();
    assert_eq!(info.name, "pdf");
    assert_eq!(info.when_to_use.as_deref(), Some("When a PDF is attached"));
    assert_eq!(info.fields["allowed-tools"], "Bash");
    assert_eq!(info.listing(), "Extract text When a PDF is attached");
}

#[test]
fn the_listing_is_one_line_and_cut_at_the_limit() {
    let dir = tempdir().unwrap();
    let skill = dir.path().join("s");
    let long = "é".repeat(2000);
    write(
        &skill.join("SKILL.md"),
        &format!("---\ndescription: >\n  a\n  b\nwhen_to_use: {long}\n---\n"),
    );
    let listing = read_skill(&skill).unwrap().listing();
    assert!(listing.starts_with("a b é"));
    assert_eq!(listing.chars().count(), LISTING_MAX_CHARS);
}

#[test]
fn a_missing_folder_or_skill_file_is_not_found() {
    let dir = tempdir().unwrap();
    let missing = read_skill(&dir.path().join("nope")).unwrap_err();
    assert!(matches!(&missing, Error::NotFound(m) if m.contains("folder not found")));
    fs::create_dir_all(dir.path().join("empty")).unwrap();
    let no_file = read_skill(&dir.path().join("empty")).unwrap_err();
    assert!(matches!(&no_file, Error::NotFound(m) if m.contains("SKILL.md not found")));
}

#[test]
fn a_scan_finds_the_skills_under_a_folder() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(&root.join("b").join("SKILL.md"), "---\nname: bee\n---\n");
    write(&root.join("a").join("SKILL.md"), "A skill\n");
    write(&root.join(".hidden").join("SKILL.md"), "x\n");
    write(&root.join("c").join("README.md"), "no skill\n");
    let names: Vec<_> = scan_skills(root)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["a", "bee"]);
}

#[test]
fn a_scan_of_a_skill_folder_gives_that_skill() {
    let dir = tempdir().unwrap();
    write(&dir.path().join("SKILL.md"), "One\n");
    write(&dir.path().join("sub").join("SKILL.md"), "Two\n");
    assert_eq!(scan_skills(dir.path()).unwrap().len(), 1);
}

#[test]
fn a_scan_falls_back_to_a_skills_folder_and_reports_a_missing_one() {
    let dir = tempdir().unwrap();
    write(&dir.path().join("skills").join("x").join("SKILL.md"), "X\n");
    let found = scan_skills(dir.path()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "x");
    assert!(matches!(
        scan_skills(&dir.path().join("nope")),
        Err(Error::NotFound(_))
    ));
}

// ---- plugins ----

#[test]
fn a_plugin_without_a_manifest_is_named_after_its_folder() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("pdf-tools");
    write(
        &root.join("skills").join("fill").join("SKILL.md"),
        "Fill forms\n",
    );
    write(
        &root.join("skills").join("empty").join("notes.txt"),
        "no skill\n",
    );
    write(&root.join("commands").join("run.md"), "Run\n");
    write(&root.join("agents").join("helper.md"), "Help\n");
    write(&root.join("hooks").join("hooks.json"), "{}");
    fs::create_dir_all(root.join("monitors")).unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    let info = read_plugin(&root).unwrap();
    assert_eq!(info.name, "pdf-tools");
    assert_eq!(info.skills.len(), 1);
    assert_eq!(info.skills[0].name, "fill");
    assert!(!info.single_skill);
    assert_eq!(info.commands, ["run"]);
    assert_eq!(info.agents, ["helper"]);
    assert!(info.has_hooks && info.has_monitors && info.has_bin);
    assert!(info.servers.is_empty() && info.warnings.is_empty());
}

#[test]
fn the_manifest_gives_the_name_and_a_bad_one_a_warning() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("folder");
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"pdf-tools","version":"1.2.0","description":"PDF helpers"}"#,
    );
    let info = read_plugin(&root).unwrap();
    assert_eq!(info.name, "pdf-tools");
    assert_eq!(info.version.as_deref(), Some("1.2.0"));
    assert_eq!(info.description.as_deref(), Some("PDF helpers"));

    write(
        &root.join(".claude-plugin").join("plugin.json"),
        "{not json",
    );
    let broken = read_plugin(&root).unwrap();
    assert_eq!(broken.name, "folder");
    assert_eq!(broken.warnings.len(), 1);
    assert!(broken.warnings[0].contains("plugin.json"));
}

#[test]
fn a_root_skill_without_a_skills_folder_is_a_single_skill_plugin() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("solo");
    write(&root.join("SKILL.md"), "---\nname: solo-skill\n---\nbody\n");
    let info = read_plugin(&root).unwrap();
    assert!(info.single_skill);
    assert_eq!(info.skills[0].name, "solo-skill");

    fs::create_dir_all(root.join("skills")).unwrap();
    let with_folder = read_plugin(&root).unwrap();
    assert!(!with_folder.single_skill);
    assert!(with_folder.skills.is_empty());
}

#[test]
fn mcp_json_servers_are_read_with_the_plugin_root_filled_in() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(
        &root.join(".mcp.json"),
        r#"{"mcpServers":{
            "db": {"command":"${CLAUDE_PLUGIN_ROOT}/bin/db","args":["--root","${CLAUDE_PLUGIN_ROOT}"],
                   "env":{"TOKEN":"${DB_TOKEN}"}},
            "api": {"type":"http","url":"https://example.com/mcp","headers":{"X":"1"}},
            "old": {"type":"sse","url":"https://example.com/sse"},
            "odd": {"type":"carrier-pigeon","url":"x"},
            "empty": {}
        }}"#,
    );
    let info = read_plugin(&root).unwrap();
    let root_text = root.to_string_lossy().into_owned();
    let names: Vec<_> = info.servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["api", "db", "old"]);
    let db = &info.servers[1].config;
    match db {
        McpServerConfig::Stdio {
            command, args, env, ..
        } => {
            assert_eq!(command, &format!("{root_text}/bin/db"));
            assert_eq!(args, &vec!["--root".to_string(), root_text.clone()]);
            // Only the plugin root is replaced; the other variable stays for the server's start.
            assert_eq!(env["TOKEN"], "${DB_TOKEN}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&info.servers[2].config, McpServerConfig::Http { url, .. } if url == "https://example.com/sse")
    );
    assert_eq!(info.warnings.len(), 2, "{:?}", info.warnings);
    assert!(info
        .warnings
        .iter()
        .any(|w| w.contains("odd") && w.contains("carrier-pigeon")));
    assert!(info.warnings.iter().any(|w| w.contains("empty")));
}

#[test]
fn mcp_json_may_be_the_servers_object_itself() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root.join(".mcp.json"), r#"{"db":{"command":"db-server"}}"#);
    let info = read_plugin(&root).unwrap();
    assert_eq!(info.servers.len(), 1);
    assert_eq!(info.servers[0].name, "db");
}

#[test]
fn servers_in_the_manifest_are_read_after_mcp_json() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root.join(".mcp.json"), r#"{"a":{"command":"from-file"}}"#);
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"mcpServers":{"a":{"command":"from-manifest"},"b":{"command":"b"}}}"#,
    );
    let info = read_plugin(&root).unwrap();
    let a = info.servers.iter().find(|s| s.name == "a").unwrap();
    assert!(matches!(&a.config, McpServerConfig::Stdio { command, .. } if command == "from-file"));
    assert_eq!(info.servers.len(), 2);
}

#[test]
fn a_broken_mcp_json_is_a_warning_and_no_servers() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(&root.join(".mcp.json"), "[1,2");
    let info = read_plugin(&root).unwrap();
    assert!(info.servers.is_empty());
    assert!(info.warnings[0].contains(".mcp.json"));
}

#[test]
fn server_items_get_the_plugin_prefix() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("p");
    write(
        &root.join(".mcp.json"),
        r#"{"My Server":{"command":"a"},"my_server":{"command":"a2"},"my-server":{"command":"b"},"db":{"command":"c"}}"#,
    );
    let items = read_plugin(&root).unwrap().server_items("pdf-tools");
    let names: Vec<_> = items.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        // `My Server` and `my_server` come out the same: the first (by key) is kept.
        [
            "plugin_pdf-tools_my_server",
            "plugin_pdf-tools_db",
            "plugin_pdf-tools_my-server"
        ]
    );
    for (name, _) in &items {
        assert!(
            dots_core::library::check_name(dots_core::library::LibraryKind::McpServer, name)
                .is_ok()
        );
    }
}

#[test]
fn a_plugin_folder_that_is_missing_is_not_found() {
    let dir = tempdir().unwrap();
    assert!(matches!(
        read_plugin(&dir.path().join("nope")),
        Err(Error::NotFound(_))
    ));
}

// ---- installed plugins ----

fn env_of<'a>(vars: &'a [(&'a str, String)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone())
}

#[test]
fn installed_plugins_are_read_from_a_map_of_record_lists() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("cache").join("a");
    fs::create_dir_all(&a).unwrap();
    write(
        &dir.path().join("installed_plugins.json"),
        &format!(
            r#"{{"version":2,"plugins":{{
                "zeta@mkt":[{{"scope":"project","installPath":"{p}/gone","version":"0.1"}},
                            {{"scope":"user","installPath":"{a}","version":"1.2.0"}}],
                "alpha@other-mkt":{{"scope":"user","installPath":"{p}/alpha"}},
                "broken@mkt":{{"scope":"user"}},
                "number@mkt":5
            }}}}"#,
            p = dir.path().to_string_lossy().replace('\\', "/"),
            a = a.to_string_lossy().replace('\\', "/"),
        ),
    );
    let found = read_installed(dir.path());
    let ids: Vec<_> = found.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["alpha@other-mkt", "zeta@mkt"]);
    assert_eq!(found[1].name, "zeta");
    assert_eq!(found[1].marketplace.as_deref(), Some("mkt"));
    assert_eq!(found[1].scope.as_deref(), Some("user"));
    assert_eq!(found[1].version.as_deref(), Some("1.2.0"));
    assert!(found[1].exists);
    assert!(!found[0].exists);
}

#[test]
fn installed_plugins_tolerate_a_top_level_map_and_junk() {
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("installed_plugins.json"),
        r#"{"version":1,"x@m":{"installPath":"C:/nowhere"}}"#,
    );
    let found = read_installed(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "x@m");

    for junk in ["", "not json", "[1,2]", "null", r#"{"plugins":[1]}"#] {
        write(&dir.path().join("installed_plugins.json"), junk);
        assert!(read_installed(dir.path()).is_empty(), "{junk:?}");
    }
    fs::remove_file(dir.path().join("installed_plugins.json")).unwrap();
    assert!(read_installed(dir.path()).is_empty());
}

#[test]
fn the_cache_dir_variable_moves_the_root() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    write(
        &dir.path().join("installed_plugins.json"),
        r#"{"plugins":{"p@m":{"installPath":"C:/x"}}}"#,
    );
    let vars = [
        ("CLAUDE_CODE_PLUGIN_CACHE_DIR", root.clone()),
        ("USERPROFILE", "C:/home".into()),
    ];
    let env = env_of(&vars);
    assert_eq!(plugins_root(&env).unwrap(), Path::new(&root));
    assert_eq!(installed_plugins(&env).len(), 1);
    assert!(find_installed(&env, "p@m").is_some());
    assert!(find_installed(&env, "q@m").is_none());

    let home = [("USERPROFILE", "C:/home".to_string())];
    assert_eq!(
        plugins_root(&env_of(&home)).unwrap(),
        Path::new("C:/home").join(".claude").join("plugins")
    );
    let blank = [
        ("CLAUDE_CODE_PLUGIN_CACHE_DIR", "  ".to_string()),
        ("HOME", "/h".into()),
    ];
    assert_eq!(
        plugins_root(&env_of(&blank)).unwrap(),
        Path::new("/h").join(".claude").join("plugins")
    );
    assert!(plugins_root(&env_of(&[])).is_none());
    assert!(installed_plugins(&env_of(&[])).is_empty());
}

#[test]
fn a_plugins_skills_are_named_after_its_library_item() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("folder");
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"manifest-name"}"#,
    );
    write(
        &root.join("skills").join("fill").join("SKILL.md"),
        "Fill
",
    );
    write(
        &root.join("skills").join("x").join("SKILL.md"),
        "---
name: extract
---
",
    );
    let info = read_plugin(&root).unwrap();
    let names: Vec<String> = info
        .qualified_skills("docs")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["docs:fill", "docs:extract"]);
}

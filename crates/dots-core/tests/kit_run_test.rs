//! The per-run plugin folders (MCP/skills spec section 7).

use std::fs;
use std::path::{Path, PathBuf};

use dots_core::kit::{prepare_run_kit, KitPlugin, KitSkill, MAX_COPY_BYTES};
use dots_core::library::LibraryKind;
use serde_json::Value;
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn skill_in(dir: &Path, name: &str) -> KitSkill {
    let path = dir.join("src-skills").join(name);
    write(
        &path.join("SKILL.md"),
        "---\nname: pdf\nallowed-tools: Bash(rm *)\ndescription: d\n---\nbody\n",
    );
    write(&path.join("scripts").join("run.py"), "print(1)\n");
    KitSkill {
        name: name.into(),
        dir: path,
    }
}

fn plugin_in(dir: &Path, name: &str, run_hooks: bool) -> KitPlugin {
    let root = dir.join("src-plugins").join(name);
    write(
        &root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"pdf-tools","version":"1","mcpServers":{"x":{"command":"x"}},"hooks":{"PreToolUse":[]},"monitors":[]}"#,
    );
    write(&root.join(".mcp.json"), r#"{"db":{"command":"db"}}"#);
    write(&root.join("hooks").join("hooks.json"), "{}");
    write(&root.join("monitors").join("m.json"), "{}");
    write(
        &root.join("skills").join("fill").join("SKILL.md"),
        "---\nallowed-tools:\n  - Bash\ndescription: fill\n---\nfill\n",
    );
    write(
        &root.join("commands").join("go.md"),
        "---\nallowed-tools: Bash(curl *)\n---\nGo\n",
    );
    write(
        &root.join("commands").join("sub").join("deep.md"),
        "---\nallowed-tools: Read\n---\nDeep\n",
    );
    write(
        &root.join("agents").join("helper.md"),
        "---\ntools: Read\n---\nHelp\n",
    );
    write(&root.join("bin").join("tool.cmd"), "echo\n");
    write(&root.join(".git").join("config"), "x");
    KitPlugin {
        name: name.into(),
        dir: root,
        run_hooks,
    }
}

#[test]
fn nothing_to_include_creates_nothing() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let kit = prepare_run_kit(&plugins, "r1", &[], &[]).unwrap();
    assert!(kit.plugin_dirs.is_empty() && kit.left_out.is_empty());
    assert!(!plugins.exists());
    kit.cleanup();
}

#[test]
fn skills_go_into_a_generated_dots_skills_plugin_without_allowed_tools() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let skill = skill_in(tmp.path(), "pdf");
    let kit = prepare_run_kit(&plugins, "r1", &[skill], &[]).unwrap();
    let dir = plugins.join("run-r1").join("dots-skills");
    assert_eq!(kit.plugin_dirs, std::slice::from_ref(&dir));
    let manifest: Value =
        serde_json::from_str(&read(&dir.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert_eq!(manifest["name"], "dots-skills");
    let copy = dir.join("skills").join("pdf");
    assert_eq!(
        read(&copy.join("SKILL.md")),
        "---\nname: pdf\ndescription: d\n---\nbody\n"
    );
    assert_eq!(read(&copy.join("scripts").join("run.py")), "print(1)\n");
    // The original is untouched.
    assert!(
        read(&tmp.path().join("src-skills").join("pdf").join("SKILL.md")).contains("allowed-tools")
    );
}

#[test]
fn a_plugin_copy_has_no_servers_monitors_or_hooks_and_no_allowed_tools() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "pdf-tools", false);
    let kit = prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let copy = plugins.join("run-r1").join("pdf-tools");
    assert_eq!(kit.plugin_dirs, std::slice::from_ref(&copy));
    assert!(!copy.join(".mcp.json").exists());
    assert!(!copy.join("monitors").exists());
    assert!(!copy.join("hooks").exists());
    assert!(!copy.join(".git").exists());
    assert!(copy.join("bin").join("tool.cmd").exists());
    assert!(copy.join("agents").join("helper.md").exists());
    assert_eq!(
        read(&copy.join("skills").join("fill").join("SKILL.md")),
        "---\ndescription: fill\n---\nfill\n"
    );
    assert_eq!(read(&copy.join("commands").join("go.md")), "---\n---\nGo\n");
    assert_eq!(
        read(&copy.join("commands").join("sub").join("deep.md")),
        "---\n---\nDeep\n"
    );
    let manifest: Value =
        serde_json::from_str(&read(&copy.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert_eq!(manifest["name"], "pdf-tools");
    assert_eq!(manifest["version"], "1");
    for gone in ["mcpServers", "hooks", "monitors"] {
        assert!(manifest.get(gone).is_none(), "{gone}");
    }
}

#[test]
fn run_hooks_keeps_the_hooks_and_nothing_else_changes() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "pdf-tools", true);
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let copy = plugins.join("run-r1").join("pdf-tools");
    assert!(copy.join("hooks").join("hooks.json").exists());
    assert!(!copy.join(".mcp.json").exists());
    assert!(!copy.join("monitors").exists());
    let manifest: Value =
        serde_json::from_str(&read(&copy.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert!(manifest.get("hooks").is_some());
    assert!(manifest.get("mcpServers").is_none() && manifest.get("monitors").is_none());
}

#[test]
fn a_nested_mcp_json_or_hooks_folder_is_not_special() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "p", false);
    write(
        &plugin.dir.join("skills").join("fill").join(".mcp.json"),
        "{}",
    );
    write(
        &plugin
            .dir
            .join("skills")
            .join("fill")
            .join("hooks")
            .join("h.txt"),
        "h",
    );
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let fill = plugins.join("run-r1").join("p").join("skills").join("fill");
    assert!(fill.join(".mcp.json").exists());
    assert!(fill.join("hooks").join("h.txt").exists());
}

#[test]
fn dots_skills_comes_first_and_plugins_keep_their_order() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let kit = prepare_run_kit(
        &plugins,
        "r1",
        &[skill_in(tmp.path(), "pdf")],
        &[
            plugin_in(tmp.path(), "zeta", false),
            plugin_in(tmp.path(), "alpha", false),
        ],
    )
    .unwrap();
    let names: Vec<_> = kit
        .plugin_dirs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["dots-skills", "zeta", "alpha"]);
}

#[test]
fn what_is_missing_is_left_out_with_a_reason_and_the_rest_goes_in() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    fs::create_dir_all(tmp.path().join("no-skill-file")).unwrap();
    let skills = [
        KitSkill {
            name: "gone".into(),
            dir: tmp.path().join("nope"),
        },
        KitSkill {
            name: "bare".into(),
            dir: tmp.path().join("no-skill-file"),
        },
        skill_in(tmp.path(), "pdf"),
    ];
    let plugin_list = [
        KitPlugin {
            name: "lost".into(),
            dir: tmp.path().join("nope"),
            run_hooks: false,
        },
        plugin_in(tmp.path(), "ok", false),
    ];
    let kit = prepare_run_kit(&plugins, "r1", &skills, &plugin_list).unwrap();
    let left: Vec<_> = kit
        .left_out
        .iter()
        .map(|l| (l.kind, l.name.as_str(), l.reason.as_str()))
        .collect();
    assert_eq!(
        left,
        [
            (LibraryKind::Skill, "gone", "folder not found"),
            (LibraryKind::Skill, "bare", "SKILL.md not found"),
            (LibraryKind::Plugin, "lost", "folder not found"),
        ]
    );
    assert_eq!(kit.plugin_dirs.len(), 2);
    assert!(!plugins.join("run-r1").join("lost").exists());
}

#[test]
fn only_skills_that_are_left_out_make_no_dots_skills_plugin() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let skills = [KitSkill {
        name: "gone".into(),
        dir: tmp.path().join("nope"),
    }];
    let kit = prepare_run_kit(&plugins, "r1", &skills, &[]).unwrap();
    assert!(kit.plugin_dirs.is_empty());
    assert!(!plugins.join("run-r1").join("dots-skills").exists());
    kit.cleanup();
    assert!(!plugins.join("run-r1").exists());
}

#[test]
fn a_copy_over_the_cap_is_left_out_whole() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let big = plugin_in(tmp.path(), "big", false);
    let file = fs::File::create(big.dir.join("blob.bin")).unwrap();
    file.set_len(MAX_COPY_BYTES + 1).unwrap();
    drop(file);
    // Excluded parts do not count: the same blob under `monitors/` is fine.
    let fine = plugin_in(tmp.path(), "fine", false);
    let file = fs::File::create(fine.dir.join("monitors").join("blob.bin")).unwrap();
    file.set_len(MAX_COPY_BYTES + 1).unwrap();
    drop(file);
    let kit = prepare_run_kit(&plugins, "r1", &[], &[big, fine]).unwrap();
    assert_eq!(kit.left_out.len(), 1);
    assert_eq!(kit.left_out[0].name, "big");
    assert_eq!(kit.left_out[0].reason, "larger than 50 MB");
    assert!(!plugins.join("run-r1").join("big").exists());
    assert_eq!(kit.plugin_dirs.len(), 1);
}

#[test]
fn cleanup_deletes_the_run_folder_and_only_that() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let other = prepare_run_kit(&plugins, "r2", &[skill_in(tmp.path(), "a")], &[]).unwrap();
    let kit = prepare_run_kit(&plugins, "r1", &[skill_in(tmp.path(), "b")], &[]).unwrap();
    kit.cleanup();
    kit.cleanup();
    assert!(!plugins.join("run-r1").exists());
    assert!(plugins.join("run-r2").exists());
    other.cleanup();
}

#[test]
fn an_old_folder_of_the_same_run_is_replaced() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    write(&plugins.join("run-r1").join("stale").join("x"), "x");
    prepare_run_kit(&plugins, "r1", &[skill_in(tmp.path(), "a")], &[]).unwrap();
    assert!(!plugins.join("run-r1").join("stale").exists());
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_not_followed() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let plugin = plugin_in(tmp.path(), "p", false);
    let secret = tmp.path().join("secret.txt");
    write(&secret, "secret");
    std::os::unix::fs::symlink(&secret, plugin.dir.join("link.txt")).unwrap();
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    assert!(!plugins.join("run-r1").join("p").join("link.txt").exists());
}

fn hooky_plugin(dir: &Path, name: &str, run_hooks: bool) -> KitPlugin {
    let plugin = plugin_in(dir, name, run_hooks);
    let hooks = "hooks:\n  PreToolUse:\n    - command: evil\n";
    write(
        &plugin.dir.join("skills").join("fill").join("SKILL.md"),
        &format!("---\ndescription: fill\n{hooks}---\nfill\n"),
    );
    write(
        &plugin.dir.join("commands").join("go.md"),
        &format!("---\ndescription: go\n{hooks}---\nGo\n"),
    );
    write(
        &plugin.dir.join("agents").join("helper.md"),
        &format!(
            "---\nname: helper\ntools: Read\npermissionMode: bypassPermissions\nmcpServers:\n  x:\n    command: x\n{hooks}---\nHelp\n"
        ),
    );
    plugin
}

#[test]
fn frontmatter_hooks_are_removed_from_a_plugin_unless_it_runs_hooks() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    prepare_run_kit(
        &plugins,
        "off",
        &[],
        &[hooky_plugin(tmp.path(), "p", false)],
    )
    .unwrap();
    prepare_run_kit(&plugins, "on", &[], &[hooky_plugin(tmp.path(), "p", true)]).unwrap();
    let files = [
        ["skills", "fill", "SKILL.md"],
        ["commands", "go.md", ""],
        ["agents", "helper.md", ""],
    ];
    for file in files {
        let rel: std::path::PathBuf = file.iter().filter(|p| !p.is_empty()).collect();
        let off = read(&plugins.join("run-off").join("p").join(&rel));
        let on = read(&plugins.join("run-on").join("p").join(&rel));
        assert!(
            !off.contains("hooks:") && !off.contains("evil"),
            "{rel:?}: {off}"
        );
        assert!(
            on.contains("hooks:") && on.contains("evil"),
            "{rel:?}: {on}"
        );
    }
}

#[test]
fn agents_lose_permission_mode_and_mcp_servers_even_with_hooks_on() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    prepare_run_kit(&plugins, "on", &[], &[hooky_plugin(tmp.path(), "p", true)]).unwrap();
    let agent = read(
        &plugins
            .join("run-on")
            .join("p")
            .join("agents")
            .join("helper.md"),
    );
    assert!(
        !agent.contains("permissionMode") && !agent.contains("mcpServers"),
        "{agent}"
    );
    assert!(
        agent.contains("name: helper") && agent.contains("tools: Read"),
        "{agent}"
    );
    assert!(agent.ends_with("---\nHelp\n"));
}

#[test]
fn a_standalone_skill_never_keeps_hooks() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    let skill = skill_in(tmp.path(), "pdf");
    write(
        &skill.dir.join("SKILL.md"),
        "---\ndescription: d\nhooks:\n  Stop:\n    - command: evil\n---\nbody\n",
    );
    prepare_run_kit(&plugins, "r1", &[skill], &[]).unwrap();
    let copy = read(
        &plugins
            .join("run-r1")
            .join("dots-skills")
            .join("skills")
            .join("pdf")
            .join("SKILL.md"),
    );
    assert_eq!(copy, "---\ndescription: d\n---\nbody\n");
}

#[test]
fn the_copied_manifest_is_named_after_the_library_item() {
    let tmp = tempdir().unwrap();
    let plugins = tmp.path().join("plugins");
    // The manifest says pdf-tools; the Library item is called docs.
    let named = plugin_in(tmp.path(), "docs", false);
    // No manifest at all.
    let bare = KitPlugin {
        name: "bare".into(),
        dir: tmp.path().join("src-plugins").join("bare-src"),
        run_hooks: false,
    };
    write(&bare.dir.join("skills").join("s").join("SKILL.md"), "s\n");
    // A manifest that is not JSON.
    let broken = KitPlugin {
        name: "broken".into(),
        dir: tmp.path().join("src-plugins").join("broken-src"),
        run_hooks: false,
    };
    write(
        &broken.dir.join(".claude-plugin").join("plugin.json"),
        "{nope",
    );
    prepare_run_kit(&plugins, "r1", &[], &[named, bare, broken]).unwrap();
    for name in ["docs", "bare", "broken"] {
        let path = plugins
            .join("run-r1")
            .join(name)
            .join(".claude-plugin")
            .join("plugin.json");
        let manifest: Value = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(manifest["name"], name);
    }
    let docs = plugins
        .join("run-r1")
        .join("docs")
        .join(".claude-plugin")
        .join("plugin.json");
    assert_eq!(
        serde_json::from_str::<Value>(&read(&docs)).unwrap()["version"],
        "1"
    );
}

fn copy_of(name: &str, build: impl Fn(&Path), run_hooks: bool) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("src").join(name);
    fs::create_dir_all(&root).unwrap();
    build(&root);
    let plugin = KitPlugin {
        name: name.into(),
        dir: root,
        run_hooks,
    };
    let plugins = tmp.path().join("plugins");
    prepare_run_kit(&plugins, "r1", &[], &[plugin]).unwrap();
    let copy = plugins.join("run-r1").join(name);
    (tmp, copy)
}

#[test]
fn names_are_matched_without_regard_to_case() {
    let (_tmp, copy) = copy_of(
        "p",
        |root| {
            write(&root.join("Hooks").join("hooks.json"), "{}");
            write(&root.join(".MCP.json"), "{}");
            write(&root.join("Monitors").join("x"), "x");
            write(
                &root.join("Commands").join("go.md"),
                "---\nallowed-tools: Bash(curl *)\n---\nGo\n",
            );
            write(
                &root.join("skills").join("a").join("Skill.md"),
                "---\nallowed-tools: Bash\n---\nA\n",
            );
        },
        false,
    );
    assert!(!copy.join("Hooks").exists());
    assert!(!copy.join(".MCP.json").exists());
    assert!(!copy.join("Monitors").exists());
    assert_eq!(read(&copy.join("Commands").join("go.md")), "---\n---\nGo\n");
    assert_eq!(
        read(&copy.join("skills").join("a").join("Skill.md")),
        "---\n---\nA\n"
    );
}

#[test]
fn markdown_in_folders_named_by_the_manifest_is_cleaned_too() {
    let (_tmp, copy) = copy_of(
        "p",
        |root| {
            write(
                &root.join(".claude-plugin").join("plugin.json"),
                r#"{"commands":"./extra","agents":["./more"]}"#,
            );
            write(
                &root.join("extra").join("go.MD"),
                "---\nallowed-tools: Bash(curl *)\n---\nGo\n",
            );
            write(
                &root.join("more").join("a.md"),
                "---\nname: a\npermissionMode: bypassPermissions\nmcpServers:\n  x:\n    command: x\n---\nA\n",
            );
        },
        false,
    );
    assert_eq!(read(&copy.join("extra").join("go.MD")), "---\n---\nGo\n");
    assert_eq!(
        read(&copy.join("more").join("a.md")),
        "---\nname: a\n---\nA\n"
    );
}

#[test]
fn a_frontmatter_in_a_form_dots_does_not_read_is_dropped() {
    let forms = [
        "---\n{allowed-tools: \"Bash(*)\", description: x}\n---\nbody\n",
        "---\n  allowed-tools: Bash(*)\n  description: x\n---\nbody\n",
        "---\n\"allowed\\x2dtools\": Bash(*)\ndescription: x\n---\nbody\n",
        "---\n? allowed-tools\n: Bash(*)\n---\nbody\n",
    ];
    let (_tmp, copy) = copy_of(
        "p",
        |root| {
            for (i, form) in forms.iter().enumerate() {
                write(&root.join("commands").join(format!("c{i}.md")), form);
            }
        },
        false,
    );
    for i in 0..forms.len() {
        assert_eq!(
            read(&copy.join("commands").join(format!("c{i}.md"))),
            "---\n---\nbody\n",
            "form {i}"
        );
    }
}

#[test]
fn lsp_servers_are_left_out() {
    let (_tmp, copy) = copy_of(
        "p",
        |root| {
            write(&root.join(".LSP.json"), "{}");
            write(
                &root.join(".claude-plugin").join("plugin.json"),
                r#"{"name":"p","lspServers":{"x":{"command":"x"}}}"#,
            );
            write(&root.join("sub").join(".lsp.json"), "{}");
        },
        true,
    );
    assert!(!copy.join(".LSP.json").exists());
    assert!(copy.join("sub").join(".lsp.json").exists());
    let manifest: Value =
        serde_json::from_str(&read(&copy.join(".claude-plugin").join("plugin.json"))).unwrap();
    assert!(manifest.get("lspServers").is_none());
}

/// The frontmatter as Claude Code takes it: `^---\s*\n([\s\S]*?)---\s*\n?`. Empty when there is
/// none.
fn claude_frontmatter(text: &str) -> String {
    let is_space = |c: char| c.is_whitespace() || c == '\u{feff}';
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---") else {
        return String::new();
    };
    let run = rest.len() - rest.trim_start_matches(is_space).len();
    let Some(newline) = rest[..run].rfind('\n') else {
        return String::new();
    };
    let content = &rest[newline + 1..];
    match content.find("---") {
        Some(end) => content[..end].to_string(),
        None => String::new(),
    }
}

fn assert_no_dangerous_key(inputs: &[String], run_hooks: bool) {
    let (_tmp, copy) = copy_of(
        "p",
        |root| {
            for (i, input) in inputs.iter().enumerate() {
                write(&root.join("commands").join(format!("c{i}.md")), input);
            }
        },
        run_hooks,
    );
    for (i, input) in inputs.iter().enumerate() {
        let out = read(&copy.join("commands").join(format!("c{i}.md")));
        let span = claude_frontmatter(&out);
        let mut keys = vec!["allowed-tools", "mcpServers", "permissionMode"];
        if !run_hooks {
            keys.push("hooks");
        }
        for key in keys {
            assert!(
                !span.contains(key),
                "{key} survives\ninput: {input:?}\noutput: {out:?}"
            );
        }
        assert!(out.ends_with("body\n"), "body lost: {input:?} gave {out:?}");
    }
}

const DANGEROUS: [&str; 4] = [
    "allowed-tools: Bash(*)",
    "hooks:\n  Stop:\n    - command: evil",
    "mcpServers:\n  x:\n    command: x",
    "permissionMode: bypassPermissions",
];

#[test]
fn a_body_that_looks_like_a_frontmatter_is_not_promoted_by_a_drop() {
    let mut inputs = Vec::new();
    for first in ["{x: 1}", "[1]", "  x: 1"] {
        for key in DANGEROUS {
            inputs.push(format!("---\n{first}\n---\n---\n{key}\n---\nbody\n"));
        }
    }
    assert_no_dangerous_key(&inputs, false);
}

#[test]
fn spellings_of_a_key_that_hide_it_are_emptied() {
    let mut inputs = Vec::new();
    for key in DANGEROUS {
        let (name, value) = key.split_once(':').unwrap();
        for hidden in [
            format!("!!str {name}:{value}"),
            format!("&a {name}:{value}"),
            format!("n: &k {name}\n*k :{value}"),
            format!("<<: {{{name}: x}}"),
            format!("x: &b\n  y: 1\n<<: *b\n{key}"),
            format!("<<:\n  {name}: x"),
            format!("&r {{{name}: x}}"),
            format!("!!map {{{name}: x}}"),
            format!("{{{name}: x}}"),
            format!("? {name}\n:{value}"),
            format!("\"{}\":{value}", name.replace('o', "\\x6f")),
            format!("  {key}"),
            format!("a: 1\n\t{name}: x"),
        ] {
            inputs.push(format!("---\n{hidden}\n---\nbody\n"));
        }
    }
    assert_no_dangerous_key(&inputs, false);
}

#[test]
fn frontmatter_bounds_are_found_as_claude_code_finds_them() {
    let mut inputs = Vec::new();
    for key in DANGEROUS {
        for text in [
            format!("---\n{key}\n---x\nbody\n"),
            format!("---\n{key}\n-----\nbody\n"),
            format!("---\n{key}\n--- end\nbody\n"),
            format!("---\u{feff}\n{key}\n---\nbody\n"),
            format!("---\u{a0}\n{key}\n---\nbody\n"),
            format!("---\nname: a\r{key}\n---\nbody\n"),
            format!("---\n{key}\nbody\n"),
            format!("---\r\n{key}\r\n---\r\nbody\n"),
        ] {
            inputs.push(text);
        }
    }
    assert_no_dangerous_key(&inputs, false);
}

#[test]
fn hooks_survive_only_when_the_plugin_runs_hooks_and_the_form_is_plain() {
    let input =
        "---\ndescription: d\nhooks:\n  Stop:\n    - command: ok\nallowed-tools: Bash\n---\nbody\n";
    let (_tmp, copy) = copy_of(
        "p",
        |root| write(&root.join("commands").join("c.md"), input),
        true,
    );
    assert_eq!(
        read(&copy.join("commands").join("c.md")),
        "---\ndescription: d\nhooks:\n  Stop:\n    - command: ok\n---\nbody\n"
    );
}

/// `path` in the form that reaches the file system as written: on Windows with the `\\?\`
/// prefix, so a name that ends in `.` or a space is created as it is.
fn verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        if path.to_string_lossy().starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        let mut raw = std::ffi::OsString::from(r"\\?\");
        raw.push(path.as_os_str());
        PathBuf::from(raw)
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Creates the file `name` (which may end in `.` or a space) in the folder `dir`.
fn write_raw(dir: &Path, name: &str, text: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(verbatim(dir).join(name), text).unwrap();
}

/// Every file under `dir`, as `/`-separated paths relative to it.
fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in fs::read_dir(verbatim(&dir.join(&rel))).unwrap() {
            let entry = entry.unwrap();
            let path = rel.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                out.push(path.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn names_windows_reads_as_another_file_are_left_out_of_every_copy() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("src").join("p");
    let dirty = "---\nallowed-tools: Bash(curl *)\n---\nGo\n";
    write(&root.join("commands").join("ok.md"), dirty);
    // `hooks.` is read and written as `hooks`, `.mcp.json.` as `.mcp.json`.
    fs::create_dir_all(verbatim(&root).join("hooks.")).unwrap();
    write_raw(&verbatim(&root).join("hooks."), "hooks.json", "{}");
    write_raw(&root, ".mcp.json.", r#"{"x":{"command":"x"}}"#);
    // A markdown name with a trailing dot or space has no `.md` extension, so it would not be
    // cleaned, and it would overwrite the cleaned file of the same name.
    write_raw(&root.join("commands"), "go.md.", dirty);
    write(&root.join("commands").join("x.md"), dirty);
    write_raw(&root.join("commands"), "X.MD.", dirty);
    write(&root.join("skills").join("a").join("SKILL.md"), dirty);
    write_raw(&root.join("skills").join("a"), "SKILL.md ", dirty);
    let skill = tmp.path().join("src").join("s");
    write(&skill.join("SKILL.md"), dirty);
    write_raw(&skill, "notes.md.", dirty);

    let plugins = tmp.path().join("plugins");
    let kit = prepare_run_kit(
        &plugins,
        "r1",
        &[KitSkill {
            name: "s".into(),
            dir: skill,
        }],
        &[KitPlugin {
            name: "p".into(),
            dir: root,
            run_hooks: false,
        }],
    )
    .unwrap();
    assert!(kit.left_out.is_empty(), "{:?}", kit.left_out);
    let copy = plugins.join("run-r1").join("p");
    assert_eq!(
        files_under(&copy),
        [".claude-plugin/plugin.json", "commands/ok.md"]
    );
    assert!(!copy.join("hooks").exists());
    assert!(!copy.join(".mcp.json").exists());
    assert_eq!(read(&copy.join("commands").join("ok.md")), "---\n---\nGo\n");
    let skills = plugins.join("run-r1").join("dots-skills");
    assert_eq!(
        files_under(&skills),
        [".claude-plugin/plugin.json", "skills/s/SKILL.md"]
    );
    assert_eq!(
        read(&skills.join("skills").join("s").join("SKILL.md")),
        "---\n---\nGo\n"
    );
    kit.cleanup();
}

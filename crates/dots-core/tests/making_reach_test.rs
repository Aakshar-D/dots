//! The reach check (dots-make-dots design sections 6, 7.2 and 9): what a maker asks for a new
//! dot, or as a change to a dot it made, becomes the whole dot, the parts beyond the maker's
//! reach, and the dot without them. Each row of the table in section 6 and each folder rule is
//! tried within reach and beyond it.

mod common;

use std::path::{Path, PathBuf};

use common::fake_dot;
use dots_core::context_rules::ContextRules;
use dots_core::library::{LibraryKind, Uses};
use dots_core::making::reach::{
    reach_change, reach_new, real_path, Extra, Reach, Wanted, WantedPermissions,
};
use dots_core::model::{Dot, DotSpec, EngineKind, Making, WorkspaceMode};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::schedule::Schedule;
use dots_core::spend::DotCaps;
use dots_core::vault::Grants;
use dots_core::Error;
use tempfile::TempDir;

/// The folders of a test: the maker's, one outside it, and where worktree copies are made.
struct Place {
    _tmp: TempDir,
    maker_dir: PathBuf,
    outside: PathBuf,
    worktrees: PathBuf,
}

fn place() -> Place {
    let tmp = tempfile::tempdir().unwrap();
    let root = real_path(tmp.path());
    let place = Place {
        maker_dir: root.join("maker"),
        outside: root.join("outside"),
        worktrees: root.join("data").join("worktrees"),
        _tmp: tmp,
    };
    for dir in [&place.maker_dir, &place.outside, &place.worktrees] {
        std::fs::create_dir_all(dir).unwrap();
    }
    place
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A maker named Researcher in `place`'s folder: sandboxed, 40 turns, 30 minutes.
fn maker(place: &Place, mode: WorkspaceMode) -> Dot {
    let mut dot = fake_dot("Researcher", &place.maker_dir, mode);
    dot.spec.making.on = true;
    dot
}

fn wanted() -> Wanted {
    Wanted {
        name: Some("Blog Watcher".into()),
        instructions: Some("Watch three blogs.".into()),
        ..Wanted::default()
    }
}

fn new(maker: &Dot, wanted: &Wanted, place: &Place) -> Reach {
    reach_new(maker, wanted, &place.worktrees).unwrap()
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn grants(pairs: &[(&str, &[&str])]) -> Grants {
    Grants(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), names(v)))
            .collect(),
    )
}

fn item(kind: LibraryKind, name: &str) -> Extra {
    Extra::Item {
        item_kind: kind,
        name: name.into(),
    }
}

fn variable(consumer: &str, name: &str) -> Extra {
    Extra::Variable {
        consumer: consumer.into(),
        name: name.into(),
    }
}

fn daily() -> Schedule {
    serde_json::from_value(serde_json::json!({"rule": {"kind": "daily", "time": "09:00"}})).unwrap()
}

/// `spec` as `DotSpec::validate` takes it: a blank folder is filled when the dot is created
/// (`DotAdmin::create`), so it stands in here.
fn assert_valid(spec: &DotSpec) {
    let mut spec = spec.clone();
    if spec.workdir.is_empty() {
        spec.workdir = "C:/tmp/new".into();
    }
    spec.validate().unwrap();
}

fn invalid(result: dots_core::Result<Reach>) -> String {
    match result {
        Err(Error::Invalid(text)) => text,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// One item of each kind.
fn uses_all() -> Uses {
    Uses {
        mcp_servers: names(&["github"]),
        skills: names(&["pdf-tools"]),
        plugins: names(&["reviewer"]),
        context: names(&["handbook"]),
        apps: names(&["blog_board"]),
    }
}

#[test]
fn a_new_dot_with_only_a_name_and_instructions_takes_the_makers_settings_and_nothing_more() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Worktree);
    let m = &mut maker.spec;
    m.engine = EngineKind::Local;
    m.model = "qwen3".into();
    m.endpoint_url = Some("http://127.0.0.1:1234".into());
    m.policy
        .rules
        .push(Rule::new("Bash(npm test:*)", Action::Allow));
    m.policy.preset = Preset::Custom;
    m.uses = uses_all();
    m.variables = grants(&[("shell", &["NPM_TOKEN"])]);
    m.browser = true;
    m.use_user_settings = true;
    m.max_turns = 55;
    m.timeout_secs = 999;
    m.approval_wait_secs = 100;
    m.quiet_limit_secs = Some(600);
    m.use_global_instructions = false;
    m.check_results = false;
    m.use_graph = false;
    m.context_rules = Some(ContextRules {
        enabled: true,
        compact_at: 50,
        clear_at: 70,
    });
    m.spend_cap = DotCaps {
        daily_usd: Some(2.0),
        daily_tokens: None,
    };
    m.schedule = Some(daily());
    m.enabled = false;

    let reach = new(&maker, &wanted(), &place);
    let m = &maker.spec;
    let expected = DotSpec {
        name: "Blog Watcher".into(),
        instructions: "Watch three blogs.".into(),
        engine: EngineKind::Local,
        model: "qwen3".into(),
        endpoint_url: Some("http://127.0.0.1:1234".into()),
        workdir: String::new(),
        workspace_mode: WorkspaceMode::Folder,
        schedule: None,
        policy: m.policy.clone(),
        uses: Uses::default(),
        variables: Grants::default(),
        browser: false,
        quiet_limit_secs: Some(600),
        spend_cap: DotCaps::default(),
        use_global_instructions: false,
        check_results: false,
        use_graph: false,
        context_rules: m.context_rules,
        making: Making::default(),
        use_user_settings: false,
        max_turns: 55,
        timeout_secs: 999,
        approval_wait_secs: 100,
        enabled: true,
    };
    assert_eq!(reach.spec, expected);
    assert_eq!(reach.without, expected);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.create_folder, None);
    assert!(!reach.spec.making.on);
}

#[test]
fn a_new_dot_needs_a_name_and_instructions_and_must_be_a_valid_dot() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let mut w = wanted();
    w.name = None;
    assert_eq!(
        invalid(reach_new(&maker, &w, &place.worktrees)),
        "A new dot needs a name."
    );
    w.name = Some("  ".into());
    assert_eq!(
        invalid(reach_new(&maker, &w, &place.worktrees)),
        "A new dot needs a name."
    );
    let mut w = wanted();
    w.instructions = Some(String::new());
    assert_eq!(
        invalid(reach_new(&maker, &w, &place.worktrees)),
        "A new dot needs instructions."
    );
    // Today's rules for any dot (design section 6, notes).
    let mut w = wanted();
    w.name = Some("bad/name".into());
    assert!(invalid(reach_new(&maker, &w, &place.worktrees)).contains("1 to 64"));
    let mut w = wanted();
    w.max_turns = Some(0);
    assert!(invalid(reach_new(&maker, &w, &place.worktrees)).contains("max_turns"));
    // `Custom` is no plain preset.
    let mut w = wanted();
    w.permissions = Some(WantedPermissions::Preset(Preset::Custom));
    assert!(invalid(reach_new(&maker, &w, &place.worktrees)).contains("permissions is"));
}

#[test]
fn a_named_model_and_schedule_are_taken_as_asked() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let mut w = wanted();
    w.model = Some("haiku".into());
    w.schedule = Some(Some(daily()));
    let reach = new(&maker, &w, &place);
    assert_eq!(reach.spec.model, "haiku");
    assert_eq!(reach.spec.schedule, Some(daily()));
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec, reach.without);
    // "No schedule" is what a new dot has anyway.
    w.schedule = Some(None);
    assert_eq!(new(&maker, &w, &place).spec.schedule, None);
}

#[test]
fn an_item_the_maker_uses_is_within_reach_and_one_it_lacks_is_an_extra() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.uses = uses_all();

    let mut w = wanted();
    w.uses = Some(uses_all());
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.uses, uses_all());
    assert_eq!(reach.spec, reach.without);

    // One more of each kind, which the maker lacks.
    let mut more = uses_all();
    more.mcp_servers.push("slack".into());
    more.skills.push("xlsx".into());
    more.plugins.push("linter".into());
    more.context.push("pricing".into());
    more.apps.push("crm".into());
    w.uses = Some(more.clone());
    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            item(LibraryKind::McpServer, "slack"),
            item(LibraryKind::Skill, "xlsx"),
            item(LibraryKind::Plugin, "linter"),
            item(LibraryKind::Context, "pricing"),
            item(LibraryKind::App, "crm"),
        ]
    );
    assert_eq!(reach.spec.uses, more);
    assert_eq!(reach.without.uses, uses_all());

    // A name the maker has under another kind is not the maker's item.
    let mut w = wanted();
    w.uses = Some(Uses {
        skills: names(&["github"]),
        ..Uses::default()
    });
    assert_eq!(
        new(&maker, &w, &place).extras,
        vec![item(LibraryKind::Skill, "github")]
    );
}

#[test]
fn a_variable_is_within_reach_only_for_the_consumer_the_maker_is_granted_it_for() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.uses.mcp_servers = names(&["github"]);
    maker.spec.variables = grants(&[
        ("mcp:github", &["GITHUB_TOKEN", "GITHUB_ORG"]),
        ("shell", &["NPM_TOKEN"]),
    ]);

    let mut w = wanted();
    w.uses = Some(maker.spec.uses.clone());
    // Granted, in any case, as `Grants::allows` reads names.
    w.variables = Some(grants(&[
        ("mcp:github", &["github_token"]),
        ("shell", &["NPM_TOKEN"]),
    ]));
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.variables, w.variables.clone().unwrap());
    assert_eq!(reach.spec, reach.without);

    // Not granted at all, and granted for a different consumer only.
    w.variables = Some(grants(&[
        ("mcp:github", &["GITHUB_TOKEN", "NPM_TOKEN"]),
        ("shell", &["NPM_TOKEN", "GITHUB_TOKEN", "AWS_KEY"]),
    ]));
    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            variable("mcp:github", "NPM_TOKEN"),
            variable("shell", "GITHUB_TOKEN"),
            variable("shell", "AWS_KEY"),
        ]
    );
    assert_eq!(reach.spec.variables, w.variables.clone().unwrap());
    assert_eq!(
        reach.without.variables,
        grants(&[("mcp:github", &["GITHUB_TOKEN"]), ("shell", &["NPM_TOKEN"])])
    );

    // A consumer left with no name is dropped.
    w.variables = Some(grants(&[("shell", &["AWS_KEY"])]));
    let reach = new(&maker, &w, &place);
    assert_eq!(reach.extras, vec![variable("shell", "AWS_KEY")]);
    assert_eq!(reach.without.variables, Grants::default());
}

#[test]
fn a_plain_preset_is_within_reach_when_no_stronger_than_the_makers() {
    let place = place();
    let order = [Preset::ReadOnly, Preset::Sandboxed, Preset::Trusted];
    for (m, makers) in order.iter().enumerate() {
        let mut maker = maker(&place, WorkspaceMode::Folder);
        maker.spec.policy = Policy::preset(*makers);
        for (p, preset) in order.iter().enumerate() {
            let mut w = wanted();
            w.permissions = Some(WantedPermissions::Preset(*preset));
            let reach = new(&maker, &w, &place);
            assert_eq!(reach.spec.policy, Policy::preset(*preset));
            if p <= m {
                assert!(reach.extras.is_empty(), "{makers:?} gives {preset:?}");
                assert_eq!(reach.spec, reach.without);
            } else {
                assert_eq!(
                    reach.extras,
                    vec![Extra::Preset {
                        preset: *preset,
                        makers: *makers
                    }],
                    "{makers:?} gives {preset:?}"
                );
                // Lowered to the maker's.
                assert_eq!(reach.without.policy, maker.spec.policy);
            }
        }
    }
}

#[test]
fn a_custom_maker_gives_its_own_policy_or_read_only() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.policy = Policy::preset(Preset::Trusted);
    maker
        .spec
        .policy
        .rules
        .push(Rule::new("Bash(rm:*)", Action::Deny));
    maker.spec.policy.preset = Preset::Custom;

    let mut w = wanted();
    w.permissions = Some(WantedPermissions::Maker);
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.policy, maker.spec.policy);

    w.permissions = Some(WantedPermissions::Preset(Preset::ReadOnly));
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.policy, Policy::preset(Preset::ReadOnly));
    assert_eq!(reach.spec, reach.without);

    for preset in [Preset::Sandboxed, Preset::Trusted] {
        w.permissions = Some(WantedPermissions::Preset(preset));
        let reach = new(&maker, &w, &place);
        assert_eq!(
            reach.extras,
            vec![Extra::Preset {
                preset,
                makers: Preset::Custom
            }]
        );
        assert_eq!(reach.spec.policy, Policy::preset(preset));
        assert_eq!(reach.without.policy, maker.spec.policy);
    }
}

#[test]
fn the_browser_and_the_user_settings_are_within_reach_only_if_the_maker_has_them() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    let mut w = wanted();
    w.browser = Some(true);
    w.use_user_settings = Some(true);

    let reach = new(&maker, &w, &place);
    assert_eq!(reach.extras, vec![Extra::Browser, Extra::UserSettings]);
    assert!(reach.spec.browser && reach.spec.use_user_settings);
    assert!(!reach.without.browser && !reach.without.use_user_settings);

    maker.spec.browser = true;
    maker.spec.use_user_settings = true;
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert!(reach.spec.browser && reach.spec.use_user_settings);
    assert_eq!(reach.spec, reach.without);

    // Asking for them off never asks.
    w.browser = Some(false);
    w.use_user_settings = Some(false);
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert!(!reach.spec.browser && !reach.spec.use_user_settings);
}

#[test]
fn a_limit_equal_to_the_makers_is_within_reach_above_is_an_extra_and_below_is_kept() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    assert_eq!((maker.spec.max_turns, maker.spec.timeout_secs), (40, 1800));

    let mut w = wanted();
    w.max_turns = Some(40);
    w.timeout_secs = Some(1800);
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!((reach.spec.max_turns, reach.spec.timeout_secs), (40, 1800));

    w.max_turns = Some(41);
    w.timeout_secs = Some(7200);
    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            Extra::MaxTurns {
                wanted: 41,
                makers: 40
            },
            Extra::Timeout {
                wanted_secs: 7200,
                makers_secs: 1800
            },
        ]
    );
    assert_eq!((reach.spec.max_turns, reach.spec.timeout_secs), (41, 7200));
    assert_eq!(
        (reach.without.max_turns, reach.without.timeout_secs),
        (40, 1800)
    );

    w.max_turns = Some(5);
    w.timeout_secs = Some(60);
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!((reach.spec.max_turns, reach.spec.timeout_secs), (5, 60));
    assert_eq!(reach.spec, reach.without);
}

#[test]
fn without_is_the_request_minus_every_extra_and_a_valid_dot() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.uses.skills = names(&["pdf-tools"]);
    maker.spec.variables = grants(&[("shell", &["NPM_TOKEN"])]);

    let mut w = wanted();
    w.model = Some("haiku".into());
    w.folder = Some(text(&place.outside));
    w.permissions = Some(WantedPermissions::Preset(Preset::Trusted));
    w.uses = Some(Uses {
        mcp_servers: names(&["github"]),
        skills: names(&["pdf-tools", "xlsx"]),
        ..Uses::default()
    });
    w.variables = Some(grants(&[
        ("mcp:github", &["GITHUB_TOKEN"]),
        ("shell", &["NPM_TOKEN", "AWS_KEY"]),
    ]));
    w.browser = Some(true);
    w.use_user_settings = Some(true);
    w.max_turns = Some(80);
    w.timeout_secs = Some(7200);
    w.schedule = Some(Some(daily()));

    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            item(LibraryKind::McpServer, "github"),
            item(LibraryKind::Skill, "xlsx"),
            variable("mcp:github", "GITHUB_TOKEN"),
            variable("shell", "AWS_KEY"),
            Extra::Preset {
                preset: Preset::Trusted,
                makers: Preset::Sandboxed
            },
            Extra::Folder {
                path: text(&place.outside),
                below_maker: false
            },
            Extra::Browser,
            Extra::UserSettings,
            Extra::MaxTurns {
                wanted: 80,
                makers: 40
            },
            Extra::Timeout {
                wanted_secs: 7200,
                makers_secs: 1800
            },
        ]
    );

    // As asked.
    let spec = &reach.spec;
    assert_eq!(spec.workdir, text(&place.outside));
    assert_eq!(spec.workspace_mode, WorkspaceMode::Folder);
    assert_eq!(spec.policy, Policy::preset(Preset::Trusted));
    assert_eq!(spec.uses, w.uses.clone().unwrap());
    assert_eq!(spec.variables, w.variables.clone().unwrap());
    assert!(spec.browser && spec.use_user_settings);
    assert_eq!((spec.max_turns, spec.timeout_secs), (80, 7200));
    assert_valid(spec);

    // The same dot with nothing asked beyond reach: what is within reach stays.
    let mut plain = w.clone();
    plain.folder = None;
    plain.permissions = None;
    plain.uses = Some(Uses {
        skills: names(&["pdf-tools"]),
        ..Uses::default()
    });
    plain.variables = Some(grants(&[("shell", &["NPM_TOKEN"])]));
    plain.browser = None;
    plain.use_user_settings = None;
    plain.max_turns = None;
    plain.timeout_secs = None;
    let within = new(&maker, &plain, &place);
    assert!(within.extras.is_empty());
    assert_eq!(reach.without, within.spec);
    assert_eq!(reach.without.model, "haiku");
    assert_eq!(reach.without.schedule, Some(daily()));
    assert_eq!(reach.without.workdir, "");
    assert_eq!(reach.without.policy, maker.spec.policy);
    assert_valid(&reach.without);
}

#[test]
fn a_blank_folder_is_a_new_folder_of_the_dots_own() {
    let place = place();
    for mode in [WorkspaceMode::Folder, WorkspaceMode::Worktree] {
        let maker = maker(&place, mode);
        for folder in [None, Some(String::new()), Some("  ".to_string())] {
            let mut w = wanted();
            w.folder = folder;
            let reach = new(&maker, &w, &place);
            assert!(reach.extras.is_empty());
            assert_eq!(reach.spec.workdir, "");
            assert_eq!(reach.spec.workspace_mode, WorkspaceMode::Folder);
            assert_eq!(reach.create_folder, None);
        }
    }
}

#[test]
fn the_makers_own_folder_is_within_reach_in_the_makers_mode() {
    let place = place();
    for mode in [WorkspaceMode::Folder, WorkspaceMode::Worktree] {
        let maker = maker(&place, mode);
        // Named as it is, and by a relative path from itself.
        for folder in [text(&place.maker_dir), ".".to_string()] {
            let mut w = wanted();
            w.folder = Some(folder);
            let reach = new(&maker, &w, &place);
            assert!(reach.extras.is_empty());
            assert_eq!(reach.spec.workdir, maker.spec.workdir);
            assert_eq!(reach.spec.workspace_mode, mode);
            assert_eq!(reach.create_folder, None);
            assert_eq!(reach.spec, reach.without);
        }
    }
}

#[test]
fn a_subfolder_of_a_folder_mode_maker_is_within_reach_and_made_when_missing() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let there = place.maker_dir.join("research").join("blogs");
    std::fs::create_dir_all(&there).unwrap();
    let missing = place.maker_dir.join("research").join("new").join("deep");

    // One that exists, given absolute and given relative.
    for folder in [
        text(&there),
        "research/blogs".to_string(),
        r"research\blogs".to_string(),
    ] {
        let mut w = wanted();
        w.folder = Some(folder);
        let reach = new(&maker, &w, &place);
        assert!(reach.extras.is_empty());
        assert_eq!(reach.spec.workdir, text(&there));
        assert_eq!(reach.spec.workspace_mode, WorkspaceMode::Folder);
        assert_eq!(reach.create_folder, None);
        assert_eq!(reach.spec, reach.without);
    }

    // One that does not exist yet: the caller makes it; this check makes nothing.
    for folder in [text(&missing), "research/new/deep".to_string()] {
        let mut w = wanted();
        w.folder = Some(folder);
        let reach = new(&maker, &w, &place);
        assert!(reach.extras.is_empty());
        assert_eq!(reach.spec.workdir, text(&missing));
        assert_eq!(reach.spec.workspace_mode, WorkspaceMode::Folder);
        assert_eq!(reach.create_folder, Some(missing.clone()));
        assert_eq!(reach.spec, reach.without);
        assert!(!place.maker_dir.join("research").join("new").exists());
    }

    // A worktree-mode maker's own mode does not reach a subfolder; see the test below. And a
    // file is no folder.
    std::fs::write(place.maker_dir.join("notes.txt"), "x").unwrap();
    for folder in ["notes.txt", "notes.txt/below"] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        assert!(invalid(reach_new(&maker, &w, &place.worktrees)).contains("is a file"));
    }
}

#[test]
fn dot_dot_that_climbs_out_of_the_makers_folder_is_anywhere_else() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    std::fs::create_dir_all(place.maker_dir.join("research")).unwrap();

    let mut w = wanted();
    w.folder = Some("research/../../outside".into());
    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![Extra::Folder {
            path: text(&place.outside),
            below_maker: false
        }]
    );
    assert_eq!(reach.spec.workdir, text(&place.outside));
    assert_eq!(reach.without.workdir, "");

    // Through a folder that does not exist, too.
    w.folder = Some("nowhere/../../outside".into());
    assert_eq!(new(&maker, &w, &place).extras, reach.extras);

    // A `..` that stays inside is inside.
    w.folder = Some("research/../research".into());
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.workdir, text(&place.maker_dir.join("research")));
}

#[test]
fn a_subfolder_under_a_worktree_mode_maker_is_an_extra() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Worktree);
    let there = place.maker_dir.join("docs");
    std::fs::create_dir_all(&there).unwrap();

    let mut w = wanted();
    w.folder = Some("docs".into());
    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![Extra::Folder {
            path: text(&there),
            below_maker: true
        }]
    );
    assert_eq!(reach.spec.workdir, text(&there));
    assert_eq!(reach.spec.workspace_mode, WorkspaceMode::Folder);
    assert_eq!(reach.create_folder, None);
    assert_eq!(reach.without.workdir, "");
    assert_eq!(reach.without.workspace_mode, WorkspaceMode::Folder);

    // One still to be made is made only if the person allows it.
    let missing = place.maker_dir.join("docs").join("new");
    w.folder = Some("docs/new".into());
    let reach = new(&maker, &w, &place);
    assert_eq!(
        reach.extras,
        vec![Extra::Folder {
            path: text(&missing),
            below_maker: true
        }]
    );
    assert_eq!(reach.create_folder, Some(missing));
}

#[test]
fn a_folder_inside_a_runs_worktree_copy_is_refused() {
    let place = place();
    let copy = place.worktrees.join("01RUN");
    std::fs::create_dir_all(copy.join("src")).unwrap();
    for mode in [WorkspaceMode::Folder, WorkspaceMode::Worktree] {
        let maker = maker(&place, mode);
        for folder in [
            text(&copy),
            text(&copy.join("src")),
            text(&copy.join("not-there")),
            text(&place.worktrees),
        ] {
            let mut w = wanted();
            w.folder = Some(folder);
            let said = invalid(reach_new(&maker, &w, &place.worktrees));
            assert_eq!(
                said,
                "That folder is a run's working copy, which does not last. Leave the folder \
                 blank for a new one, or use Researcher's own folder."
            );
        }
    }
}

#[test]
fn anywhere_else_is_an_extra_when_it_exists_and_refused_when_it_does_not() {
    let place = place();
    for mode in [WorkspaceMode::Folder, WorkspaceMode::Worktree] {
        let maker = maker(&place, mode);
        let mut w = wanted();
        w.folder = Some(text(&place.outside));
        let reach = new(&maker, &w, &place);
        assert_eq!(
            reach.extras,
            vec![Extra::Folder {
                path: text(&place.outside),
                below_maker: false
            }]
        );
        assert_eq!(reach.spec.workdir, text(&place.outside));
        assert_eq!(reach.spec.workspace_mode, WorkspaceMode::Folder);
        assert_eq!(reach.create_folder, None);
        assert_eq!(reach.without.workdir, "");

        // The folder above the maker's is not the maker's.
        w.folder = Some("..".into());
        let reach = new(&maker, &w, &place);
        assert!(matches!(
            reach.extras[..],
            [Extra::Folder {
                below_maker: false,
                ..
            }]
        ));

        let missing = place.outside.join("not-there");
        w.folder = Some(text(&missing));
        assert_eq!(
            invalid(reach_new(&maker, &w, &place.worktrees)),
            format!(
                "The folder {} does not exist. Leave the folder blank for a new one, or name \
                 one that exists.",
                missing.display()
            )
        );
        std::fs::write(place.outside.join("file.txt"), "x").unwrap();
        w.folder = Some(text(&place.outside.join("file.txt")));
        assert!(invalid(reach_new(&maker, &w, &place.worktrees)).contains("is a file"));
    }
}

#[cfg(windows)]
#[test]
fn a_difference_of_case_does_not_put_an_inside_path_outside() {
    let place = place();
    let there = place.maker_dir.join("Research");
    std::fs::create_dir_all(&there).unwrap();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    // The maker's folder as the person typed it.
    maker.spec.workdir = maker.spec.workdir.to_uppercase();

    let mut w = wanted();
    w.folder = Some(text(&there).to_lowercase());
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    // The real path, in the case the folder has.
    assert_eq!(reach.spec.workdir, text(&there));

    // Parts that do not exist yet are compared without their case as well.
    w.folder = Some(text(&place.maker_dir.join("NEW")).to_lowercase());
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.create_folder, Some(place.maker_dir.join("new")));

    // And the maker's own folder in another case is the maker's own.
    w.folder = Some(text(&place.maker_dir).to_lowercase());
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.workdir, maker.spec.workdir);
}

/// Makes `link` lead to the folder `to`: a symlink, or on Windows a junction where this PC
/// may not make symlinks. `false` when it can make neither.
fn link_dir(to: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        if std::os::windows::fs::symlink_dir(to, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(to)
            .output()
            .is_ok_and(|out| out.status.success())
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(to, link).is_ok()
    }
}

#[test]
fn a_link_inside_the_makers_folder_that_leads_outside_is_outside() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    std::fs::create_dir_all(place.outside.join("deep")).unwrap();
    let link = place.maker_dir.join("shortcut");
    if !link_dir(&place.outside, &link) {
        eprintln!("skipped: this PC can make neither a symlink nor a junction");
        return;
    }

    for (folder, real) in [
        ("shortcut", place.outside.clone()),
        ("shortcut/deep", place.outside.join("deep")),
    ] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        let reach = new(&maker, &w, &place);
        assert_eq!(
            reach.extras,
            vec![Extra::Folder {
                path: text(&real),
                below_maker: false
            }]
        );
        assert_eq!(reach.spec.workdir, text(&real));
    }

    // Outside, so it must exist.
    let mut w = wanted();
    w.folder = Some("shortcut/not-there".into());
    assert!(invalid(reach_new(&maker, &w, &place.worktrees)).contains("does not exist"));

    // A link outside that leads into the maker's folder is inside.
    let back = place.outside.join("back");
    assert!(link_dir(&place.maker_dir, &back));
    let mut w = wanted();
    w.folder = Some(text(&back));
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.workdir, maker.spec.workdir);
}

/// A dot Researcher made, which the person then gave more than Researcher has.
fn made_dot(place: &Place) -> Dot {
    let dir = place.maker_dir.join("blogs");
    std::fs::create_dir_all(&dir).unwrap();
    let mut dot = fake_dot("Blog Watcher", &dir, WorkspaceMode::Folder);
    let s = &mut dot.spec;
    s.policy = Policy::preset(Preset::Trusted);
    s.uses.skills = names(&["pdf-tools", "xlsx"]);
    s.uses.mcp_servers = names(&["slack"]);
    s.variables = grants(&[("mcp:slack", &["SLACK_TOKEN"]), ("shell", &["AWS_KEY"])]);
    s.browser = true;
    s.use_user_settings = true;
    s.max_turns = 60;
    s.timeout_secs = 3600;
    s.making = Making {
        on: true,
        per_run: 2,
        total: 5,
    };
    s.enabled = false;
    s.spend_cap = DotCaps {
        daily_usd: Some(1.5),
        daily_tokens: None,
    };
    s.schedule = Some(daily());
    dot
}

fn change(maker: &Dot, made: &Dot, wanted: &Wanted, place: &Place) -> Reach {
    reach_change(maker, made, wanted, &place.worktrees).unwrap()
}

#[test]
fn a_change_of_the_instructions_alone_changes_nothing_else_and_asks_nothing() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.uses.skills = names(&["pdf-tools"]);
    let made = made_dot(&place);

    let w = Wanted {
        instructions: Some("Watch four blogs.".into()),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    let mut expected = made.spec.clone();
    expected.instructions = "Watch four blogs.".into();
    assert_eq!(reach.spec, expected);
    assert_eq!(reach.without, expected);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.create_folder, None);
    // Its own making settings, switch, cap and schedule stay as the person has them.
    assert_eq!(reach.spec.making, made.spec.making);
    assert!(!reach.spec.enabled);
    assert_eq!(reach.spec.spend_cap, made.spec.spend_cap);
    assert_eq!(reach.spec.schedule, Some(daily()));

    // Nothing named at all is no change.
    let reach = change(&maker, &made, &Wanted::default(), &place);
    assert_eq!(reach.spec, made.spec);
    assert!(reach.extras.is_empty());
}

#[test]
fn a_change_names_as_an_extra_only_what_the_dot_does_not_hold_already() {
    let place = place();
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.uses.skills = names(&["pdf-tools"]);
    maker.spec.variables = grants(&[("shell", &["NPM_TOKEN"])]);
    let made = made_dot(&place);

    // `xlsx` and `slack` are held already; `docx` is new and the maker lacks it.
    let mut uses = made.spec.uses.clone();
    uses.skills.push("docx".into());
    let w = Wanted {
        uses: Some(uses.clone()),
        variables: Some(grants(&[
            ("mcp:slack", &["SLACK_TOKEN", "SLACK_TEAM"]),
            ("shell", &["AWS_KEY", "NPM_TOKEN"]),
        ])),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            item(LibraryKind::Skill, "docx"),
            variable("mcp:slack", "SLACK_TEAM"),
        ]
    );
    assert_eq!(reach.spec.uses, uses);
    assert_eq!(reach.without.uses, made.spec.uses);
    assert_eq!(
        reach.without.variables,
        grants(&[
            ("mcp:slack", &["SLACK_TOKEN"]),
            ("shell", &["AWS_KEY", "NPM_TOKEN"])
        ])
    );
    // The rest of the dot is as it was, in both.
    let mut rest = reach.spec.clone();
    rest.uses = made.spec.uses.clone();
    rest.variables = made.spec.variables.clone();
    assert_eq!(rest, made.spec);
}

#[test]
fn removing_and_lowering_never_ask() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let made = made_dot(&place);

    let w = Wanted {
        uses: Some(Uses {
            skills: names(&["xlsx"]),
            ..Uses::default()
        }),
        variables: Some(Grants::default()),
        browser: Some(false),
        use_user_settings: Some(false),
        // Below the dot's own, above the maker's.
        max_turns: Some(50),
        timeout_secs: Some(2400),
        permissions: Some(WantedPermissions::Preset(Preset::Sandboxed)),
        schedule: Some(None),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert!(reach.extras.is_empty(), "{:?}", reach.extras);
    assert_eq!(reach.spec.uses.skills, names(&["xlsx"]));
    assert!(reach.spec.uses.mcp_servers.is_empty());
    assert_eq!(reach.spec.variables, Grants::default());
    assert!(!reach.spec.browser && !reach.spec.use_user_settings);
    assert_eq!((reach.spec.max_turns, reach.spec.timeout_secs), (50, 2400));
    assert_eq!(reach.spec.policy, Policy::preset(Preset::Sandboxed));
    assert_eq!(reach.spec.schedule, None);
    assert_eq!(reach.spec, reach.without);

    // Naming what it has is no change and asks nothing.
    let w = Wanted {
        browser: Some(true),
        use_user_settings: Some(true),
        max_turns: Some(60),
        timeout_secs: Some(3600),
        permissions: Some(WantedPermissions::Preset(Preset::Trusted)),
        folder: Some(made.spec.workdir.clone()),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert!(reach.extras.is_empty(), "{:?}", reach.extras);
    assert_eq!(reach.spec, made.spec);
}

#[test]
fn a_change_above_the_makers_reach_is_an_extra_even_where_the_dot_is_above_it_already() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let mut made = made_dot(&place);

    // The dot has 60 turns and an hour, both above the maker's 40 and 30 minutes.
    let w = Wanted {
        max_turns: Some(80),
        timeout_secs: Some(7200),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            Extra::MaxTurns {
                wanted: 80,
                makers: 40
            },
            Extra::Timeout {
                wanted_secs: 7200,
                makers_secs: 1800
            },
        ]
    );
    assert_eq!((reach.spec.max_turns, reach.spec.timeout_secs), (80, 7200));
    // Without them the dot keeps what the person gave it.
    assert_eq!(
        (reach.without.max_turns, reach.without.timeout_secs),
        (60, 3600)
    );

    // A dot below the maker's is raised to the maker's.
    made.spec.max_turns = 10;
    made.spec.timeout_secs = 600;
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(reach.extras.len(), 2);
    assert_eq!(
        (reach.without.max_turns, reach.without.timeout_secs),
        (40, 1800)
    );

    // The browser, the user settings and a stronger preset on a dot that lacks them.
    made.spec.browser = false;
    made.spec.use_user_settings = false;
    made.spec.policy = Policy::preset(Preset::ReadOnly);
    let w = Wanted {
        browser: Some(true),
        use_user_settings: Some(true),
        permissions: Some(WantedPermissions::Preset(Preset::Trusted)),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(
        reach.extras,
        vec![
            Extra::Preset {
                preset: Preset::Trusted,
                makers: Preset::Sandboxed
            },
            Extra::Browser,
            Extra::UserSettings,
        ]
    );
    assert_eq!(reach.spec.policy, Policy::preset(Preset::Trusted));
    assert!(reach.spec.browser && reach.spec.use_user_settings);
    // Without them: the maker's policy, which is within reach and above the dot's read-only.
    assert_eq!(reach.without.policy, maker.spec.policy);
    assert!(!reach.without.browser && !reach.without.use_user_settings);

    // The maker's own policy is always within reach.
    let w = Wanted {
        permissions: Some(WantedPermissions::Maker),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.policy, maker.spec.policy);
}

#[test]
fn a_changed_folder_follows_the_folder_rules_and_stays_as_it_was_without_the_extra() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let made = made_dot(&place);

    // Blank is not named: the dot keeps its folder.
    let w = Wanted {
        folder: Some(String::new()),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(reach.spec, made.spec);

    // Another folder below the maker's.
    let w = Wanted {
        folder: Some("news".into()),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert!(reach.extras.is_empty());
    let news = place.maker_dir.join("news");
    assert_eq!(reach.spec.workdir, text(&news));
    assert_eq!(reach.create_folder, Some(news));
    assert_eq!(reach.spec, reach.without);

    // One outside it.
    let w = Wanted {
        folder: Some(text(&place.outside)),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(
        reach.extras,
        vec![Extra::Folder {
            path: text(&place.outside),
            below_maker: false
        }]
    );
    assert_eq!(reach.spec.workdir, text(&place.outside));
    assert_eq!(reach.without, made.spec);

    // A worktree copy is refused for a change as well.
    let w = Wanted {
        folder: Some(text(&place.worktrees.join("01RUN"))),
        ..Wanted::default()
    };
    assert!(invalid(reach_change(&maker, &made, &w, &place.worktrees)).contains("working copy"));

    // A change must leave a valid dot.
    let w = Wanted {
        name: Some(String::new()),
        ..Wanted::default()
    };
    assert!(invalid(reach_change(&maker, &made, &w, &place.worktrees)).contains("1 to 64"));
}

#[test]
fn each_extra_says_itself_in_one_plain_line() {
    let lines: Vec<String> = [
        item(LibraryKind::Skill, "pdf-tools"),
        item(LibraryKind::McpServer, "github"),
        item(LibraryKind::Plugin, "reviewer"),
        item(LibraryKind::Context, "handbook"),
        item(LibraryKind::App, "blog_board"),
        variable("mcp:github", "GITHUB_TOKEN"),
        variable("shell", "NPM_TOKEN"),
        Extra::Preset {
            preset: Preset::Trusted,
            makers: Preset::Sandboxed,
        },
        Extra::Preset {
            preset: Preset::Sandboxed,
            makers: Preset::Custom,
        },
        Extra::Folder {
            path: r"C:\work\other".into(),
            below_maker: false,
        },
        Extra::Folder {
            path: r"C:\work\repo\docs".into(),
            below_maker: true,
        },
        Extra::SettingsFolder {
            path: r"C:\work\.git".into(),
        },
        Extra::Browser,
        Extra::UserSettings,
        Extra::MaxTurns {
            wanted: 80,
            makers: 40,
        },
        Extra::Timeout {
            wanted_secs: 7200,
            makers_secs: 1800,
        },
        Extra::Timeout {
            wanted_secs: 5400,
            makers_secs: 45,
        },
    ]
    .iter()
    .map(|extra| extra.line("Researcher"))
    .collect();
    assert_eq!(
        lines,
        [
            "The skill 'pdf-tools'",
            "The MCP server 'github'",
            "The plugin 'reviewer'",
            "The context collection 'handbook'",
            "The app 'blog_board'",
            "The variable GITHUB_TOKEN for the server 'github'",
            "The variable NPM_TOKEN for the shell",
            "The preset Trusted (Researcher has Sandboxed)",
            "The preset Sandboxed (Researcher has its own rules)",
            r"The folder C:\work\other, outside Researcher's folder",
            r"The folder C:\work\repo\docs, inside Researcher's folder (Researcher itself works in copies of it)",
            r"The folder C:\work\.git, inside Researcher's folder, where git or Claude keep their own settings",
            "The browser",
            "Your Claude settings",
            "A turn limit of 80 (Researcher has 40)",
            "A time limit of 2 h (Researcher has 30 min)",
            "A time limit of 1 h 30 min (Researcher has 45 s)",
        ]
    );
}

#[test]
fn extras_round_trip_as_json() {
    let extras = vec![
        item(LibraryKind::Skill, "pdf-tools"),
        variable("mcp:github", "GITHUB_TOKEN"),
        Extra::Preset {
            preset: Preset::Trusted,
            makers: Preset::Sandboxed,
        },
        Extra::Folder {
            path: r"C:\work\other".into(),
            below_maker: false,
        },
        Extra::Folder {
            path: r"C:\work\repo\docs".into(),
            below_maker: true,
        },
        Extra::SettingsFolder {
            path: r"C:\work\.git".into(),
        },
        Extra::Browser,
        Extra::UserSettings,
        Extra::MaxTurns {
            wanted: 80,
            makers: 40,
        },
        Extra::Timeout {
            wanted_secs: 7200,
            makers_secs: 1800,
        },
    ];
    let json = serde_json::to_value(&extras).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {"kind": "item", "item_kind": "skill", "name": "pdf-tools"},
            {"kind": "variable", "consumer": "mcp:github", "name": "GITHUB_TOKEN"},
            {"kind": "preset", "preset": "trusted", "makers": "sandboxed"},
            {"kind": "folder", "path": r"C:\work\other", "below_maker": false},
            {"kind": "folder", "path": r"C:\work\repo\docs", "below_maker": true},
            {"kind": "settings_folder", "path": r"C:\work\.git"},
            {"kind": "browser"},
            {"kind": "user_settings"},
            {"kind": "max_turns", "wanted": 80, "makers": 40},
            {"kind": "timeout", "wanted_secs": 7200, "makers_secs": 1800},
        ])
    );
    let back: Vec<Extra> = serde_json::from_value(json).unwrap();
    assert_eq!(back, extras);
}

#[test]
fn wanted_round_trips_as_json_and_refuses_what_it_does_not_know() {
    let json = serde_json::json!({
        "name": "Blog Watcher",
        "instructions": "Watch three blogs.",
        "model": "haiku",
        "folder": "research/blogs",
        "permissions": "read-only",
        "uses": {"skills": ["pdf-tools"], "mcp_servers": ["github"]},
        "variables": {"mcp:github": ["GITHUB_TOKEN"], "shell": ["NPM_TOKEN"]},
        "browser": false,
        "use_user_settings": false,
        "max_turns": 20,
        "timeout_secs": 900,
        "schedule": {"rule": {"kind": "weekly", "days": ["mon"], "time": "09:00"}},
    });
    let w: Wanted = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(w.name.as_deref(), Some("Blog Watcher"));
    assert_eq!(w.folder.as_deref(), Some("research/blogs"));
    assert_eq!(
        w.permissions,
        Some(WantedPermissions::Preset(Preset::ReadOnly))
    );
    assert_eq!(
        w.uses,
        Some(Uses {
            skills: names(&["pdf-tools"]),
            mcp_servers: names(&["github"]),
            ..Uses::default()
        })
    );
    assert_eq!(
        w.variables,
        Some(grants(&[
            ("mcp:github", &["GITHUB_TOKEN"]),
            ("shell", &["NPM_TOKEN"])
        ]))
    );
    assert_eq!((w.browser, w.use_user_settings), (Some(false), Some(false)));
    assert_eq!((w.max_turns, w.timeout_secs), (Some(20), Some(900)));
    assert!(matches!(w.schedule, Some(Some(_))));
    let again: Wanted = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(again, w);

    // The maker's own permissions.
    let w: Wanted = serde_json::from_value(serde_json::json!({"permissions": "maker"})).unwrap();
    assert_eq!(w.permissions, Some(WantedPermissions::Maker));
    assert_eq!(
        serde_json::to_value(&w).unwrap(),
        serde_json::json!({"permissions": "maker"})
    );
    for bad in ["custom", "admin"] {
        let err = serde_json::from_value::<Wanted>(serde_json::json!({"permissions": bad}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("permissions is \"maker\""), "{err}");
    }

    // Nothing named: an empty object, and back.
    assert_eq!(
        serde_json::to_value(Wanted::default()).unwrap(),
        serde_json::json!({})
    );
    assert_eq!(
        serde_json::from_value::<Wanted>(serde_json::json!({})).unwrap(),
        Wanted::default()
    );
    // `null` is not named, for every field.
    let nulls: Wanted = serde_json::from_value(serde_json::json!({
        "name": null, "instructions": null, "model": null, "folder": null,
        "permissions": null, "uses": null, "variables": null, "browser": null,
        "use_user_settings": null, "max_turns": null, "timeout_secs": null, "schedule": null,
    }))
    .unwrap();
    assert_eq!(nulls, Wanted::default());

    // "No schedule" is the word "none".
    let w: Wanted = serde_json::from_value(serde_json::json!({"schedule": "none"})).unwrap();
    assert_eq!(w.schedule, Some(None));
    let json = serde_json::to_value(&w).unwrap();
    assert_eq!(json, serde_json::json!({"schedule": "none"}));
    assert_eq!(serde_json::from_value::<Wanted>(json).unwrap(), w);
    let err = serde_json::from_value::<Wanted>(serde_json::json!({"schedule": "never"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("schedule is a schedule or \"none\""), "{err}");
    assert!(serde_json::from_value::<Wanted>(
        serde_json::json!({"schedule": {"rule": {"kind": "x"}}})
    )
    .is_err());

    // A field it does not have is refused: the account, the engine, "Can make dots".
    for field in ["making", "engine", "org_id", "workspace_mode", "enabled"] {
        let err = serde_json::from_value::<Wanted>(serde_json::json!({ (field): true }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"), "{err}");
    }
}

/// The one extra of `reach`, as JSON: a check that does not name the type's variant.
fn only_extra(reach: &Reach) -> serde_json::Value {
    assert_eq!(reach.extras.len(), 1, "{:?}", reach.extras);
    serde_json::to_value(&reach.extras[0]).unwrap()
}

#[test]
fn a_maker_whose_folder_is_not_a_full_path_has_no_folder_within_reach() {
    let place = place();
    for own in [".", "./", "a/..", "C:"] {
        let mut maker = maker(&place, WorkspaceMode::Folder);
        maker.spec.workdir = own.into();
        // Any folder given by its full path is beyond its reach, even the one it runs in.
        for folder in [&place.outside, &place.maker_dir] {
            let mut w = wanted();
            w.folder = Some(text(folder));
            let reach = new(&maker, &w, &place);
            assert_eq!(
                reach.extras,
                vec![Extra::Folder {
                    path: text(folder),
                    below_maker: false
                }],
                "{own}"
            );
            assert_eq!(reach.without.workdir, "");
        }
        // And one relative to it cannot be found.
        for folder in ["notes", ".", "../outside"] {
            let mut w = wanted();
            w.folder = Some(folder.into());
            assert_eq!(
                invalid(reach_new(&maker, &w, &place.worktrees)),
                "Name the folder by its full path: Researcher's own folder is not one.",
                "{own} {folder}"
            );
        }
        // A blank folder is still a new folder of the dot's own.
        let reach = new(&maker, &wanted(), &place);
        assert!(reach.extras.is_empty());
    }
}

#[test]
fn a_sibling_folder_that_shares_the_start_of_the_name_is_outside() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let sibling = place.maker_dir.with_file_name("makershop");
    std::fs::create_dir_all(&sibling).unwrap();
    for folder in [text(&sibling), "../makershop".to_string()] {
        let mut w = wanted();
        w.folder = Some(folder);
        assert_eq!(
            new(&maker, &w, &place).extras,
            vec![Extra::Folder {
                path: text(&sibling),
                below_maker: false
            }]
        );
    }
}

#[cfg(windows)]
#[test]
fn a_folder_on_another_computer_or_a_device_is_refused_before_the_disk_is_read() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    for folder in [
        r"\\dots-reach-test.invalid\share\x",
        r"//dots-reach-test.invalid/share/x",
        r"\\dots-reach-test.invalid@SSL\x",
        r"\\?\UNC\dots-reach-test.invalid\share\x",
        r"\\.\pipe\dots-reach-test",
        r"\\.\C:\Windows",
        r"\\?\GLOBALROOT\Device\Null",
    ] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        assert_eq!(
            invalid(reach_new(&maker, &w, &place.worktrees)),
            "That folder is on another computer or a device. Name a folder on this PC.",
            "{folder}"
        );
    }
    // A drive without its root names a different folder at each moment.
    let drive = text(&place.maker_dir)[..2].to_string();
    for folder in [format!("{drive}notes"), drive.clone()] {
        let mut w = wanted();
        w.folder = Some(folder.clone());
        assert_eq!(
            invalid(reach_new(&maker, &w, &place.worktrees)),
            format!(
                "Name the folder by its full path, such as {drive}\\notes, or by its path from \
                 Researcher's folder."
            ),
            "{folder}"
        );
    }
    // A plain drive path, also in its long form, is read as ever.
    let mut w = wanted();
    w.folder = Some(format!(r"\\?\{}", text(&place.maker_dir.join("research"))));
    let reach = new(&maker, &w, &place);
    assert!(reach.extras.is_empty());
    assert_eq!(reach.spec.workdir, text(&place.maker_dir.join("research")));
}

#[cfg(windows)]
#[test]
fn a_folder_is_read_as_windows_reads_it_where_the_dot_works() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let research = place.maker_dir.join("research");
    std::fs::create_dir_all(&research).unwrap();
    let above = place.maker_dir.parent().unwrap().to_path_buf();
    // Windows drops the trailing dots and spaces of the last part: `.. ` is the folder above,
    // not one inside.
    for folder in [".. ", "research/../.. "] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        let reach = new(&maker, &w, &place);
        assert_eq!(
            reach.extras,
            vec![Extra::Folder {
                path: text(&above),
                below_maker: false
            }],
            "{folder:?}"
        );
        assert_eq!(reach.spec.workdir, text(&above));
    }
    let mut w = wanted();
    w.folder = Some("research/.. ".into());
    assert_eq!(new(&maker, &w, &place).spec.workdir, maker.spec.workdir);
    // And `research. ` is `research`.
    for folder in ["research.", "research. ", "research/new. "] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        let reach = new(&maker, &w, &place);
        assert!(reach.extras.is_empty(), "{folder:?}");
        assert!(!reach.spec.workdir.ends_with([' ', '.']), "{folder:?}");
    }
}

#[test]
fn git_and_claude_folders_below_the_maker_are_beyond_its_reach() {
    let place = place();
    std::fs::create_dir_all(place.maker_dir.join(".git").join("hooks")).unwrap();
    std::fs::create_dir_all(place.maker_dir.join("app").join(".claude")).unwrap();
    for mode in [WorkspaceMode::Folder, WorkspaceMode::Worktree] {
        let maker = maker(&place, mode);
        for (folder, real) in [
            (".git", place.maker_dir.join(".git")),
            (".git/hooks", place.maker_dir.join(".git").join("hooks")),
            (".git/new", place.maker_dir.join(".git").join("new")),
            ("app/.claude", place.maker_dir.join("app").join(".claude")),
            (
                "app/.Claude/x",
                place.maker_dir.join("app").join(".claude").join("x"),
            ),
            ("new/.claude", place.maker_dir.join("new").join(".claude")),
            (".git.", place.maker_dir.join(".git")),
        ] {
            let mut w = wanted();
            w.folder = Some(folder.into());
            let reach = new(&maker, &w, &place);
            // Elsewhere case counts, and `.git.` is a folder of its own.
            if cfg!(windows) || !matches!(folder, ".git." | "app/.Claude/x") {
                assert_eq!(
                    only_extra(&reach),
                    serde_json::json!({"kind": "settings_folder", "path": text(&real)}),
                    "{folder}"
                );
                assert_eq!(reach.spec.workdir, text(&real));
            } else {
                assert_eq!(only_extra(&reach)["kind"], "settings_folder", "{folder}");
            }
            assert_eq!(reach.spec.workspace_mode, WorkspaceMode::Folder);
            assert_eq!(reach.without.workdir, "");
            assert_eq!(
                reach.extras[0].line("Researcher"),
                format!(
                    "The folder {}, inside Researcher's folder, where git or Claude keep their \
                     own settings",
                    reach.spec.workdir
                )
            );
        }
        #[cfg(windows)]
        {
            let mut w = wanted();
            w.folder = Some(".GIT".into());
            assert_eq!(
                only_extra(&new(&maker, &w, &place))["kind"],
                "settings_folder"
            );
        }
    }

    // Only the parts below the maker's folder count: a maker that itself works inside a
    // `.claude` folder reaches its subfolders, and `.github` is no settings folder.
    let maker_dir = place.maker_dir.join(".claude").join("worktrees").join("w1");
    std::fs::create_dir_all(maker_dir.join(".github")).unwrap();
    let maker = fake_dot("Researcher", &maker_dir, WorkspaceMode::Folder);
    for folder in ["docs", ".github", ".gitx"] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        let reach = new(&maker, &w, &place);
        assert!(reach.extras.is_empty(), "{folder}: {:?}", reach.extras);
        assert_eq!(reach.spec.workdir, text(&maker_dir.join(folder)));
    }
}

#[test]
fn a_change_that_moves_a_made_dot_into_git_is_an_extra() {
    let place = place();
    std::fs::create_dir_all(place.maker_dir.join(".git")).unwrap();
    let maker = maker(&place, WorkspaceMode::Folder);
    let made = made_dot(&place);
    let w = Wanted {
        folder: Some(".git".into()),
        ..Wanted::default()
    };
    let reach = change(&maker, &made, &w, &place);
    assert_eq!(
        only_extra(&reach),
        serde_json::json!({"kind": "settings_folder", "path": text(&place.maker_dir.join(".git"))})
    );
    assert_eq!(reach.spec.workdir, text(&place.maker_dir.join(".git")));
    assert_eq!(reach.without, made.spec);
}

#[test]
fn a_dangling_link_inside_the_makers_folder_is_outside() {
    let place = place();
    let maker = maker(&place, WorkspaceMode::Folder);
    let target = place.outside.join("gone");
    std::fs::create_dir_all(&target).unwrap();
    let link = place.maker_dir.join("shortcut");
    if !link_dir(&target, &link) {
        eprintln!("skipped: this PC can make neither a symlink nor a junction");
        return;
    }
    std::fs::remove_dir(&target).unwrap();
    for folder in ["shortcut", "shortcut/below"] {
        let mut w = wanted();
        w.folder = Some(folder.into());
        let said = invalid(reach_new(&maker, &w, &place.worktrees));
        assert!(said.contains("does not exist"), "{folder}: {said}");
    }
}

/// A custom policy: `rules` on a default of `default`, MCP tools denied.
fn custom(rules: &[(&str, Action)], default: Action) -> Policy {
    Policy {
        preset: Preset::Custom,
        rules: rules.iter().map(|(t, a)| Rule::new(t, *a)).collect(),
        default,
        mcp_default: Action::Deny,
        ask_send_delete: true,
    }
}

#[test]
fn a_custom_maker_gives_read_only_only_when_its_rules_allow_all_that_read_only_allows() {
    let place = place();
    let mut w = wanted();
    w.permissions = Some(WantedPermissions::Preset(Preset::ReadOnly));
    let read_only = Extra::Preset {
        preset: Preset::ReadOnly,
        makers: Preset::Custom,
    };

    // One meant to call one MCP tool, which may not read the disk.
    let narrow = custom(
        &[
            ("Read", Action::Deny),
            ("mcp__notes__add", Action::Allow),
            ("MakeDots", Action::Allow),
        ],
        Action::Deny,
    );
    // The rules a custom policy starts from: workflows ask, which read-only allows.
    let from_sandboxed = Policy::preset(Preset::Custom);
    // Trusted but for one folder, which read-only would read.
    let mut but_secret = Policy::preset(Preset::Trusted);
    but_secret
        .rules
        .push(Rule::new("Read(C:/secret/**)", Action::Deny));
    but_secret.preset = Preset::Custom;
    // Allows nothing beyond its default, which asks.
    let asks = custom(&[], Action::Ask);

    for policy in [narrow, from_sandboxed, but_secret, asks] {
        let mut maker = maker(&place, WorkspaceMode::Folder);
        maker.spec.policy = policy.clone();
        let reach = new(&maker, &w, &place);
        assert_eq!(reach.extras, vec![read_only.clone()], "{policy:?}");
        assert_eq!(reach.spec.policy, Policy::preset(Preset::ReadOnly));
        assert_eq!(reach.without.policy, policy);
        assert_eq!(
            reach.extras[0].line("Researcher"),
            "The preset Read-only (Researcher has its own rules)"
        );
        // Its own rules it may still give.
        let mut own = wanted();
        own.permissions = Some(WantedPermissions::Maker);
        assert!(new(&maker, &own, &place).extras.is_empty());
    }

    // One that allows everything, but for one command, gives read-only.
    let mut wide = Policy::preset(Preset::Trusted);
    wide.rules.push(Rule::new("Bash(rm:*)", Action::Deny));
    wide.preset = Preset::Custom;
    let mut maker = maker(&place, WorkspaceMode::Folder);
    maker.spec.policy = wide.clone();
    assert!(new(&maker, &w, &place).extras.is_empty());

    // A made dot whose own custom rules allow all that read-only allows may be changed to
    // read-only by a narrower maker: it holds that already.
    maker.spec.policy = custom(&[("Read", Action::Deny)], Action::Deny);
    let mut made = made_dot(&place);
    made.spec.policy = wide;
    let change_to = Wanted {
        permissions: Some(WantedPermissions::Preset(Preset::ReadOnly)),
        ..Wanted::default()
    };
    assert!(change(&maker, &made, &change_to, &place).extras.is_empty());

    // Plain presets keep their order: a Sandboxed maker gives read-only.
    let maker = self::maker(&place, WorkspaceMode::Folder);
    assert_eq!(maker.spec.policy.preset, Preset::Sandboxed);
    assert!(new(&maker, &w, &place).extras.is_empty());
}

#[test]
fn uses_with_a_group_it_does_not_have_is_refused_by_name() {
    use dots_core::making::tools::{check_arguments, Tool};
    let err = serde_json::from_value::<Wanted>(serde_json::json!({"uses": {"skill": ["pdf"]}}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("uses has no group \"skill\""), "{err}");
    assert!(
        err.contains("mcp_servers, skills, plugins, context, apps"),
        "{err}"
    );

    let make = serde_json::json!({
        "name": "Blog Watcher",
        "instructions": "Watch three blogs.",
        "purpose": "Watches blogs",
        "uses": {"skills": ["pdf-tools"], "skill": ["xlsx"]},
    });
    let change = serde_json::json!({"dot": "Blog Watcher", "uses": {"mcp_server": ["github"]}});
    for (tool, call, group) in [
        (Tool::MakeDot, make, "skill"),
        (Tool::ChangeDot, change, "mcp_server"),
    ] {
        let said = check_arguments(tool, &call).unwrap_err();
        assert!(
            said.contains(&format!("uses has no group \"{group}\"")),
            "{said}"
        );
    }
    // The groups it has still read, and `null` is not named.
    let w: Wanted =
        serde_json::from_value(serde_json::json!({"uses": {"apps": ["board"]}})).unwrap();
    assert_eq!(w.uses.unwrap().apps, names(&["board"]));
    let w: Wanted = serde_json::from_value(serde_json::json!({"uses": null})).unwrap();
    assert_eq!(w.uses, None);
}

//! Importing Claude Code agents as dots (import agents spec section 3): the scan of the user,
//! project and plugin folders, and the import that re-reads each file. The home folder is a
//! temporary one (`Config::claude_home`), so the real `~/.claude` is never read.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::spec;
use dots_core::claude_agents::{AgentSourceKind, ScannedAgent};
use dots_core::model::{DotSpec, WorkspaceMode};
use dots_core::policy::{Action, Policy, Preset};
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

/// `base` joined with each part of `rel` (`a/b.md`), so the path reads as the scan's does.
fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(base.to_path_buf(), |p, part| p.join(part))
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn agent(name: &str, extra: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: The {name} agent\n{extra}---\n{body}\n")
}

/// A runtime whose home folder is `<tmp>/home`, with no run ever started.
async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.max_concurrent_runs = 0;
    cfg.claude_home = Some(dir.path().join("home"));
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

impl Rt {
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn user_agent(&self, file: &str, text: &str) -> PathBuf {
        let path = under(&self.home(), &format!(".claude/agents/{file}"));
        write(&path, text);
        path
    }

    /// A plugin `name` installed under the home folder, with one agent file.
    fn plugin_agent(&self, plugin: &str, file: &str, text: &str) -> PathBuf {
        let root = self.dir.path().join("plugins").join(plugin);
        let plugins = under(&self.home(), ".claude/plugins");
        fs::create_dir_all(&plugins).unwrap();
        let listed = json!({"version": 2, "plugins": {
            format!("{plugin}@market"): [{"scope": "user", "installPath": root, "version": "1.0.0"}]
        }});
        fs::write(plugins.join("installed_plugins.json"), listed.to_string()).unwrap();
        let path = under(&root, &format!("agents/{file}"));
        write(&path, text);
        path
    }

    async fn scan(&self) -> Vec<ScannedAgent> {
        self.rt.scan_claude_agents().await.unwrap()
    }
}

fn find<'a>(found: &'a [ScannedAgent], path: &Path) -> &'a ScannedAgent {
    let path = path.to_string_lossy();
    found
        .iter()
        .find(|a| a.path == path)
        .unwrap_or_else(|| panic!("{path} not in {found:#?}"))
}

#[tokio::test]
async fn the_scan_finds_user_project_and_plugin_agents() {
    let rt = start().await;
    let user = rt.user_agent(
        "team/reviewer.md",
        &agent("reviewer", "tools: Read, Grep\nmodel: opus\n", "Review."),
    );
    let project_dir = rt.dir.path().join("project");
    let project = under(&project_dir, ".claude/agents/tester.md");
    write(&project, &agent("tester", "", "Test."));
    let mut with_project = spec("has-project");
    with_project.workdir = project_dir.to_string_lossy().into_owned();
    rt.rt.create_dot(with_project, None, None).await.unwrap();
    let plugin = rt.plugin_agent(
        "pdf-tools",
        "pdf.md",
        &agent("pdf", "tools: [Read, Frob]\n", "PDFs."),
    );
    let broken = rt.user_agent("empty.md", "---\nname: empty\n---\n");

    let found = rt.scan().await;
    assert_eq!(found.len(), 4, "{found:#?}");
    let r = find(&found, &user);
    assert_eq!(r.source, AgentSourceKind::User);
    assert_eq!(r.name, "reviewer");
    assert_eq!(r.description.as_deref(), Some("The reviewer agent"));
    assert_eq!(r.model.as_deref(), Some("opus"));
    assert_eq!(r.tools, Some(vec!["Read".to_string(), "Grep".to_string()]));
    assert!(r.error.is_none() && !r.already_imported);
    let t = find(&found, &project);
    assert_eq!(t.source, AgentSourceKind::Project);
    assert_eq!(
        t.project.as_deref(),
        Some(project_dir.to_string_lossy().as_ref())
    );
    let p = find(&found, &plugin);
    assert_eq!(p.source, AgentSourceKind::Plugin);
    assert_eq!(p.plugin.as_deref(), Some("pdf-tools"));
    assert!(p.warnings.iter().any(|w| w.contains("Frob")));
    let b = find(&found, &broken);
    assert!(b.error.as_deref().unwrap().contains("no instructions"));
}

#[tokio::test]
async fn an_agent_whose_name_is_taken_is_already_imported() {
    let rt = start().await;
    rt.rt
        .create_dot(spec("Reviewer"), None, None)
        .await
        .unwrap();
    let path = rt.user_agent("reviewer.md", &agent("reviewer", "", "Review."));
    assert!(find(&rt.scan().await, &path).already_imported);
}

#[tokio::test]
async fn an_import_makes_dots_from_the_files() {
    let rt = start().await;
    let reviewer = rt.user_agent(
        "reviewer.md",
        &agent(
            "code reviewer",
            "tools:\n  - Read\n  - Bash\nmodel: haiku\n",
            "Review code.",
        ),
    );
    let project_dir = rt.dir.path().join("project");
    let tester = under(&project_dir, ".claude/agents/tester.md");
    write(&tester, &agent("tester", "model: inherit\n", "Test."));
    let mut with_project = spec("has-project");
    with_project.workdir = project_dir.to_string_lossy().into_owned();
    with_project.workspace_mode = WorkspaceMode::Folder;
    rt.rt.create_dot(with_project, None, None).await.unwrap();

    let paths = vec![
        reviewer.to_string_lossy().into_owned(),
        tester.to_string_lossy().into_owned(),
    ];
    let done = rt.rt.import_claude_agents(paths, None, None).await.unwrap();
    assert!(done.failed.is_empty(), "{:?}", done.failed);
    assert_eq!(done.created.len(), 2);

    let r = &done.created[0].spec;
    assert_eq!(r.name, "code-reviewer");
    assert_eq!(r.instructions, "Review code.");
    assert_eq!(r.model, "haiku");
    assert_eq!(r.policy.preset, Preset::Custom);
    assert_eq!(
        r.policy.resolve("Read", &json!({"file_path": "a"})),
        Action::Allow
    );
    assert_eq!(
        r.policy.resolve("Bash", &json!({"command": "ls"})),
        Action::Ask
    );
    assert_eq!(r.workspace_mode, WorkspaceMode::Folder);
    // A folder of its own where a dot made without one gets it (`Config::new_dot_root`).
    let own = PathBuf::from(&r.workdir);
    assert_eq!(
        own.parent().unwrap(),
        rt.dir.path().join("data").join("new-dots")
    );
    assert!(own.is_dir());

    let t = &done.created[1].spec;
    assert_eq!(t.name, "tester");
    assert_eq!(t.model, DotSpec::new("x", "y", "z").model);
    assert_eq!(t.policy, Policy::preset(Preset::Sandboxed));
    assert_eq!(t.workdir, project_dir.to_string_lossy());
    assert_eq!(t.workspace_mode, WorkspaceMode::Folder);
}

#[tokio::test]
async fn an_agent_that_names_no_model_gets_the_default_model() {
    let rt = start().await;
    let mut s = rt.rt.settings().await;
    s.default_model = "opus".into();
    rt.rt.update_settings(s).await.unwrap();
    let plain = rt.user_agent("plain.md", &agent("plain", "", "Help."));
    let named = rt.user_agent(
        "named.md",
        &agent(
            "named",
            "model: haiku
",
            "Help.",
        ),
    );
    let paths = vec![
        plain.to_string_lossy().into_owned(),
        named.to_string_lossy().into_owned(),
    ];
    let done = rt.rt.import_claude_agents(paths, None, None).await.unwrap();
    let models: Vec<&str> = done.created.iter().map(|d| d.spec.model.as_str()).collect();
    assert_eq!(models, ["opus", "haiku"]);
}

#[tokio::test]
async fn a_taken_name_gets_a_number() {
    let rt = start().await;
    rt.rt.create_dot(spec("helper"), None, None).await.unwrap();
    let a = rt.user_agent("a.md", &agent("helper", "", "One."));
    let b = rt.user_agent("b.md", &agent("Helper", "", "Two."));
    let paths = vec![
        a.to_string_lossy().into_owned(),
        b.to_string_lossy().into_owned(),
    ];
    let done = rt.rt.import_claude_agents(paths, None, None).await.unwrap();
    let names: Vec<&str> = done.created.iter().map(|d| d.spec.name.as_str()).collect();
    assert_eq!(names, ["helper-2", "Helper-3"]);
}

#[tokio::test]
async fn only_paths_of_a_fresh_scan_are_imported() {
    let rt = start().await;
    let outside = under(rt.dir.path(), "elsewhere/agent.md");
    write(&outside, &agent("sneaky", "", "Hi."));
    let broken = rt.user_agent("empty.md", "---\nname: empty\n---\n");
    let changed = rt.user_agent("changed.md", &agent("changed", "", "Old."));
    // The file changes after the person saw it: the import reads it again.
    write(&changed, &agent("changed", "", "New."));
    let paths = vec![
        outside.to_string_lossy().into_owned(),
        broken.to_string_lossy().into_owned(),
        changed.to_string_lossy().into_owned(),
    ];
    let done = rt.rt.import_claude_agents(paths, None, None).await.unwrap();
    assert_eq!(done.created.len(), 1);
    assert_eq!(done.created[0].spec.instructions, "New.");
    assert_eq!(done.failed.len(), 2);
    assert!(
        done.failed[0].reason.contains("not found"),
        "{:?}",
        done.failed
    );
    assert!(done.failed[1].reason.contains("no instructions"));
}

#[tokio::test]
async fn an_import_into_an_unknown_workspace_is_refused() {
    let rt = start().await;
    let a = rt.user_agent("a.md", &agent("a", "", "One."));
    let err = rt
        .rt
        .import_claude_agents(
            vec![a.to_string_lossy().into_owned()],
            None,
            Some("nope".into()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert!(rt.rt.list_dots().await.unwrap().is_empty());
}

#[tokio::test]
async fn without_a_home_folder_nothing_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    assert!(rt.scan_claude_agents().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_path_given_twice_is_imported_once() {
    let rt = start().await;
    let a = rt.user_agent("a.md", &agent("a", "", "One."));
    let path = a.to_string_lossy().into_owned();
    let done = rt
        .rt
        .import_claude_agents(vec![path.clone(), path], None, None)
        .await
        .unwrap();
    assert_eq!(done.created.len(), 1);
    assert!(done.failed.is_empty(), "{:?}", done.failed);
}

#[tokio::test]
async fn imports_at_the_same_time_never_pick_the_same_name() {
    let rt = start().await;
    let a = rt.user_agent("a.md", &agent("helper", "", "One."));
    let paths = vec![a.to_string_lossy().into_owned()];
    let (one, two, three) = tokio::join!(
        rt.rt.import_claude_agents(paths.clone(), None, None),
        rt.rt.import_claude_agents(paths.clone(), None, None),
        rt.rt.import_claude_agents(paths, None, None),
    );
    let mut names: Vec<String> = [one, two, three]
        .into_iter()
        .flat_map(|r| r.unwrap().created)
        .map(|d| d.spec.name)
        .collect();
    names.sort();
    assert_eq!(names, ["helper", "helper-2", "helper-3"]);
}

#[tokio::test]
async fn only_projects_of_the_workspace_in_view_are_scanned() {
    let rt = start().await;
    let ops = rt.rt.personal_workspace_add("Ops").await.unwrap();
    let project_dir = rt.dir.path().join("ops-project");
    let tester = under(&project_dir, ".claude/agents/tester.md");
    write(&tester, &agent("tester", "", "Test."));
    let mut elsewhere = spec("in-ops");
    elsewhere.workdir = project_dir.to_string_lossy().into_owned();
    rt.rt
        .create_dot(elsewhere, None, Some(ops.id.clone()))
        .await
        .unwrap();
    assert!(rt.scan().await.is_empty());
    rt.rt.set_current_scope("personal", &ops.id).await.unwrap();
    assert_eq!(rt.scan().await.len(), 1);
}

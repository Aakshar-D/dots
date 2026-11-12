//! The one path for creating, changing and deleting a dot (dots-make-dots design section 12):
//! `DotAdmin` does for a dot a maker makes what the runtime does for one the person makes (the
//! folder, the schedule row, the role checks, the cleanup on delete), records who made it, and
//! stores a made dot's grants exactly as given (section 6, Variables).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, roster, sign, sign_roster, token, ACME, ACME_NAME,
    ISSUED, START, TOKEN,
};
use common::spec;
use dots_core::accounts::identity;
use dots_core::events::RuntimeEvent;
use dots_core::library::{LibraryConfig, McpServerConfig};
use dots_core::model::{Dot, DotSpec, Made, WorkspaceMode};
use dots_core::schedule::Schedule;
use dots_core::vault::{Grants, MemorySecrets};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

struct T {
    dir: TempDir,
    /// Where a dot made without a folder gets one (`Config::new_dot_root`).
    root: PathBuf,
    rt: Runtime,
}

fn config(dir: &TempDir) -> Config {
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.new_dot_root = dir.path().join("Dots");
    cfg
}

async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    let root = cfg.new_dot_root.clone();
    let rt = Runtime::start(cfg).await.unwrap();
    T { dir, root, rt }
}

/// A dot named `name` that works in the test's folder itself.
fn folder_spec(dir: &TempDir, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

/// A dot named `name` that leaves its folder blank.
fn blank_spec(name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = String::new();
    s
}

/// Every `minutes`, as the editor sends it. The first slot is hours away, so nothing fires
/// during a test.
fn every(minutes: u32) -> Schedule {
    serde_json::from_value(json!({
        "rule": {"kind": "every", "minutes": minutes},
        "catch_up": true,
        "until": null
    }))
    .unwrap()
}

/// A maker named Researcher, created as the person creates a dot.
async fn maker(t: &T) -> Dot {
    let mut s = folder_spec(&t.dir, "Researcher");
    s.making.on = true;
    t.rt.create_dot(s, None, None).await.unwrap()
}

/// The record of a dot `maker` made in its run `run-1`.
fn made_by(maker: &Dot) -> Made {
    Made {
        by_dot_id: Some(maker.id.clone()),
        by_name: maker.spec.name.clone(),
        run_id: "run-1".into(),
        purpose: "Reads the feeds".into(),
    }
}

/// Puts an MCP server named `github` in the Library whose config reads `GITHUB_TOKEN`.
async fn add_github(rt: &Runtime) {
    let config = LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: [("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    });
    rt.store()
        .add_library_item("github", &config, None)
        .await
        .unwrap();
}

fn github_token() -> Grants {
    Grants([("mcp:github".to_string(), vec!["GITHUB_TOKEN".to_string()])].into())
}

/// The dots named by the `ScheduleUpdated` events published since the last call.
fn updated(rx: &mut Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::ScheduleUpdated { dot_id } = ev {
            ids.push(dot_id);
        }
    }
    ids
}

/// The names of the folders in the new-dot root.
fn folders(root: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_made_dot_is_created_like_any_other_and_records_who_made_it() {
    let t = start().await;
    let maker = maker(&t).await;
    let mut rx = t.rt.subscribe();

    let mut s = blank_spec("Feed Reader");
    s.schedule = Some(every(600));
    let made = made_by(&maker);
    let dot =
        t.rt.dot_admin()
            .create(s, None, None, Some(made.clone()))
            .await
            .unwrap();

    // Its folder is a new one under the new-dot root, in folder mode.
    let want = t.root.join("feed-reader");
    assert_eq!(std::path::Path::new(&dot.spec.workdir), want);
    assert!(want.is_dir());
    assert_eq!(dot.spec.workspace_mode, WorkspaceMode::Folder);
    // It is in Personal's Default workspace, as a dot created with neither named.
    assert_eq!(
        (dot.org_id.as_deref(), dot.workspace_id.as_str()),
        (None, "default")
    );
    // Its schedule has its row, and the screens heard of it.
    let row = t.rt.store().get_schedule_state(&dot.id).await.unwrap();
    assert!(row.is_some());
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    // Who made it is as given, and its maker lists it.
    assert_eq!(dot.made, Some(made.clone()));
    assert_eq!(t.rt.get_dot(&dot.id).await.unwrap().made, Some(made));
    let listed = t.rt.store().list_made_dots(&maker.id).await.unwrap();
    assert_eq!(
        listed.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        [dot.id.as_str()]
    );

    // Without `made` the admin creates the dot as the runtime does: no maker recorded.
    let by_hand =
        t.rt.dot_admin()
            .create(blank_spec("By hand"), None, None, None)
            .await
            .unwrap();
    assert_eq!(by_hand.made, None);
    assert!(t.root.join("by-hand").is_dir());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_made_dot_gets_no_automatic_grant() {
    let t = start().await;
    add_github(&t.rt).await;
    let maker = maker(&t).await;

    // The person's dot that uses the server is granted the variables of its config.
    let mut by_hand = folder_spec(&t.dir, "By hand");
    by_hand.uses.mcp_servers = vec!["github".into()];
    let saved = t.rt.create_dot(by_hand, None, None).await.unwrap();
    assert_eq!(saved.spec.variables, github_token());

    // The same spec, made by a maker, keeps what it was given: nothing.
    let mut s = folder_spec(&t.dir, "Blog Watcher");
    s.uses.mcp_servers = vec!["github".into()];
    let dot =
        t.rt.dot_admin()
            .create(s, None, None, Some(made_by(&maker)))
            .await
            .unwrap();
    assert_eq!(dot.spec.uses.mcp_servers, ["github"]);
    assert_eq!(dot.spec.variables, Grants::default());
    assert_eq!(
        t.rt.get_dot(&dot.id).await.unwrap().spec.variables,
        Grants::default()
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn a_failed_made_create_removes_the_folder_it_made() {
    let t = start().await;
    let maker = maker(&t).await;
    let first =
        t.rt.dot_admin()
            .create(
                blank_spec("Blog Watcher"),
                None,
                None,
                Some(made_by(&maker)),
            )
            .await
            .unwrap();
    assert_eq!(folders(&t.root), ["blog-watcher"]);

    // The name is taken: the folder claimed for the second one goes again.
    let clash =
        t.rt.dot_admin()
            .create(
                blank_spec("blog watcher"),
                None,
                None,
                Some(made_by(&maker)),
            )
            .await;
    assert!(matches!(clash, Err(Error::Conflict(_))), "{clash:?}");
    assert_eq!(folders(&t.root), ["blog-watcher"]);
    let made = t.rt.store().list_made_dots(&maker.id).await.unwrap();
    assert_eq!(
        made.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        [first.id.as_str()]
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn an_exact_update_stores_grants_as_given() {
    let t = start().await;
    add_github(&t.rt).await;
    let maker = maker(&t).await;
    let admin = t.rt.dot_admin();

    // A made dot that starts to use the server, changed by its maker: no grant is added.
    let plain = admin
        .create(
            folder_spec(&t.dir, "Plain"),
            None,
            None,
            Some(made_by(&maker)),
        )
        .await
        .unwrap();
    let mut uses_it = plain.spec.clone();
    uses_it.uses.mcp_servers = vec!["github".into()];
    let exact = admin.update(&plain.id, uses_it, true).await.unwrap();
    assert_eq!(exact.spec.uses.mcp_servers, ["github"]);
    assert_eq!(exact.spec.variables, Grants::default());
    // And what it is given is what it holds.
    let mut given = exact.spec.clone();
    given.variables = github_token();
    let exact = admin.update(&plain.id, given, true).await.unwrap();
    assert_eq!(exact.spec.variables, github_token());

    // The same change saved as the person saves it grants the server its variables.
    let other = admin
        .create(
            folder_spec(&t.dir, "Other"),
            None,
            None,
            Some(made_by(&maker)),
        )
        .await
        .unwrap();
    let mut uses_it = other.spec.clone();
    uses_it.uses.mcp_servers = vec!["github".into()];
    let synced = admin.update(&other.id, uses_it, false).await.unwrap();
    assert_eq!(synced.spec.variables, github_token());
    t.rt.shutdown();
}

#[tokio::test]
async fn an_update_keeps_who_made_the_dot() {
    let t = start().await;
    let maker = maker(&t).await;
    let admin = t.rt.dot_admin();
    let dot = admin
        .create(
            blank_spec("Blog Watcher"),
            None,
            None,
            Some(made_by(&maker)),
        )
        .await
        .unwrap();
    let mut rx = t.rt.subscribe();

    // The maker's change: a new name and a schedule, whose row is made.
    let mut s = dot.spec.clone();
    s.name = "Feed Watcher".into();
    s.schedule = Some(every(600));
    let saved = admin.update(&dot.id, s.clone(), true).await.unwrap();
    assert_eq!(saved.made, dot.made);
    assert_eq!(saved.spec.name, "Feed Watcher");
    assert!(t
        .rt
        .store()
        .get_schedule_state(&dot.id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);

    // The person's edits, through the admin and through the runtime.
    s.instructions = "Watch the feeds.".into();
    let saved = admin.update(&dot.id, s.clone(), false).await.unwrap();
    assert_eq!(saved.made, dot.made);
    s.schedule = None;
    let saved = t.rt.update_dot(&dot.id, s).await.unwrap();
    assert_eq!(saved.made, dot.made);
    assert_eq!(
        t.rt.store().get_schedule_state(&dot.id).await.unwrap(),
        None
    );
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    assert_eq!(t.rt.store().count_made_dots(&maker.id).await.unwrap(), 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn deleting_through_the_admin_cleans_up_as_the_runtime_does() {
    let t = start().await;
    let maker = maker(&t).await;
    let admin = t.rt.dot_admin();
    let mut s = blank_spec("Blog Watcher");
    s.schedule = Some(every(600));
    let dot = admin
        .create(s, None, None, Some(made_by(&maker)))
        .await
        .unwrap();
    let plain = admin
        .create(blank_spec("Plain"), None, None, Some(made_by(&maker)))
        .await
        .unwrap();
    // The person has its conversation open.
    t.rt.set_viewing(Some(&dot.id));
    let mut rx = t.rt.subscribe();

    admin.delete(&dot.id).await.unwrap();
    assert!(matches!(
        t.rt.get_dot(&dot.id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        t.rt.store().get_schedule_state(&dot.id).await.unwrap(),
        None
    );
    assert_eq!(updated(&mut rx), [dot.id.as_str()]);
    // It can no longer be viewed: the admin shares the runtime's own record of that.
    assert_eq!(t.rt.viewing(), None);
    assert_eq!(t.rt.store().count_made_dots(&maker.id).await.unwrap(), 1);

    // A dot without a schedule had no row to announce.
    admin.delete(&plain.id).await.unwrap();
    assert!(updated(&mut rx).is_empty());
    assert!(matches!(
        admin.delete(&plain.id).await,
        Err(Error::NotFound(_))
    ));
    t.rt.shutdown();
}

/// Both engines serve their tools with the check-ins, which is where the tools for making dots
/// find the admin. It is the runtime's own, and is gone with the runtime.
#[tokio::test]
async fn a_runs_tools_reach_the_admin_through_the_check_ins() {
    let t = start().await;
    let maker = maker(&t).await;
    let checkins = t.rt.checkins().clone();
    let admin = checkins.dot_admin().expect("the admin, set at start");
    let dot = admin
        .create(
            blank_spec("Blog Watcher"),
            None,
            None,
            Some(made_by(&maker)),
        )
        .await
        .unwrap();
    assert_eq!(t.rt.get_dot(&dot.id).await.unwrap().made, dot.made);
    t.rt.delete_dot(&dot.id).await.unwrap();
    assert!(matches!(
        admin.delete(&dot.id).await,
        Err(Error::NotFound(_))
    ));

    drop(admin);
    t.rt.shutdown();
    drop(t.rt);
    assert!(checkins.dot_admin().is_none());
}

/// A runtime joined to Acme, whose signed member list the fake OpenBao serves, as
/// `runtime_roles_test.rs` sets it up.
struct Acme {
    t: T,
    bao: FakeOpenBao,
    keys: minisign::KeyPair,
}

impl Acme {
    async fn start() -> Acme {
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config(&dir);
        cfg.clock = FakeClock::new(at(START));
        cfg.secrets = Some(Arc::new(MemorySecrets::new()));
        let root = cfg.new_dot_root.clone();
        let rt = Runtime::start(cfg).await.unwrap();
        Acme {
            t: T { dir, root, rt },
            bao,
            keys: keys(),
        }
    }

    /// Serves a member list at `seq` with an owner who is not this PC, and this PC as `role`.
    async fn serve_list(&self, seq: u64, role: &str) {
        let me = identity(self.t.rt.store()).await.unwrap().public_key;
        let members = json!([
            member("01OWNER", "owner@acme.com", "owner", &public_key(&keys())),
            member("01ME", "me@acme.com", role, &me),
        ]);
        let extra = json!({"v": 2, "roster": "required", "admins": [], "offline_hours": 0});
        let signed = sign(&self.keys, &policy(ACME, ISSUED, extra));
        self.bao.put_env("secret", "dots", json!({}));
        self.bao.put_policy("secret", "dots", &signed);
        let list = sign_roster(&self.keys, &roster(ACME, seq, members));
        self.bao.put_roster("secret", "dots", &list);
    }

    /// Joins Acme with this PC on its member list as `role`.
    async fn join_as(&self, role: &str) {
        self.serve_list(1, role).await;
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        let orgs = self.t.rt.vault().orgs().expect("the runtime's orgs");
        orgs.join(&code, Some(token())).await.unwrap();
    }

    /// Serves the list again at `seq` with this PC as `role`, and fetches it.
    async fn become_(&self, seq: u64, role: &str) {
        self.serve_list(seq, role).await;
        self.t.rt.refresh_org(ACME).await.unwrap();
    }
}

fn member(id: &str, email: &str, role: &str, key: &str) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": {"default": "member"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

/// The text of a `NotAllowed` refusal; anything else fails the test.
fn refused<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::NotAllowed(text)) => text,
        other => panic!("expected NotAllowed, got {other:?}"),
    }
}

#[tokio::test]
async fn the_admin_applies_the_role_checks() {
    let a = Acme::start().await;
    let t = &a.t;
    a.join_as("member").await;
    let admin = t.rt.dot_admin();
    let mut s = folder_spec(&t.dir, "Researcher");
    s.making.on = true;
    let maker = t.rt.create_dot(s, Some(ACME.into()), None).await.unwrap();
    // A Member may make a dot in the account.
    let dot = admin
        .create(
            folder_spec(&t.dir, "Blog Watcher"),
            Some(ACME.into()),
            None,
            Some(made_by(&maker)),
        )
        .await
        .unwrap();
    assert_eq!(dot.org_id.as_deref(), Some(ACME));

    a.become_(2, "viewer").await;
    let edit = "Acme Engineering: a Viewer can't create or change dots. Ask an Admin or Owner.";
    // The made create is refused in the words the person's create is, and makes no folder.
    assert_eq!(
        refused(
            t.rt.create_dot(blank_spec("By hand"), Some(ACME.into()), None)
                .await
        ),
        edit
    );
    assert_eq!(
        refused(
            admin
                .create(
                    blank_spec("Feed Reader"),
                    Some(ACME.into()),
                    None,
                    Some(made_by(&maker)),
                )
                .await
        ),
        edit
    );
    assert!(folders(&t.root).is_empty());
    // So are a change and a delete, exact or not.
    let mut changed = dot.spec.clone();
    changed.instructions = "something else".into();
    assert_eq!(
        refused(admin.update(&dot.id, changed.clone(), true).await),
        edit
    );
    assert_eq!(refused(admin.update(&dot.id, changed, false).await), edit);
    assert_eq!(refused(admin.delete(&dot.id).await), edit);
    assert_eq!(t.rt.store().get_dot(&dot.id).await.unwrap().spec, dot.spec);

    // An account that is not joined on this PC, and a workspace it does not have, are refused
    // as for the person.
    let unknown = admin
        .create(
            blank_spec("Elsewhere"),
            Some("no-such-org".into()),
            None,
            Some(made_by(&maker)),
        )
        .await;
    assert!(matches!(unknown, Err(Error::Invalid(_))), "{unknown:?}");
    let nowhere = admin
        .create(
            blank_spec("Nowhere"),
            None,
            Some("no-such-workspace".into()),
            Some(made_by(&maker)),
        )
        .await;
    assert!(matches!(nowhere, Err(Error::NotFound(_))), "{nowhere:?}");
    assert!(folders(&t.root).is_empty());
    t.rt.shutdown();
}

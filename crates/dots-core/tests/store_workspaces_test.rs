//! The workspaces of each account, and the one every dot and team is in (accounts design §6.3).

mod common;

use common::{spec, temp_store};
use dots_core::accounts::{workspace_id_for, Workspace};
use dots_core::model::TeamSpec;
use dots_core::store::{Store, WorkspaceRow};
use dots_core::vault::org::{OrgRow, SopsSettings, SourceSettings, UseRules};
use dots_core::Error;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

fn acme() -> OrgRow {
    OrgRow {
        id: "acme-eng".into(),
        name: "Acme Engineering".into(),
        source: SourceSettings::Sops(SopsSettings {
            location: "https://files.acme.example/dots/env.sops.json".into(),
        }),
        policy_key: KEY.into(),
        last_issued_at: None,
        last_fetch_at: None,
        joined_at: "2026-10-22T09:00:00.000Z".into(),
        member: Default::default(),
        pending_request: None,
    }
}

fn row(scope: &str, id: &str, name: &str) -> WorkspaceRow {
    WorkspaceRow {
        scope: scope.into(),
        id: id.into(),
        name: name.into(),
        use_rules: None,
        created_at: "2026-10-30T09:00:00Z".into(),
    }
}

fn team(name: &str, org: Option<&str>, workspace: &str) -> TeamSpec {
    TeamSpec {
        name: name.into(),
        description: String::new(),
        members: Vec::new(),
        org_id: org.map(str::to_string),
        workspace_id: workspace.into(),
    }
}

fn ids(rows: &[WorkspaceRow]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

#[tokio::test]
async fn personal_has_its_default_workspace_after_migration() {
    let (_dir, store) = temp_store().await;
    let all = store.list_workspaces("personal").await.unwrap();
    assert_eq!(ids(&all), ["default"]);
    assert_eq!(all[0].name, "Default");
    assert!(all[0].use_rules.is_none());
    assert!(all[0].created_at.ends_with('Z'));
    assert!(store.workspace("personal", "nope").await.unwrap().is_none());
}

#[tokio::test]
async fn add_list_and_rename_a_workspace() {
    let (_dir, store) = temp_store().await;
    let mut pay = row("personal", "payments", "Payments");
    pay.use_rules = Some(UseRules {
        allow: vec!["PAY_*".into()],
        deny: vec!["PAY_ROOT".into()],
    });
    store.add_workspace(&pay).await.unwrap();
    store
        .add_workspace(&row("personal", "api", "API"))
        .await
        .unwrap();
    let all = store.list_workspaces("personal").await.unwrap();
    assert_eq!(ids(&all), ["default", "api", "payments"]);
    assert_eq!(all[2], pay);

    let err = store.add_workspace(&pay).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");

    store
        .rename_workspace("personal", "payments", "Billing")
        .await
        .unwrap();
    let renamed = store
        .workspace("personal", "payments")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.name, "Billing");
    assert_eq!(renamed.use_rules, pay.use_rules);
    let err = store
        .rename_workspace("personal", "ghost", "X")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn delete_moves_dots_and_teams_to_default_and_refuses_default() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store
        .add_workspace(&row("personal", "payments", "Payments"))
        .await
        .unwrap();
    store
        .add_workspace(&row("acme-eng", "payments", "Payments"))
        .await
        .unwrap();
    let mine = store
        .create_dot(&spec("mine"), None, "payments")
        .await
        .unwrap();
    let theirs = store
        .create_dot(&spec("theirs"), Some("acme-eng"), "payments")
        .await
        .unwrap();
    assert_eq!(mine.workspace_id, "payments");
    let my_team = store
        .create_team(&team("mine", None, "payments"))
        .await
        .unwrap();
    assert_eq!(my_team.workspace_id, "payments");
    let their_team = store
        .create_team(&team("theirs", Some("acme-eng"), "payments"))
        .await
        .unwrap();

    store
        .delete_workspace("personal", "payments")
        .await
        .unwrap();

    assert!(store
        .workspace("personal", "payments")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        store.get_dot(&mine.id).await.unwrap().workspace_id,
        "default"
    );
    assert_eq!(
        store.get_team(&my_team.id).await.unwrap().workspace_id,
        "default"
    );
    // The org's workspace of the same id, and what is in it, are untouched.
    assert!(store
        .workspace("acme-eng", "payments")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store.get_dot(&theirs.id).await.unwrap().workspace_id,
        "payments"
    );
    assert_eq!(
        store.get_team(&their_team.id).await.unwrap().workspace_id,
        "payments"
    );

    match store
        .delete_workspace("personal", "default")
        .await
        .unwrap_err()
    {
        Error::Invalid(m) => assert_eq!(m, "The Default workspace can't be deleted."),
        e => panic!("{e:?}"),
    }
    assert!(store
        .workspace("personal", "default")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn set_dot_workspace_moves_one_dot() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    store.set_dot_workspace(&dot.id, "api").await.unwrap();
    assert_eq!(store.get_dot(&dot.id).await.unwrap().workspace_id, "api");
    let err = store.set_dot_workspace("ghost", "api").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn mirror_replaces_the_orgs_rows_and_moves_vanished_workspaces_dots() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store
        .add_workspace(&row("acme-eng", "payments", "Payments"))
        .await
        .unwrap();
    store
        .add_workspace(&row("acme-eng", "api", "API"))
        .await
        .unwrap();
    let pay = store
        .create_dot(&spec("pay"), Some("acme-eng"), "payments")
        .await
        .unwrap();
    let api = store
        .create_dot(&spec("api"), Some("acme-eng"), "api")
        .await
        .unwrap();
    let pay_team = store
        .create_team(&team("pay", Some("acme-eng"), "payments"))
        .await
        .unwrap();

    let rules = UseRules {
        allow: vec!["API_*".into()],
        deny: vec![],
    };
    let roster = vec![
        Workspace {
            id: "default".into(),
            name: "Everyone".into(),
            use_rules: None,
        },
        Workspace {
            id: "api".into(),
            name: "API platform".into(),
            use_rules: Some(rules.clone()),
        },
        Workspace {
            id: "ops".into(),
            name: "Ops".into(),
            use_rules: None,
        },
    ];
    let vanished = store.mirror_workspaces("acme-eng", &roster).await.unwrap();
    assert_eq!(vanished, ["payments"]);

    let all = store.list_workspaces("acme-eng").await.unwrap();
    assert_eq!(ids(&all), ["default", "api", "ops"]);
    assert_eq!(all[0].name, "Everyone");
    assert_eq!(all[1].name, "API platform");
    assert_eq!(all[1].use_rules, Some(rules));
    assert_eq!(
        store.get_dot(&pay.id).await.unwrap().workspace_id,
        "default"
    );
    assert_eq!(store.get_dot(&api.id).await.unwrap().workspace_id, "api");
    assert_eq!(
        store.get_team(&pay_team.id).await.unwrap().workspace_id,
        "default"
    );
    // Personal is not touched.
    assert_eq!(
        ids(&store.list_workspaces("personal").await.unwrap()),
        ["default"]
    );
}

#[tokio::test]
async fn an_org_has_a_default_workspace_until_it_is_removed() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    let all = store.list_workspaces("acme-eng").await.unwrap();
    assert_eq!(ids(&all), ["default"]);
    store
        .add_workspace(&row("acme-eng", "api", "API"))
        .await
        .unwrap();

    store.remove_org("acme-eng").await.unwrap();
    assert!(store.list_workspaces("acme-eng").await.unwrap().is_empty());
    assert_eq!(
        ids(&store.list_workspaces("personal").await.unwrap()),
        ["default"]
    );
}

#[test]
fn a_team_spec_without_a_workspace_is_in_default() {
    let spec: TeamSpec = serde_json::from_str(r#"{"name":"Ops"}"#).unwrap();
    assert_eq!(spec.workspace_id, "default");
}

#[test]
fn workspace_ids_come_from_names() {
    assert_eq!(workspace_id_for("Payments Team", &[]), "payments-team");
    let taken = vec!["payments-team".to_string()];
    assert_eq!(workspace_id_for("Payments Team", &taken), "payments-team-2");
    assert_eq!(workspace_id_for("Default", &[]), "default-2");
    assert_eq!(workspace_id_for("***", &[]), "workspace");
    let two = vec!["api".to_string(), "api-2".to_string()];
    assert_eq!(workspace_id_for("API!", &two), "api-3");

    let long = workspace_id_for("Platform Reliability Engineering Team", &[]);
    assert!(long.len() <= 32, "{long}");
    assert_eq!(long, "platform-reliability-engineering");
    let again = workspace_id_for(
        "Platform Reliability Engineering Team",
        std::slice::from_ref(&long),
    );
    assert!(again.len() <= 32, "{again}");
    assert_eq!(again, "platform-reliability-engineeri-2");
    // Cutting at a dash leaves no dash at the end.
    assert_eq!(
        workspace_id_for("abcdefghijklmnopqrstuvwxyzabcde fgh", &[]),
        "abcdefghijklmnopqrstuvwxyzabcde"
    );
}

#[tokio::test]
async fn removing_an_org_puts_its_dots_and_teams_in_default() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store
        .add_workspace(&row("acme-eng", "payments", "Payments"))
        .await
        .unwrap();
    store
        .add_workspace(&row("personal", "payments", "Payments"))
        .await
        .unwrap();
    let theirs = store
        .create_dot(&spec("theirs"), Some("acme-eng"), "payments")
        .await
        .unwrap();
    let mine = store
        .create_dot(&spec("mine"), None, "payments")
        .await
        .unwrap();
    let crew = store
        .create_team(&team("crew", Some("acme-eng"), "payments"))
        .await
        .unwrap();

    store.remove_org("acme-eng").await.unwrap();
    let left = store.get_dot(&theirs.id).await.unwrap();
    assert_eq!(left.workspace_id, "default");
    // It keeps its org (§6.4): joining again re-attaches it.
    assert_eq!(left.org_id.as_deref(), Some("acme-eng"));
    assert_eq!(
        store.get_team(&crew.id).await.unwrap().workspace_id,
        "default"
    );
    // Personal's dots are not touched.
    assert_eq!(
        store.get_dot(&mine.id).await.unwrap().workspace_id,
        "payments"
    );
}

#[tokio::test]
async fn mirror_moves_dots_in_a_workspace_it_never_saw() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    // Placed in a workspace the org's rows never had (left and joined again, say).
    let lost = store
        .create_dot(&spec("lost"), Some("acme-eng"), "ghost")
        .await
        .unwrap();
    let crew = store
        .create_team(&team("crew", Some("acme-eng"), "ghost"))
        .await
        .unwrap();
    let kept = store
        .create_dot(&spec("kept"), Some("acme-eng"), "api")
        .await
        .unwrap();
    let mine = store
        .create_dot(&spec("mine"), None, "ghost")
        .await
        .unwrap();
    let roster = vec![Workspace {
        id: "api".into(),
        name: "API".into(),
        use_rules: None,
    }];
    store.mirror_workspaces("acme-eng", &roster).await.unwrap();
    assert_eq!(
        store.get_dot(&lost.id).await.unwrap().workspace_id,
        "default"
    );
    assert_eq!(
        store.get_team(&crew.id).await.unwrap().workspace_id,
        "default"
    );
    assert_eq!(store.get_dot(&kept.id).await.unwrap().workspace_id, "api");
    // Another account's dots are not touched.
    assert_eq!(store.get_dot(&mine.id).await.unwrap().workspace_id, "ghost");
}

/// Migration 0023 on a database with a joined org, a team and dots made before it: Personal
/// and the org get a Default workspace, and every dot and team is in it.
#[tokio::test]
async fn migration_0023_puts_what_was_there_in_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 23)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO orgs (id, name, source, policy_key, joined_at) VALUES ('acme-eng', \
         'Acme Engineering', ?, ?, '2026-10-22T09:00:00.000Z')",
    )
    .bind(serde_json::to_string(&acme().source).unwrap())
    .bind(KEY)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO teams (id, name, description, created_at, updated_at, org_id) VALUES \
         ('T1', 'crew', '', '2026-10-01T00:00:00.000Z', '2026-10-01T00:00:00.000Z', 'acme-eng')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let policy = serde_json::to_string(&dots_core::policy::Policy::preset(
        dots_core::policy::Preset::Sandboxed,
    ))
    .unwrap();
    for (id, org) in [("D1", None), ("D2", Some("acme-eng"))] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, uses, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at, org_id) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', \
             'C:/tmp/repo', 'worktree', ?, ?, '{}', 40, 1800, 240, \
             '2026-10-01T00:00:00.000Z', '2026-10-01T00:00:00.000Z', ?)",
        )
        .bind(id)
        .bind(format!("dot-{id}"))
        .bind(format!("token-{id}"))
        .bind(&policy)
        .bind(org)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        ids(&store.list_workspaces("personal").await.unwrap()),
        ["default"]
    );
    assert_eq!(
        ids(&store.list_workspaces("acme-eng").await.unwrap()),
        ["default"]
    );
    for id in ["D1", "D2"] {
        assert_eq!(store.get_dot(id).await.unwrap().workspace_id, "default");
    }
    assert_eq!(
        store.get_dot("D2").await.unwrap().org_id.as_deref(),
        Some("acme-eng")
    );
    assert_eq!(store.get_team("T1").await.unwrap().workspace_id, "default");
    store.close().await;
}

mod common;

use common::{init_repo, spec, temp_store};
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::model::{MemberSpec, ProjectSpec, TeamSpec, WorkflowSpec};
use dots_core::store::Store;
use dots_core::teams::Teams;
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::Error;

async fn dot(store: &Store, name: &str) -> String {
    store.create_dot(&spec(name), None).await.unwrap().id
}

fn team(name: &str, members: &[&str]) -> TeamSpec {
    TeamSpec {
        org_id: None,
        name: name.into(),
        description: "Runs Dots.".into(),
        members: members
            .iter()
            .map(|id| MemberSpec {
                dot_id: id.to_string(),
                role: "helper".into(),
            })
            .collect(),
    }
}

fn flow(name: &str, member: &str) -> WorkflowSpec {
    WorkflowSpec {
        name: name.into(),
        definition: Definition {
            trigger: Trigger::Manual,
            steps: vec![Step {
                id: "a".into(),
                name: "Work".into(),
                kind: StepKind::Agent {
                    member: member.into(),
                    job: "Fix it.".into(),
                },
            }],
            budget: Budget::default(),
        },
        enabled: true,
    }
}

fn project(name: &str, repo: &std::path::Path) -> ProjectSpec {
    ProjectSpec {
        name: name.into(),
        repo_path: repo.to_string_lossy().to_string(),
        default_branch: None,
        remote: "origin".into(),
    }
}

#[tokio::test]
async fn a_team_keeps_its_members_in_order_with_names_and_roles() {
    let (_dir, store) = temp_store().await;
    let helper = dot(&store, "helper").await;
    let dev = dot(&store, "dev").await;
    let teams = Teams::new(store.clone(), new_bus());
    let made = teams
        .create(&team("Dots management", &[&dev, &helper]))
        .await
        .unwrap();
    assert_eq!(made.name, "Dots management");
    let names: Vec<_> = made.members.iter().map(|m| m.dot_name.as_str()).collect();
    assert_eq!(names, ["dev", "helper"]);
    assert_eq!(made.members[0].role, "helper");
    assert_eq!(teams.list().await.unwrap(), vec![made.clone()]);

    let changed = teams
        .update(&made.id, &team("Dots crew", &[&helper]))
        .await
        .unwrap();
    assert_eq!(changed.name, "Dots crew");
    assert_eq!(changed.members.len(), 1);
}

#[tokio::test]
async fn team_rules_and_conflicts() {
    let (_dir, store) = temp_store().await;
    let helper = dot(&store, "helper").await;
    let teams = Teams::new(store.clone(), new_bus());
    teams.create(&team("A", &[&helper])).await.unwrap();
    assert!(matches!(
        teams.create(&team("A", &[])).await,
        Err(Error::Conflict(m)) if m == "a team named \"A\" already exists"
    ));
    assert!(matches!(
        teams.create(&team("a", &[])).await,
        Err(Error::Conflict(m)) if m == "a team named \"a\" already exists"
    ));
    assert!(matches!(
        teams.create(&team(" ", &[])).await,
        Err(Error::Invalid(m)) if m == "a team name must be 1 to 64 characters"
    ));
    assert!(matches!(
        teams.create(&team("B", &[&helper, &helper])).await,
        Err(Error::Invalid(m)) if m == format!("{helper} is in the team twice")
    ));
    assert!(matches!(
        teams.create(&team("C", &["nobody"])).await,
        Err(Error::Invalid(m)) if m == "a member is not a dot"
    ));
    let names: Vec<_> = teams
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["A"]);
}

#[tokio::test]
async fn a_deleted_dot_leaves_its_teams_and_its_workflows_get_a_problem() {
    let (_dir, store) = temp_store().await;
    let helper = dot(&store, "helper").await;
    let dev = dot(&store, "dev").await;
    let teams = Teams::new(store.clone(), new_bus());
    let t = teams.create(&team("T", &[&helper, &dev])).await.unwrap();
    let w = teams
        .create_workflow(&t.id, &flow("Fix", &dev))
        .await
        .unwrap();
    assert_eq!(w.problem, None);
    store.delete_dot(&dev).await.unwrap();
    assert_eq!(teams.get(&t.id).await.unwrap().members.len(), 1);
    let w = teams.get_workflow(&w.id).await.unwrap();
    assert_eq!(
        w.problem,
        Some(format!("step \"Work\": {dev} is not a member of the team"))
    );
}

#[tokio::test]
async fn workflows_are_validated_against_the_team() {
    let (_dir, store) = temp_store().await;
    let helper = dot(&store, "helper").await;
    let dev = dot(&store, "dev").await;
    let teams = Teams::new(store.clone(), new_bus());
    let t = teams.create(&team("T", &[&helper])).await.unwrap();
    assert!(matches!(
        teams.create_workflow(&t.id, &flow("Fix", &dev)).await,
        Err(Error::Invalid(m)) if m == format!("step \"Work\": {dev} is not a member of the team")
    ));
    let w = teams
        .create_workflow(&t.id, &flow("Fix", &helper))
        .await
        .unwrap();
    assert!(matches!(
        teams.create_workflow(&t.id, &flow("Fix", &helper)).await,
        Err(Error::Conflict(m)) if m == "the team has a workflow named \"Fix\""
    ));
    let mut off = flow("Fix later", &helper);
    off.enabled = false;
    let w2 = teams.update_workflow(&w.id, &off).await.unwrap();
    assert_eq!(w2.spec.name, "Fix later");
    assert!(!w2.spec.enabled);
    assert_eq!(teams.list_workflows(&t.id).await.unwrap(), vec![w2]);
    teams.delete_workflow(&w.id).await.unwrap();
    assert!(teams.list_workflows(&t.id).await.unwrap().is_empty());
    teams.delete(&t.id).await.unwrap();
    assert!(matches!(teams.get(&t.id).await, Err(Error::NotFound(_))));
}

#[tokio::test]
async fn a_project_is_a_git_repo_with_safe_git_names() {
    let (dir, store) = temp_store().await;
    let teams = Teams::new(store.clone(), new_bus());
    let t = teams.create(&team("T", &[])).await.unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let p = teams
        .add_project(&t.id, &project("Dots", &repo))
        .await
        .unwrap();
    assert_eq!(p.spec.remote, "origin");
    assert_eq!(teams.get(&t.id).await.unwrap().projects, vec![p.clone()]);

    let plain = dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        teams.add_project(&t.id, &project("Plain", &plain)).await,
        Err(Error::Invalid(m)) if m.ends_with("is not a git repository")
    ));
    assert!(matches!(
        teams.add_project(&t.id, &project("Dots", &repo)).await,
        Err(Error::Conflict(m)) if m == "the team has a project named \"Dots\""
    ));
    let mut bad = project("Other", &repo);
    bad.remote = "--upload-pack=x".into();
    assert!(matches!(
        teams.add_project(&t.id, &bad).await,
        Err(Error::Invalid(m)) if m.starts_with("the remote must be")
    ));
    let mut branch = project("Dots", &repo);
    branch.default_branch = Some("main".into());
    let p2 = teams.update_project(&p.id, &branch).await.unwrap();
    assert_eq!(p2.spec.default_branch.as_deref(), Some("main"));
    teams.delete_project(&p.id).await.unwrap();
    assert!(teams.get(&t.id).await.unwrap().projects.is_empty());
}

#[tokio::test]
async fn each_change_is_announced() {
    let (_dir, store) = temp_store().await;
    let helper = dot(&store, "helper").await;
    let bus = new_bus();
    let mut events = bus.subscribe();
    let teams = Teams::new(store.clone(), bus);
    let t = teams.create(&team("T", &[&helper])).await.unwrap();
    let w = teams
        .create_workflow(&t.id, &flow("Fix", &helper))
        .await
        .unwrap();
    teams.delete_workflow(&w.id).await.unwrap();
    teams.delete(&t.id).await.unwrap();
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        seen.push(match e {
            RuntimeEvent::TeamUpdated { team_id } => format!("team {}", team_id == t.id),
            RuntimeEvent::WorkflowUpdated { workflow_id, .. } => {
                format!("workflow {}", workflow_id == w.id)
            }
            other => format!("{other:?}"),
        });
    }
    assert_eq!(
        seen,
        ["team true", "workflow true", "workflow true", "team true"]
    );
}

#[tokio::test]
async fn project_and_workflow_names_are_unique_without_case() {
    let (dir, store) = temp_store().await;
    let helper = dot(&store, "helper").await;
    let teams = Teams::new(store.clone(), new_bus());
    let t = teams.create(&team("T", &[&helper])).await.unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    teams
        .add_project(&t.id, &project("Dots", &repo))
        .await
        .unwrap();
    assert!(matches!(
        teams.add_project(&t.id, &project("dots", &repo)).await,
        Err(Error::Conflict(m)) if m == "the team has a project named \"dots\""
    ));
    teams
        .create_workflow(&t.id, &flow("Fix", &helper))
        .await
        .unwrap();
    assert!(matches!(
        teams.create_workflow(&t.id, &flow("fix", &helper)).await,
        Err(Error::Conflict(m)) if m == "the team has a workflow named \"fix\""
    ));
}

fn acme_team(name: &str, members: &[&str]) -> TeamSpec {
    TeamSpec {
        org_id: Some(common::orgs::ACME.into()),
        ..team(name, members)
    }
}

async fn acme_dot(store: &Store, name: &str) -> String {
    store
        .create_dot(&spec(name), Some(common::orgs::ACME))
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn team_takes_spec_org() {
    let (_dir, store) = temp_store().await;
    let a = acme_dot(&store, "a").await;
    let t = store.create_team(&acme_team("crew", &[&a])).await.unwrap();
    assert_eq!(t.org_id.as_deref(), Some(common::orgs::ACME));
    let personal = store.create_team(&team("solo", &[])).await.unwrap();
    assert_eq!(personal.org_id, None);
}

#[tokio::test]
async fn team_refuses_dot_of_other_account() {
    let (_dir, store) = temp_store().await;
    let p = dot(&store, "pat").await;
    let a = acme_dot(&store, "amy").await;
    let msg = "dot 'pat' is in another account than the team";
    match store.create_team(&acme_team("crew", &[&p])).await {
        Err(Error::Invalid(m)) => assert_eq!(m, msg),
        other => panic!("{other:?}"),
    }
    // A personal team refuses an Acme dot too.
    match store.create_team(&team("solo", &[&a])).await {
        Err(Error::Invalid(m)) => assert_eq!(m, "dot 'amy' is in another account than the team"),
        other => panic!("{other:?}"),
    }
    let t = store.create_team(&acme_team("crew", &[&a])).await.unwrap();
    match store
        .update_team(&t.id, &acme_team("crew", &[&a, &p]))
        .await
    {
        Err(Error::Invalid(m)) => assert_eq!(m, msg),
        other => panic!("{other:?}"),
    }
    // The refused update changed nothing, and the spec's org_id cannot move the team.
    let moved = store.update_team(&t.id, &team("crew", &[&a])).await;
    assert_eq!(moved.unwrap().org_id.as_deref(), Some(common::orgs::ACME));
}

#[tokio::test]
async fn backfill_assigns_single_org_teams() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("dots.db");
    let store = Store::open(&db).await.unwrap();
    let (a, b, c) = (
        dot(&store, "a").await,
        dot(&store, "b").await,
        dot(&store, "c").await,
    );
    let all = store.create_team(&team("all", &[&a, &b])).await.unwrap();
    let mixed = store.create_team(&team("mixed", &[&b, &c])).await.unwrap();
    let empty = store.create_team(&team("empty", &[])).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db.display()))
        .await
        .unwrap();
    for id in [&a, &b] {
        sqlx::query("UPDATE dots SET org_id = ? WHERE id = ?")
            .bind(common::orgs::ACME)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE teams SET org_id = NULL")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(dots_core::store::TEAM_ORG_BACKFILL_SQL)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        store.get_team(&all.id).await.unwrap().org_id.as_deref(),
        Some(common::orgs::ACME)
    );
    assert_eq!(store.get_team(&mixed.id).await.unwrap().org_id, None);
    assert_eq!(store.get_team(&empty.id).await.unwrap().org_id, None);
}

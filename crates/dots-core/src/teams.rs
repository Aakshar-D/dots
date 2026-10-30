//! Teams (teams and workflows spec sections 3, 4 and 11): named groups of dots with their
//! projects and workflows. `Teams` makes, changes and deletes them and announces each change.

use std::path::Path;

use crate::accounts::Permission;
use crate::events::{Bus, RuntimeEvent};
use crate::model::{Project, ProjectSpec, Team, TeamSpec, Workflow, WorkflowSpec};
use crate::store::Store;
use crate::vault::org::Orgs;
use crate::workspace::git;
use crate::{Error, Result};

/// Cheap to clone.
#[derive(Clone)]
pub struct Teams {
    store: Store,
    bus: Bus,
    /// The orgs a team's account role is read from (members spec §6.3); without them, an
    /// account's team acts as `legacy`.
    orgs: Option<Orgs>,
}

/// Refuses a project folder that is not a git repo.
pub(crate) async fn check_repo(repo_path: &str) -> Result<()> {
    let path = Path::new(repo_path.trim());
    let inside = if path.is_dir() {
        git(path, &["rev-parse", "--is-inside-work-tree"])
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };
    if inside != "true" {
        return Err(Error::Invalid(format!(
            "{} is not a git repository",
            path.display()
        )));
    }
    Ok(())
}

impl Teams {
    pub fn new(store: Store, bus: Bus) -> Self {
        Self {
            store,
            bus,
            orgs: None,
        }
    }

    /// The orgs whose roles gate changes to an account's teams.
    pub fn with_orgs(mut self, orgs: Orgs) -> Self {
        self.orgs = Some(orgs);
        self
    }

    /// `Ok` when this PC may change dots and teams in the account `org_id` (members spec §6.3).
    async fn require_edit(&self, org_id: Option<&str>) -> Result<()> {
        crate::accounts::require(self.orgs.as_ref(), org_id, Permission::EditDots).await
    }

    /// `require_edit` in the account of the team `team_id`.
    async fn require_edit_team(&self, team_id: &str) -> Result<()> {
        let team = self.store.get_team(team_id).await?;
        self.require_edit(team.org_id.as_deref()).await
    }

    fn team_updated(&self, team_id: &str) {
        let _ = self.bus.send(RuntimeEvent::TeamUpdated {
            team_id: team_id.to_string(),
        });
    }

    fn workflow_updated(&self, team_id: &str, workflow_id: &str) {
        let _ = self.bus.send(RuntimeEvent::WorkflowUpdated {
            team_id: team_id.to_string(),
            workflow_id: workflow_id.to_string(),
        });
    }

    pub async fn list(&self) -> Result<Vec<Team>> {
        self.store.list_teams().await
    }

    pub async fn get(&self, id: &str) -> Result<Team> {
        self.store.get_team(id).await
    }

    pub async fn create(&self, spec: &TeamSpec) -> Result<Team> {
        self.require_edit(spec.org_id.as_deref()).await?;
        let team = self.store.create_team(spec).await?;
        self.team_updated(&team.id);
        Ok(team)
    }

    /// Replaces the team's name, description and members. A workflow that names a member
    /// removed here keeps its definition and gets a `problem`.
    pub async fn update(&self, id: &str, spec: &TeamSpec) -> Result<Team> {
        self.require_edit_team(id).await?;
        let team = self.store.update_team(id, spec).await?;
        self.team_updated(id);
        Ok(team)
    }

    /// Deletes the team, its projects and workflows. Refused while it has workflow runs in
    /// flight: `Workflows::delete_team` stops them first.
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.require_edit_team(id).await?;
        if self.store.team_has_runs_in_flight(id).await? {
            return Err(Error::Conflict(
                "the team has workflow runs in flight; stop them first".into(),
            ));
        }
        self.store.delete_team(id).await?;
        self.team_updated(id);
        Ok(())
    }

    pub async fn add_project(&self, team_id: &str, spec: &ProjectSpec) -> Result<Project> {
        self.require_edit_team(team_id).await?;
        spec.validate()?;
        check_repo(&spec.repo_path).await?;
        let project = self.store.add_project(team_id, spec).await?;
        self.team_updated(team_id);
        Ok(project)
    }

    pub async fn update_project(&self, id: &str, spec: &ProjectSpec) -> Result<Project> {
        self.require_edit_team(&self.store.get_project(id).await?.team_id)
            .await?;
        spec.validate()?;
        check_repo(&spec.repo_path).await?;
        let project = self.store.update_project(id, spec).await?;
        self.team_updated(&project.team_id);
        Ok(project)
    }

    pub async fn delete_project(&self, id: &str) -> Result<()> {
        self.require_edit_team(&self.store.get_project(id).await?.team_id)
            .await?;
        let project = self.store.delete_project(id).await?;
        self.team_updated(&project.team_id);
        Ok(())
    }

    pub async fn list_workflows(&self, team_id: &str) -> Result<Vec<Workflow>> {
        self.store.get_team(team_id).await?;
        self.store.list_workflows(team_id).await
    }

    pub async fn get_workflow(&self, id: &str) -> Result<Workflow> {
        self.store.get_workflow(id).await
    }

    pub async fn create_workflow(&self, team_id: &str, spec: &WorkflowSpec) -> Result<Workflow> {
        self.require_edit_team(team_id).await?;
        let workflow = self.store.create_workflow(team_id, spec).await?;
        self.workflow_updated(team_id, &workflow.id);
        Ok(workflow)
    }

    pub async fn update_workflow(&self, id: &str, spec: &WorkflowSpec) -> Result<Workflow> {
        self.require_edit_team(&self.store.get_workflow(id).await?.team_id)
            .await?;
        let workflow = self.store.update_workflow(id, spec).await?;
        self.workflow_updated(&workflow.team_id, id);
        Ok(workflow)
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<()> {
        self.require_edit_team(&self.store.get_workflow(id).await?.team_id)
            .await?;
        let workflow = self.store.delete_workflow(id).await?;
        self.workflow_updated(&workflow.team_id, id);
        Ok(())
    }
}

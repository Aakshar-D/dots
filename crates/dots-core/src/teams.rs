//! Teams (teams and workflows spec sections 3, 4 and 11): named groups of dots with their
//! projects and workflows. `Teams` makes, changes and deletes them and announces each change.

use std::path::Path;

use crate::accounts::Permission;
use crate::events::{Bus, RuntimeEvent};
use crate::model::{Project, ProjectSpec, Team, TeamSpec, Workflow, WorkflowSpec};
use crate::store::Store;
use crate::vault::org::Orgs;
use crate::workflow::WorkflowProblem;
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

/// The team's project named `name`, compared without case and surrounding spaces, as a chat's
/// `start_workflow` and a branch on project match it: how a schedule or webhook trigger's
/// project name, or a webhook body's, is found.
pub fn project_named<'a>(team: &'a Team, name: &str) -> Option<&'a Project> {
    let wanted = name.trim();
    team.projects
        .iter()
        .find(|p| p.spec.name.trim().eq_ignore_ascii_case(wanted))
}

/// Refuses a project folder that is not a git repo.
pub(crate) async fn check_repo(repo_path: &str) -> Result<()> {
    let path = Path::new(repo_path.trim());
    if !crate::runtime::is_git_repo(path).await {
        return Err(Error::Invalid(format!(
            "{} is not a git repository",
            path.display()
        )));
    }
    Ok(())
}

/// Refuses a default branch given with a remote in front, such as `origin/main`: worktrees and
/// deploys look the name up as a branch and add the remote themselves, so it would name a ref
/// that does not exist. The remotes are the repo's (`git remote`) and the project's own, which
/// counts even before the repo has it. Called after `check_repo`.
async fn check_default_branch(spec: &ProjectSpec) -> Result<()> {
    let Some(branch) = spec.default_branch.as_deref().filter(|b| b.contains('/')) else {
        return Ok(());
    };
    let listed = git(Path::new(spec.repo_path.trim()), &["remote"])
        .await
        .unwrap_or_default();
    let remotes = listed.lines().map(str::trim).chain([spec.remote.as_str()]);
    for remote in remotes.filter(|r| !r.is_empty()) {
        if let Some(name) = branch.strip_prefix(&format!("{remote}/")) {
            return Err(Error::Invalid(format!(
                "give the default branch without the remote: \"{name}\", not \"{branch}\""
            )));
        }
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

    /// `Ok` when this PC may change dots and teams in the workspace `workspace_id` of the
    /// account `org_id` (members spec §6.3).
    async fn require_edit(&self, org_id: Option<&str>, workspace_id: &str) -> Result<()> {
        crate::accounts::require_in(
            self.orgs.as_ref(),
            org_id,
            workspace_id,
            Permission::EditDots,
        )
        .await
    }

    /// `require_edit` in the account and workspace of the team `team_id`.
    async fn require_edit_team(&self, team_id: &str) -> Result<()> {
        let team = self.store.get_team(team_id).await?;
        crate::accounts::require_team(self.orgs.as_ref(), &team, Permission::EditDots).await
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

    /// Announces each workflow of the team: its members changed, so its `problem` may have.
    /// Called after the change, which stands whatever this finds: a failed read is logged.
    async fn workflows_updated(&self, team_id: &str) {
        match self.store.list_workflows(team_id).await {
            Ok(workflows) => workflows
                .iter()
                .for_each(|w| self.workflow_updated(team_id, &w.id)),
            Err(e) => {
                tracing::warn!(team = %team_id, "announcing the team's workflows failed: {e}")
            }
        }
    }

    /// Announces the teams `team_ids` of a dot that was deleted, read before it was: it left
    /// them, so each team and each of its workflows changed.
    pub(crate) async fn dot_deleted(&self, team_ids: &[String]) {
        for team_id in team_ids {
            self.team_updated(team_id);
            self.workflows_updated(team_id).await;
        }
    }

    /// Announces the teams of the dot `dot_id`, which was renamed: a team shows its members by
    /// name.
    pub(crate) async fn dot_renamed(&self, dot_id: &str) {
        match self.store.teams_of_dot(dot_id).await {
            Ok(team_ids) => team_ids.iter().for_each(|id| self.team_updated(id)),
            Err(e) => tracing::warn!(dot = %dot_id, "announcing the sol's teams failed: {e}"),
        }
    }

    pub async fn list(&self) -> Result<Vec<Team>> {
        self.store.list_teams().await
    }

    pub async fn get(&self, id: &str) -> Result<Team> {
        self.store.get_team(id).await
    }

    pub async fn create(&self, spec: &TeamSpec) -> Result<Team> {
        self.require_edit(spec.org_id.as_deref(), &spec.workspace_id)
            .await?;
        let scope = spec.org_id.as_deref().unwrap_or(crate::accounts::PERSONAL);
        if self
            .store
            .workspace(scope, &spec.workspace_id)
            .await?
            .is_none()
        {
            return Err(Error::NotFound(format!(
                "No workspace '{}' in {}.",
                spec.workspace_id,
                crate::accounts::account_label(self.orgs.as_ref(), spec.org_id.as_deref()).await
            )));
        }
        let team = self.store.create_team(spec).await?;
        self.team_updated(&team.id);
        Ok(team)
    }

    /// Replaces the team's name, description and members; its account and workspace stay. A workflow that names a member
    /// removed here keeps its definition and gets a `problem`. When the members change, each
    /// workflow of the team is announced too.
    pub async fn update(&self, id: &str, spec: &TeamSpec) -> Result<Team> {
        self.require_edit_team(id).await?;
        let mut before = self.store.team_member_ids(id).await?;
        let team = self.store.update_team(id, spec).await?;
        self.team_updated(id);
        // Which dots are members, not their order or roles: what a workflow's `problem` reads.
        let mut after: Vec<String> = team.members.iter().map(|m| m.dot_id.clone()).collect();
        before.sort();
        after.sort();
        if before != after {
            self.workflows_updated(id).await;
        }
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
        let workflows = self.store.list_workflows(id).await?;
        self.store.delete_team(id).await?;
        for workflow in &workflows {
            self.workflow_updated(id, &workflow.id);
        }
        self.team_updated(id);
        Ok(())
    }

    pub async fn add_project(&self, team_id: &str, spec: &ProjectSpec) -> Result<Project> {
        self.require_edit_team(team_id).await?;
        spec.validate()?;
        check_repo(&spec.repo_path).await?;
        check_default_branch(spec).await?;
        let project = self.store.add_project(team_id, spec).await?;
        self.team_updated(team_id);
        Ok(project)
    }

    pub async fn update_project(&self, id: &str, spec: &ProjectSpec) -> Result<Project> {
        self.require_edit_team(&self.store.get_project(id).await?.team_id)
            .await?;
        spec.validate()?;
        check_repo(&spec.repo_path).await?;
        check_default_branch(spec).await?;
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

    /// What saving `spec` as a workflow of the team `team_id` would be refused with, and the
    /// step it names, checked while the builder edits (section 5.8): the name, then the
    /// definition against the team's members now, by the rules a save uses
    /// (`WorkflowSpec::problem`). `None` when a save would pass them. Two refusals are left to
    /// the save: a name another workflow of the team has (which workflow is being edited is not
    /// known here), and a changed schedule with no slot ahead (read against the clock and the
    /// stored schedule at the save).
    pub async fn check_workflow(
        &self,
        team_id: &str,
        spec: &WorkflowSpec,
    ) -> Result<Option<WorkflowProblem>> {
        self.store.get_team(team_id).await?;
        Ok(spec.problem(&self.store.team_member_ids(team_id).await?))
    }

    pub async fn update_workflow(&self, id: &str, spec: &WorkflowSpec) -> Result<Workflow> {
        self.require_edit_team(&self.store.get_workflow(id).await?.team_id)
            .await?;
        let workflow = self.store.update_workflow(id, spec).await?;
        self.workflow_updated(&workflow.team_id, id);
        Ok(workflow)
    }

    /// Makes the workflow a new webhook token and returns it: the only time it is shown
    /// (section 5.1). The old token stops working at once.
    pub async fn regenerate_workflow_token(&self, id: &str) -> Result<String> {
        let team_id = self.store.get_workflow(id).await?.team_id;
        self.require_edit_team(&team_id).await?;
        let token = self.store.regenerate_workflow_token(id).await?;
        self.workflow_updated(&team_id, id);
        Ok(token)
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<()> {
        self.require_edit_team(&self.store.get_workflow(id).await?.team_id)
            .await?;
        let workflow = self.store.delete_workflow(id).await?;
        self.workflow_updated(&workflow.team_id, id);
        Ok(())
    }
}

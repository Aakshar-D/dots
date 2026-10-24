//! The rows of teams, their members and projects, and their workflows (teams spec section 4).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::Store;
use crate::model::{
    MemberSpec, Project, ProjectSpec, StartableWorkflow, Team, TeamMember, TeamSpec, Workflow,
    WorkflowSpec,
};
use crate::util::{new_id, now};
use crate::{Error, Result};

const PROJECT_COLUMNS: &str =
    "id, team_id, name, repo_path, default_branch, remote, created_at, updated_at";
const WORKFLOW_COLUMNS: &str = "id, team_id, name, definition, enabled, created_at, updated_at";

/// A unique name taken: `Conflict` with `taken`; a member that is not a dot: `Invalid`; any
/// other error as it is.
fn unique(e: sqlx::Error, taken: String) -> Error {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return Error::Conflict(taken);
        }
        if db.is_foreign_key_violation() {
            return Error::Invalid("a member is not a dot".into());
        }
    }
    e.into()
}

fn project_from_row(row: &SqliteRow) -> Result<Project> {
    Ok(Project {
        id: row.try_get("id")?,
        team_id: row.try_get("team_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        spec: ProjectSpec {
            name: row.try_get("name")?,
            repo_path: row.try_get("repo_path")?,
            default_branch: row.try_get("default_branch")?,
            remote: row.try_get("remote")?,
        },
    })
}

/// The workflow of `row`, with `problem` set against the team's `members`.
fn workflow_from_row(row: &SqliteRow, members: &[String]) -> Result<Workflow> {
    let spec = WorkflowSpec {
        name: row.try_get("name")?,
        definition: serde_json::from_str(&row.try_get::<String, _>("definition")?)?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
    };
    let problem = match spec.definition.validate(members) {
        Err(Error::Invalid(m)) => Some(m),
        Err(e) => Some(e.to_string()),
        Ok(()) => None,
    };
    Ok(Workflow {
        id: row.try_get("id")?,
        team_id: row.try_get("team_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        problem,
        spec,
    })
}

async fn insert_members(
    conn: &mut sqlx::SqliteConnection,
    team_id: &str,
    members: &[MemberSpec],
) -> Result<()> {
    for (i, m) in members.iter().enumerate() {
        sqlx::query(
            "INSERT INTO team_members (team_id, dot_id, role, position) VALUES (?, ?, ?, ?)",
        )
        .bind(team_id)
        .bind(&m.dot_id)
        .bind(m.role.trim())
        .bind(i as i64)
        .execute(&mut *conn)
        .await
        .map_err(|e| unique(e, format!("{} is in the team twice", m.dot_id)))?;
    }
    Ok(())
}

impl Store {
    pub async fn create_team(&self, spec: &TeamSpec) -> Result<Team> {
        spec.validate()?;
        let id = new_id();
        let ts = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO teams (id, name, description, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(spec.name.trim())
        .bind(spec.description.trim())
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            unique(
                e,
                format!("a team named {:?} already exists", spec.name.trim()),
            )
        })?;
        insert_members(&mut tx, &id, &spec.members).await?;
        tx.commit().await?;
        self.get_team(&id).await
    }

    /// Replaces the team's name, description and members.
    pub async fn update_team(&self, id: &str, spec: &TeamSpec) -> Result<Team> {
        spec.validate()?;
        let mut tx = self.pool.begin().await?;
        let res =
            sqlx::query("UPDATE teams SET name = ?, description = ?, updated_at = ? WHERE id = ?")
                .bind(spec.name.trim())
                .bind(spec.description.trim())
                .bind(now())
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    unique(
                        e,
                        format!("a team named {:?} already exists", spec.name.trim()),
                    )
                })?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("team {id}")));
        }
        sqlx::query("DELETE FROM team_members WHERE team_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        insert_members(&mut tx, id, &spec.members).await?;
        tx.commit().await?;
        self.get_team(id).await
    }

    pub async fn delete_team(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM teams WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("team {id}")));
        }
        Ok(())
    }

    pub async fn get_team(&self, id: &str) -> Result<Team> {
        let row = sqlx::query(
            "SELECT id, name, description, created_at, updated_at FROM teams WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("team {id}")))?;
        let members = sqlx::query(
            "SELECT m.dot_id, d.name AS dot_name, m.role FROM team_members m \
             JOIN dots d ON d.id = m.dot_id WHERE m.team_id = ? ORDER BY m.position",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|r| {
            Ok(TeamMember {
                dot_id: r.try_get("dot_id")?,
                dot_name: r.try_get("dot_name")?,
                role: r.try_get("role")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
        let projects = sqlx::query(&format!(
            "SELECT {PROJECT_COLUMNS} FROM team_projects WHERE team_id = ? ORDER BY name"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(project_from_row)
        .collect::<Result<Vec<_>>>()?;
        Ok(Team {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            members,
            projects,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }

    /// Every team, by name.
    pub async fn list_teams(&self) -> Result<Vec<Team>> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM teams ORDER BY name, id")
            .fetch_all(&self.pool)
            .await?;
        let mut teams = Vec::new();
        for id in ids {
            teams.push(self.get_team(&id).await?);
        }
        Ok(teams)
    }

    /// The dot ids of the team's members, in order.
    pub async fn team_member_ids(&self, team_id: &str) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT dot_id FROM team_members WHERE team_id = ? ORDER BY position",
        )
        .bind(team_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn add_project(&self, team_id: &str, spec: &ProjectSpec) -> Result<Project> {
        spec.validate()?;
        self.get_team(team_id).await?;
        let id = new_id();
        let ts = now();
        sqlx::query(
            "INSERT INTO team_projects (id, team_id, name, repo_path, default_branch, remote, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(team_id)
        .bind(spec.name.trim())
        .bind(spec.repo_path.trim())
        .bind(&spec.default_branch)
        .bind(&spec.remote)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            unique(
                e,
                format!("the team has a project named {:?}", spec.name.trim()),
            )
        })?;
        self.get_project(&id).await
    }

    pub async fn update_project(&self, id: &str, spec: &ProjectSpec) -> Result<Project> {
        spec.validate()?;
        let res = sqlx::query(
            "UPDATE team_projects SET name = ?, repo_path = ?, default_branch = ?, remote = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(spec.name.trim())
        .bind(spec.repo_path.trim())
        .bind(&spec.default_branch)
        .bind(&spec.remote)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            unique(
                e,
                format!("the team has a project named {:?}", spec.name.trim()),
            )
        })?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("project {id}")));
        }
        self.get_project(id).await
    }

    pub async fn delete_project(&self, id: &str) -> Result<Project> {
        let project = self.get_project(id).await?;
        sqlx::query("DELETE FROM team_projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(project)
    }

    pub async fn get_project(&self, id: &str) -> Result<Project> {
        let row = sqlx::query(&format!(
            "SELECT {PROJECT_COLUMNS} FROM team_projects WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("project {id}")))?;
        project_from_row(&row)
    }

    /// Creates a workflow of the team, validated against its members.
    pub async fn create_workflow(&self, team_id: &str, spec: &WorkflowSpec) -> Result<Workflow> {
        self.get_team(team_id).await?;
        spec.validate(&self.team_member_ids(team_id).await?)?;
        let id = new_id();
        let ts = now();
        sqlx::query(
            "INSERT INTO workflows (id, team_id, name, definition, enabled, created_at, \
             updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(team_id)
        .bind(spec.name.trim())
        .bind(serde_json::to_string(&spec.definition)?)
        .bind(spec.enabled as i64)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            unique(
                e,
                format!("the team has a workflow named {:?}", spec.name.trim()),
            )
        })?;
        self.get_workflow(&id).await
    }

    /// Replaces a workflow's name, definition and switch, validated against its team's members.
    pub async fn update_workflow(&self, id: &str, spec: &WorkflowSpec) -> Result<Workflow> {
        let team_id = self.get_workflow(id).await?.team_id;
        spec.validate(&self.team_member_ids(&team_id).await?)?;
        sqlx::query(
            "UPDATE workflows SET name = ?, definition = ?, enabled = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(spec.name.trim())
        .bind(serde_json::to_string(&spec.definition)?)
        .bind(spec.enabled as i64)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            unique(
                e,
                format!("the team has a workflow named {:?}", spec.name.trim()),
            )
        })?;
        self.get_workflow(id).await
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<Workflow> {
        let workflow = self.get_workflow(id).await?;
        sqlx::query("DELETE FROM workflows WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(workflow)
    }

    pub async fn get_workflow(&self, id: &str) -> Result<Workflow> {
        let row = sqlx::query(&format!(
            "SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("workflow {id}")))?;
        let team_id: String = row.try_get("team_id")?;
        workflow_from_row(&row, &self.team_member_ids(&team_id).await?)
    }

    /// The workflows `dot_id` may start from a chat (teams spec section 7), by team and name.
    pub async fn startable_workflows(&self, dot_id: &str) -> Result<Vec<StartableWorkflow>> {
        let mut out = Vec::new();
        for team_id in self.teams_of_dot(dot_id).await? {
            let team = self.get_team(&team_id).await?;
            for w in self.list_workflows(&team_id).await? {
                if w.spec.enabled
                    && w.problem.is_none()
                    && w.spec.definition.started_from_chat_by(dot_id)
                {
                    out.push(StartableWorkflow {
                        workflow_id: w.id,
                        workflow: w.spec.name,
                        team: team.name.clone(),
                        projects: team.projects.iter().map(|p| p.spec.name.clone()).collect(),
                    });
                }
            }
        }
        out.sort_by(|a, b| (&a.team, &a.workflow).cmp(&(&b.team, &b.workflow)));
        Ok(out)
    }

    /// The id of the project named `project` of the team of the workflow `workflow_id`.
    pub async fn get_team_project_id(&self, workflow_id: &str, project: &str) -> Result<String> {
        sqlx::query_scalar(
            "SELECT p.id FROM team_projects p JOIN workflows w ON w.team_id = p.team_id \
             WHERE w.id = ? AND p.name = ?",
        )
        .bind(workflow_id)
        .bind(project)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("project {project}")))
    }

    /// The teams `dot_id` is a member of, by id.
    pub async fn teams_of_dot(&self, dot_id: &str) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar(
                "SELECT team_id FROM team_members WHERE dot_id = ? ORDER BY team_id",
            )
            .bind(dot_id)
            .fetch_all(&self.pool)
            .await?,
        )
    }

    /// The team's workflows, by name.
    pub async fn list_workflows(&self, team_id: &str) -> Result<Vec<Workflow>> {
        let members = self.team_member_ids(team_id).await?;
        let rows = sqlx::query(&format!(
            "SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE team_id = ? ORDER BY name, id"
        ))
        .bind(team_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| workflow_from_row(r, &members))
            .collect()
    }
}

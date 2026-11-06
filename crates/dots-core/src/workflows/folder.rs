//! The folders "Open folder" opens for teams (teams spec sections 3 and 10.3): a project's repo
//! folder, and the worktree of a branch a deploy listed as ready. Each is looked up by id in the
//! store, so the UI never names a path.

use std::path::PathBuf;

use super::Workflows;
use crate::{Error, Result};

/// `path` when it is a folder now; `NotFound` as `Runtime::run_folder` says it otherwise.
fn existing(path: PathBuf) -> Result<PathBuf> {
    if !path.is_dir() {
        return Err(Error::NotFound(format!(
            "folder {} no longer exists",
            path.display()
        )));
    }
    Ok(path)
}

impl Workflows {
    /// The repo folder of the project `project_id`, for the team page's "Open folder".
    pub async fn project_folder(&self, project_id: &str) -> Result<PathBuf> {
        let project = self.store.get_project(project_id).await?;
        existing(PathBuf::from(project.spec.repo_path))
    }

    /// The worktree of `branch`, a branch a deploy step of the workflow run `id` listed as ready
    /// (mode `branch`, or any deploy of a test run): the run page's "Open folder". The folder is
    /// the one the step that made the branch worked in, read from the run's hand-offs. `Invalid`
    /// for a branch no deploy of the run listed as ready.
    pub async fn branch_folder(&self, id: &str, branch: &str) -> Result<PathBuf> {
        let wr = self.store.get_workflow_run(id).await?;
        let ready = wr.steps.iter().any(|s| {
            s.deploy_result
                .as_ref()
                .is_some_and(|d| d.ready && d.branches.iter().any(|b| b.branch == branch))
        });
        // The newest hand-off that carries the branch: a later attempt may have moved it.
        let path = wr
            .steps
            .iter()
            .rev()
            .filter_map(|s| s.handoff.as_ref())
            .flat_map(|h| &h.branches)
            .find(|b| b.branch == branch)
            .map(|b| PathBuf::from(&b.worktree_path));
        match (ready, path) {
            (true, Some(path)) => existing(path),
            _ => Err(Error::Invalid(format!(
                "workflow run {id} has no ready branch {branch}"
            ))),
        }
    }
}

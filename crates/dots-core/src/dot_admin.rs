//! The one path for creating, changing and deleting a dot (dots-make-dots design section 12).
//! The person's actions reach it through `Runtime::create_dot`, `update_dot` and `delete_dot`;
//! a run's tools reach it through the check-ins (`CheckIns::dot_admin`), on both engines. So a
//! dot a maker makes meets the checks a person's dot meets: a valid dot, a free name, Library
//! items that exist, the rights on this PC in the account and workspace, and the schedule,
//! scheduler and watcher reloads. What a made dot may hold (the maker's reach, section 6) and
//! how many a maker may make (section 10) are the caller's checks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use crate::accounts::{Access, Permission};
use crate::events::{Bus, RuntimeEvent};
use crate::followups::FollowUps;
use crate::model::{Dot, DotSpec, Made, WorkspaceMode};
use crate::runner::Runner;
use crate::runtime::{default_folder, lock, worktree_ready, Viewing, NOT_A_REPO};
use crate::scheduler::{Clock, Scheduler};
use crate::store::Store;
use crate::vault::org::Orgs;
use crate::vault::Vault;
use crate::{Error, Result};

/// Why a made dot that is to work in copies of a folder, as its maker does, is not made when
/// the folder is not ready for them: it would work in the folder itself, which its maker never
/// does.
pub const MADE_NOT_A_REPO: &str = "The new dot would work in copies of that folder, as its \
    maker does, but the folder is not the top of a git repository with a commit. Leave the \
    folder blank for a new one.";

/// What `DotAdmin::delete_rows` read of a dot before its row went, for `after_delete`.
pub(crate) struct Deleted {
    dot: Dot,
    had_row: bool,
    proposals: Vec<crate::apps::propose::AppProposal>,
    watchers: Vec<String>,
    teams: Vec<String>,
    attachments: Vec<String>,
    had_goals: bool,
}

/// What `DotAdmin` works with: the runtime's own parts, shared with it.
pub(crate) struct DotAdminParts {
    pub store: Store,
    pub bus: Bus,
    pub clock: Arc<dyn Clock>,
    /// Where a dot created without a folder gets one (`Config::new_dot_root`).
    pub new_dot_root: PathBuf,
    pub orgs: Orgs,
    pub vault: Vault,
    pub scheduler: Arc<Scheduler>,
    pub watchers: Arc<crate::watchers::Watchers>,
    pub follow_ups: Arc<FollowUps>,
    pub runner: Arc<Runner>,
    pub teams: crate::teams::Teams,
    pub apps: crate::apps::Apps,
    pub chat_files: crate::chat_files::ChatFiles,
    pub browsers: Arc<crate::browser::Browsers>,
    /// The dot being viewed (`Runtime::set_viewing`).
    pub viewing: Viewing,
}

/// Creates, changes and deletes dots, role checks included. Cheap to clone; the clones are one
/// service. The runtime makes it once, at start.
#[derive(Clone)]
pub struct DotAdmin {
    parts: Arc<DotAdminParts>,
}

/// A `DotAdmin` that does not keep it alive: what the check-ins hold, since the admin holds the
/// runner, which holds the check-ins.
pub(crate) struct WeakDotAdmin(Weak<DotAdminParts>);

impl WeakDotAdmin {
    /// The admin, while the runtime that made it exists.
    pub(crate) fn upgrade(&self) -> Option<DotAdmin> {
        self.0.upgrade().map(|parts| DotAdmin { parts })
    }
}

impl DotAdmin {
    pub(crate) fn new(parts: DotAdminParts) -> DotAdmin {
        DotAdmin {
            parts: Arc::new(parts),
        }
    }

    /// Where a dot created without a folder gets one (`Config::new_dot_root`).
    pub(crate) fn new_dot_root(&self) -> &Path {
        &self.parts.new_dot_root
    }

    pub(crate) fn downgrade(&self) -> WeakDotAdmin {
        WeakDotAdmin(Arc::downgrade(&self.parts))
    }

    /// Creates a dot, in the org `org_id` or, with `None`, in Personal, and in its workspace
    /// `workspace_id` or, with `None`, Default. The org must be joined on this PC; it is fixed
    /// from then on (env vault spec §5.2). In an account, the role in that workspace must allow
    /// changing dots, and giving it secrets when it is granted any (members spec §6.3).
    ///
    /// With `made`, the dot is one a maker made (dots-make-dots design section 7.1): who made
    /// it is recorded, and its grants are stored exactly as `spec` gives them, with no grant of
    /// a used server's variables (`Store::create_made_dot`).
    pub async fn create(
        &self,
        mut spec: DotSpec,
        org_id: Option<String>,
        workspace_id: Option<String>,
        made: Option<Made>,
    ) -> Result<Dot> {
        let p = &self.parts;
        if let Some(org) = &org_id {
            if p.store.get_org(org).await?.is_none() {
                return Err(Error::Invalid(format!(
                    "no org '{org}' is joined on this PC"
                )));
            }
        }
        let workspace_id =
            workspace_id.unwrap_or_else(|| crate::accounts::DEFAULT_WORKSPACE.into());
        self.check_workspace(org_id.as_deref(), &workspace_id)
            .await?;
        let workspace_id = workspace_id.as_str();
        self.require_in(org_id.as_deref(), workspace_id, Permission::EditDots)
            .await?;
        if !spec.variables.0.is_empty() {
            self.require_in(org_id.as_deref(), workspace_id, Permission::GrantEntries)
                .await?;
        }
        // A blank folder gets a new one of its own; remembered so a failed create removes it.
        let mut new_folder: Option<PathBuf> = None;
        if spec.workdir.trim().is_empty() {
            let used: Vec<PathBuf> = p
                .store
                .list_dots()
                .await?
                .into_iter()
                .map(|d| PathBuf::from(d.spec.workdir.trim()))
                .collect();
            let (dir, created) = default_folder(&p.new_dot_root, &spec.name, &used).await?;
            spec.workdir = dir.to_string_lossy().into_owned();
            // A folder that was there already, empty, may be another dot's: it stays.
            new_folder = created.then_some(dir);
            // A folder Dots made is never a repository's top level: a worktree would check out
            // whatever repository it happens to sit in.
            spec.workspace_mode = WorkspaceMode::Folder;
        }
        // Worktree mode needs the top level of a repository with a commit; elsewhere the dot
        // runs in the folder itself. A made dot is not moved to the folder itself: it would
        // write straight into a folder its maker only ever works in copies of.
        if spec.workspace_mode == WorkspaceMode::Worktree
            && !worktree_ready(Path::new(spec.workdir.trim())).await
        {
            if made.is_some() {
                return Err(Error::Invalid(MADE_NOT_A_REPO.into()));
            }
            spec.workspace_mode = WorkspaceMode::Folder;
        }
        let created = match &made {
            Some(made) => {
                p.store
                    .create_made_dot(&spec, org_id.as_deref(), workspace_id, made)
                    .await
            }
            None => {
                p.store
                    .create_dot(&spec, org_id.as_deref(), workspace_id)
                    .await
            }
        };
        let dot = match created {
            Ok(dot) => dot,
            Err(e) => {
                if let Some(dir) = new_folder {
                    // Only removes an empty folder.
                    let _ = tokio::fs::remove_dir(dir).await;
                }
                return Err(e);
            }
        };
        self.sync_schedule(&dot, false).await?;
        p.scheduler.reload();
        p.watchers.reload();
        Ok(dot)
    }

    /// Saves a dot's spec. A dot whose org is no longer joined on this PC is refused: it can't
    /// run until the org is joined again or the dot is deleted (env vault spec §6.4). In an
    /// account, the role must allow changing dots, and giving secrets to them when the grants
    /// change (members spec §6.3).
    ///
    /// With `exact`, the change is a maker's to a dot it made (dots-make-dots design section
    /// 9): the grants are stored exactly as `spec` gives them, with no grant of a newly used
    /// server's variables (`Store::update_dot_exact`).
    pub async fn update(&self, id: &str, spec: DotSpec, exact: bool) -> Result<Dot> {
        let p = &self.parts;
        let old = p.store.get_dot(id).await?;
        if let Some(text) = p.vault.org_removed(&old) {
            return Err(Error::Conflict(text));
        }
        self.require_dot(&old, Permission::EditDots).await?;
        if spec.variables != old.spec.variables {
            self.require_dot(&old, Permission::GrantEntries).await?;
        }
        // Only a change to the mode or the folder is refused: a stored dot whose repository
        // has gone can still have its other fields edited.
        if spec.workspace_mode == WorkspaceMode::Worktree
            && (spec.workspace_mode != old.spec.workspace_mode || spec.workdir != old.spec.workdir)
            && !spec.workdir.trim().is_empty()
            && !worktree_ready(Path::new(spec.workdir.trim())).await
        {
            return Err(Error::Invalid(NOT_A_REPO.into()));
        }
        let was_enabled = old.spec.enabled;
        let dot = if exact {
            p.store.update_dot_exact(id, &spec).await?
        } else {
            p.store.update_dot(id, &spec).await?
        };
        if dot.spec.name != old.spec.name {
            p.teams.dot_renamed(id).await;
        }
        self.sync_schedule(&dot, dot.spec.enabled && !was_enabled)
            .await?;
        p.scheduler.reload();
        p.watchers.reload();
        p.follow_ups.wake();
        // A cap raised or cleared lets its waiting runs start now.
        p.runner.nudge();
        Ok(dot)
    }

    /// Deletes the dot; its schedule row goes with it (`ON DELETE CASCADE`). In an account, the
    /// role must allow changing dots (members spec §6.3), or this PC is waiting for or removed
    /// from the member list, and only cleaning up (`require_cleanup`).
    pub async fn delete(&self, id: &str) -> Result<()> {
        let deleted = self.delete_rows(id, &[]).await?;
        self.after_delete(deleted).await;
        Ok(())
    }

    /// The first half of `delete`: the checks, and the dot's row deleted with `wakes` stored in
    /// the same transaction (`Store::delete_dot_if_idle_and_wake`). The caller then calls
    /// `after_delete` with what this returns, after any lock it holds for the rows is released:
    /// that half closes the dot's Edge and tells its teams, which can take a while.
    pub(crate) async fn delete_rows(
        &self,
        id: &str,
        wakes: &[(String, String, String)],
    ) -> Result<Deleted> {
        let p = &self.parts;
        let dot = p.store.get_dot(id).await?;
        self.require_cleanup(&dot).await?;
        let had_row = p.store.get_schedule_state(id).await?.is_some();
        let proposals = p.store.list_app_proposals(Some(id)).await?;
        let watchers: Vec<String> = p
            .store
            .list_watchers(id)
            .await?
            .into_iter()
            .map(|w| w.id)
            .collect();
        let teams = p.store.teams_of_dot(id).await?;
        let attachments = p.store.attachment_ids_of_dot(id).await?;
        let had_goals = p.store.count_intents_of_dot(id).await? > 0;
        p.store.delete_dot_if_idle_and_wake(id, wakes).await?;
        Ok(Deleted {
            dot,
            had_row,
            proposals,
            watchers,
            teams,
            attachments,
            had_goals,
        })
    }

    /// The second half of `delete`, once the dot's row is gone (`delete_rows`).
    pub(crate) async fn after_delete(&self, deleted: Deleted) {
        let p = &self.parts;
        let Deleted {
            dot,
            had_row,
            proposals,
            watchers,
            teams,
            attachments,
            had_goals,
        } = deleted;
        let id = dot.id.as_str();
        // It left its teams (`ON DELETE CASCADE`), and their workflows may now have a problem.
        p.teams.dot_deleted(&teams).await;
        // Its goals stay, unlinked (`ON DELETE SET NULL`).
        if had_goals {
            let _ = p.bus.send(RuntimeEvent::IntentsUpdated {
                org_id: dot.org_id.clone(),
                workspace_id: dot.workspace_id.clone(),
            });
        }
        // Its attached files' rows went with it; their stored copies go too (attachments spec
        // section 4.7).
        for a in &attachments {
            p.chat_files.remove(a);
        }
        // Its proposals' rows went with it; their files go too (app library spec section 6.2).
        for proposal in &proposals {
            p.apps.drop_proposed(&proposal.id).await;
        }
        // Its watchers went with it; what they staged and kept goes too.
        p.watchers.forget(&watchers);
        // It can no longer be viewed (needs-you spec section 7).
        {
            let mut viewing = lock(&p.viewing);
            if viewing.as_deref() == Some(id) {
                *viewing = None;
            }
        }
        // Its Edge closes and its profile goes with it (browser spec section 6).
        p.browsers.forget(id).await;
        if had_row {
            self.schedule_updated(id);
        }
        p.scheduler.reload();
        p.watchers.reload();
        p.follow_ups.wake();
    }

    /// Brings the dot's schedule row in line with the dot after a change of it (Scheduler v2
    /// spec §4): made for a new schedule, reset for a changed one or when `enabled_now` (the
    /// dot was disabled and is enabled now), deleted without a schedule. A change of the row
    /// is published as `ScheduleUpdated`.
    pub(crate) async fn sync_schedule(&self, dot: &Dot, enabled_now: bool) -> Result<()> {
        let schedule = dot.spec.schedule.as_ref();
        if self
            .parts
            .store
            .sync_schedule_state(&dot.id, schedule, enabled_now, self.parts.clock.now())
            .await?
        {
            self.schedule_updated(&dot.id);
        }
        Ok(())
    }

    pub(crate) fn schedule_updated(&self, dot_id: &str) {
        let _ = self.parts.bus.send(RuntimeEvent::ScheduleUpdated {
            dot_id: dot_id.to_string(),
        });
    }

    /// Refuses a workspace the account (`None` is Personal) doesn't have.
    pub(crate) async fn check_workspace(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
    ) -> Result<()> {
        let scope = org_id.unwrap_or(crate::accounts::PERSONAL);
        if self
            .parts
            .store
            .workspace(scope, workspace_id)
            .await?
            .is_some()
        {
            return Ok(());
        }
        Err(self.no_workspace(org_id, workspace_id).await)
    }

    pub(crate) async fn no_workspace(&self, org_id: Option<&str>, workspace_id: &str) -> Error {
        Error::NotFound(format!(
            "No workspace '{workspace_id}' in {}.",
            crate::accounts::account_label(Some(&self.parts.orgs), org_id).await
        ))
    }

    /// `Ok` when this PC may do `p` in the workspace `workspace_id` of `org_id`, where the
    /// member's role in the workspace narrows the account role (members spec §6.3); otherwise
    /// `Error::NotAllowed` naming the account and the role, or why this PC is not let in. Every
    /// check on a dot or team goes through here (`require_dot`), never the account role alone.
    pub async fn require_in(
        &self,
        org_id: Option<&str>,
        workspace_id: &str,
        p: Permission,
    ) -> Result<()> {
        crate::accounts::require_in(Some(&self.parts.orgs), org_id, workspace_id, p).await
    }

    /// `require` on `dot`, in its account and workspace.
    pub async fn require_dot(&self, dot: &Dot, p: Permission) -> Result<()> {
        crate::accounts::require_dot(Some(&self.parts.orgs), dot, p).await
    }

    /// `require_dot` of `EditDots` for cleaning up `dot`: switching it off or deleting it. A PC
    /// waiting for, or removed from, the account's member list may still do that: its dots
    /// can't run, and Leave removes the account. So may a member outside the dot's workspace
    /// (members spec §6.3). Enabling, editing and watcher changes stay refused for them.
    pub(crate) async fn require_cleanup(&self, dot: &Dot) -> Result<()> {
        if matches!(
            crate::accounts::access_dot(Some(&self.parts.orgs), dot),
            Access::Waiting | Access::Removed | Access::OutsideWorkspace { .. }
        ) {
            return Ok(());
        }
        self.require_dot(dot, Permission::EditDots).await
    }
}

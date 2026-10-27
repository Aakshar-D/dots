//! A dot proposes an app (app library spec section 6.2): it writes the app's folder in its
//! workspace and calls `propose_app`. Dots checks the folder, runs its tests and keeps a copy in
//! `<data>/apps/proposed/<id>/`; a problem goes back to the dot as a tool error, and nothing
//! reaches the person. A passing proposal is a review card. The person's answer continues the
//! dot's thread (`Runtime::decide_app_proposal`), as an answered question does.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::manifest::Permission;
use super::registry::{blocking, write_version};
use super::source::{self, AppSource};
use super::tests::{self, TestReport};
use super::{data, Apps};
use crate::events::RuntimeEvent;
use crate::library::{AppOrigin, LibraryItem};
use crate::policy::Action;
use crate::{Error, Result};

/// The tool's name on both engines (`mcp__dots__propose_app` on the Claude engine).
pub const TOOL: &str = "propose_app";
/// What the dot is told when the card is posted (spec section 6.2).
pub const SENT: &str =
    "Sent for review. Finish your turn; the person's answer arrives as your next message.";
/// What a second proposal in one run is told.
pub const PROPOSED_ALREADY: &str =
    "You already sent an app for review in this run; finish your turn now.";
/// What a decision on a decided proposal is told.
pub const DECIDED_ALREADY: &str = "this app proposal has already been decided";
/// What the dot reads when the person discards its app (spec section 11).
pub const DISCARDED: &str = "The person discarded the app.";
pub const MAX_NOTE: usize = 2_000;
/// The most proposals one dot may have waiting for the person.
pub const MAX_PENDING: usize = 5;
/// What a dot with `MAX_PENDING` proposals waiting is told.
pub const TOO_MANY_PENDING: &str =
    "five of your proposals already wait for the person: wait until they decide one";

pub const DESCRIPTION: &str = "Send an app you wrote in your workspace to the person for \
    review. The folder holds app.json (name, title, description, tools: each with name, \
    description, input as a JSON Schema and writes; permissions), logic.js (an ES module \
    exporting a function per tool, fn(input, ctx), with ctx.db.query(sql, params), \
    ctx.db.exec(sql, params), ctx.now, ctx.id(), ctx.actor and ctx.dots.list()), ui/index.html \
    (the person's screen, which calls dots.call(tool, input)), migrations/0001_<name>.sql and on, \
    an optional sample.sql, and tests/<name>.test.js using test(name, fn), assert.equal, \
    assert.deepEqual, assert.throws and call(tool, input, { actor, now }). To change an app, \
    send its whole folder, keeping the migrations that ran and adding new ones after them; copy \
    an installed app into your workspace with copy_app first. Dots checks the folder and runs \
    the tests; a problem comes back as an error for you to fix.";

/// The tool that copies an installed app into the run's workspace, under `ProposeApp` (spec
/// section 15): a change must keep the active version's migrations byte for byte, and only the
/// dot that wrote an app still has its folder.
pub const COPY_TOOL: &str = "copy_app";

pub const COPY_DESCRIPTION: &str = "Copy an installed app's active version, as Dots keeps it, \
    into a new folder of your workspace, to change it there and send it back with propose_app. \
    Keep its migrations exactly as they are and add new ones after them.";

/// `copy_app`'s arguments.
pub fn copy_properties() -> Value {
    json!({
        "app": { "type": "string", "description": "The app's name" },
        "path": { "type": "string", "description": "A new folder, relative to your workspace" }
    })
}

pub const COPY_REQUIRED: [&str; 2] = ["app", "path"];

/// `copy_app` as Dots' own `dots` MCP server lists it.
pub fn copy_mcp_schema() -> Value {
    json!({
        "name": COPY_TOOL,
        "description": COPY_DESCRIPTION,
        "inputSchema": {
            "type": "object", "properties": copy_properties(), "required": COPY_REQUIRED
        }
    })
}

/// The tool's arguments.
pub fn properties() -> Value {
    json!({
        "path": { "type": "string", "description": "The app's folder, relative to your workspace" },
        "note": { "type": "string", "description": "What you built or changed, for the person" }
    })
}

pub const REQUIRED: [&str; 2] = ["path", "note"];

/// The tool as Dots' own `dots` MCP server lists it.
pub fn mcp_schema() -> Value {
    json!({
        "name": TOOL,
        "description": DESCRIPTION,
        "inputSchema": { "type": "object", "properties": properties(), "required": REQUIRED }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    /// Waiting for the person.
    Pending,
    /// Added to the Library, or the app updated.
    Added,
    /// The person asked for changes.
    ChangesAsked,
    Discarded,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalStatus::Pending => "pending",
            ProposalStatus::Added => "added",
            ProposalStatus::ChangesAsked => "changes_asked",
            ProposalStatus::Discarded => "discarded",
        }
    }

    pub fn parse(s: &str) -> Result<ProposalStatus> {
        Ok(match s {
            "pending" => ProposalStatus::Pending,
            "added" => ProposalStatus::Added,
            "changes_asked" => ProposalStatus::ChangesAsked,
            "discarded" => ProposalStatus::Discarded,
            other => return Err(Error::Other(format!("unknown proposal status {other:?}"))),
        })
    }
}

/// A tool the card lists (spec section 6.2, "Summary").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
    pub writes: bool,
}

/// What the card's summary shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalSummary {
    pub title: String,
    pub description: String,
    pub tools: Vec<ToolSummary>,
    /// What it reaches outside its data: empty for "Nothing".
    pub permissions: Vec<Permission>,
    pub tests: TestReport,
    pub files: usize,
    pub bytes: u64,
    /// The migrations accepting it runs: every one for a new app, the new ones for a change.
    pub migrations: Vec<String>,
    /// The version a change was checked against; `None` for a new app.
    pub update_from: Option<u32>,
    /// Data a removed app of this name kept that fits this new app's migrations: the card
    /// offers to start on it (spec section 10).
    #[serde(default)]
    pub kept_data: Option<super::kept::KeptOffer>,
}

/// An app a dot proposed, and what the person decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppProposal {
    pub id: String,
    pub dot_id: String,
    pub run_id: String,
    pub root_run_id: String,
    pub app_name: String,
    /// The app a change is for; `None` for a new app.
    pub app_id: Option<String>,
    pub base_version: Option<u32>,
    pub note: String,
    pub summary: ProposalSummary,
    pub status: ProposalStatus,
    /// The person's "Ask for changes" text.
    pub answer: Option<String>,
    /// The run the decision started.
    pub answer_run_id: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// The person's answer on the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    /// "Add to library", or "Update" for a change.
    Add,
    /// "Add to library" on the kept data the card offered.
    AddWithKeptData,
    /// "Ask for changes": the text continues the dot's thread.
    AskForChanges {
        text: String,
    },
    Discard,
}

/// The sentence for the dot of an error: the rule itself for an input, conflict or missing
/// thing, the error otherwise.
fn said(e: Error) -> String {
    match e {
        Error::Invalid(m) | Error::Conflict(m) | Error::NotFound(m) => m,
        other => other.to_string(),
    }
}

/// A change keeps every migration the active version ran, unchanged (spec section 5: the data
/// has run them).
fn check_migrations_kept(
    current: &AppSource,
    next: &AppSource,
    version: u32,
) -> std::result::Result<(), String> {
    for old in &current.migrations {
        match next.migrations.iter().find(|m| m.number == old.number) {
            Some(m) if m.file == old.file && m.sql == old.sql => {}
            _ => {
                return Err(format!(
                    "migrations/{} differs from version {version}'s: keep the migrations that \
                     ran as they are and add new ones after them",
                    old.file
                ))
            }
        }
    }
    Ok(())
}

impl Apps {
    /// Where a proposal's files wait until it is decided.
    pub fn proposed_dir(&self, id: &str) -> PathBuf {
        self.root().join("proposed").join(id)
    }

    /// `propose_app` for the run `run_id` (spec section 6.2): checks the app folder `folder`
    /// (already confined to the run's workspace), runs its migrations on an empty database,
    /// loads `sample.sql` and runs the tests; then keeps a copy and posts the card. `Ok` is what
    /// the dot is told; `Err` the problem, in a sentence for the dot.
    pub async fn propose(
        &self,
        run_id: &str,
        folder: PathBuf,
        note: &str,
    ) -> std::result::Result<String, String> {
        let note = note.trim();
        if note.is_empty() || note.chars().count() > MAX_NOTE {
            return Err(format!("the note must be 1 to {MAX_NOTE} characters"));
        }
        let run = self.store.get_run(run_id).await.map_err(said)?;
        if self
            .store
            .pending_app_proposal_for_run(run_id)
            .await
            .map_err(said)?
            .is_some()
        {
            return Err(PROPOSED_ALREADY.to_string());
        }
        let waiting = self
            .store
            .list_app_proposals(Some(&run.dot_id))
            .await
            .map_err(said)?
            .into_iter()
            .filter(|p| p.status == ProposalStatus::Pending)
            .count();
        if waiting >= MAX_PENDING {
            return Err(TOO_MANY_PENDING.to_string());
        }
        let src = blocking(move || source::read_folder(&folder))
            .await
            .map_err(said)?
            .map_err(said)?;
        let name = src.manifest.name.clone();
        let mut kept_data = None;
        let (base, migrations) = match self.get(&name).await {
            Ok((item, config)) => {
                let current = self.load(&item, &config).await.map_err(said)?;
                check_migrations_kept(&current, &src, config.version)?;
                let newer = src
                    .migrations
                    .iter()
                    .filter(|m| m.number > current.schema_version())
                    .map(|m| m.file.clone())
                    .collect();
                (Some((item.id, config.version)), newer)
            }
            Err(Error::NotFound(_)) => {
                kept_data = self.kept_fit(&src).await.map_err(said)?;
                (
                    None,
                    src.migrations.iter().map(|m| m.file.clone()).collect(),
                )
            }
            Err(e) => return Err(said(e)),
        };
        let checked = src.clone();
        let report = blocking(move || -> std::result::Result<TestReport, String> {
            let mut conn = data::open_in_memory().map_err(said)?;
            data::migrate(&mut conn, &checked.migrations)
                .map_err(|e| format!("the migrations failed on an empty database: {}", said(e)))?;
            if let Some(sample) = &checked.sample {
                data::load_sample(&conn, sample)
                    .map_err(|e| format!("sample.sql failed: {}", said(e)))?;
            }
            Ok(tests::run_tests(&checked))
        })
        .await
        .map_err(said)??;
        if let Some(problem) = report.problem() {
            return Err(problem);
        }
        let files = src.files();
        let summary = ProposalSummary {
            title: src.manifest.title.clone(),
            description: src.manifest.description.clone(),
            tools: src
                .manifest
                .tools
                .iter()
                .map(|t| ToolSummary {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    writes: t.writes,
                })
                .collect(),
            permissions: src.manifest.permissions.clone(),
            tests: report,
            files: files.len(),
            bytes: files.values().map(|b| b.len() as u64).sum(),
            migrations,
            update_from: base.as_ref().map(|(_, v)| *v),
            kept_data,
        };
        let id = crate::util::new_id();
        let dir = self.proposed_dir(&id);
        let written = dir.clone();
        blocking(move || write_version(&written, &src))
            .await
            .map_err(said)?
            .map_err(said)?;
        let proposal = AppProposal {
            id: id.clone(),
            dot_id: run.dot_id.clone(),
            run_id: run.id.clone(),
            root_run_id: run.root_run_id.clone(),
            app_name: name,
            app_id: base.as_ref().map(|(id, _)| id.clone()),
            base_version: base.map(|(_, v)| v),
            note: note.to_string(),
            summary,
            status: ProposalStatus::Pending,
            answer: None,
            answer_run_id: None,
            created_at: crate::util::now(),
            decided_at: None,
        };
        if let Err(e) = self.store.add_app_proposal(&proposal).await {
            let _ = blocking(move || std::fs::remove_dir_all(dir)).await;
            // Two proposals at once in one run: the index keeps the first.
            if e.to_string().contains("app_proposals.run_id") {
                return Err(PROPOSED_ALREADY.to_string());
            }
            return Err(said(e));
        }
        self.proposal_changed(&proposal);
        Ok(SENT.to_string())
    }

    /// "Add to library" or "Update" for the pending proposal `p`: a new app at version 1, its
    /// switches off, on the kept data the card offered when `on_kept`, or the next version of
    /// the app it changes. A change whose app has moved on since it was checked is a
    /// `Conflict`.
    pub async fn accept(&self, p: &AppProposal, on_kept: bool) -> Result<LibraryItem> {
        if on_kept && p.base_version.is_some() {
            return Err(Error::Invalid(
                "kept data is offered only to a new app".into(),
            ));
        }
        let dir = self.proposed_dir(&p.id);
        let src = blocking(move || source::read_folder(&dir)).await??;
        let Some(base) = p.base_version else {
            return match (on_kept, &p.summary.kept_data) {
                (false, _) => self.install(src, AppOrigin::Proposed).await,
                (true, Some(kept)) => self.install_kept(src, &kept.id).await,
                (true, None) => Err(Error::Invalid(format!(
                    "no kept data was offered for the app {}",
                    p.app_name
                ))),
            };
        };
        let Some(app_id) = p.app_id.as_deref() else {
            return Err(Error::Other(format!(
                "the change to {} names no app",
                p.app_name
            )));
        };
        // Checked under the app's lock: the same app, at the version the change was checked on.
        match self
            .update_checked(&p.app_name, src, true, Some((app_id, base)))
            .await
        {
            Err(Error::NotFound(_)) => Err(Error::Conflict(format!(
                "the app {} was removed after this change was proposed; ask the dot to propose \
                 it again",
                p.app_name
            ))),
            other => other,
        }
    }

    /// `copy_app`: writes the files of the app `name`'s active version, as its version folder
    /// holds them, into `folder` (already confined to the run `run_id`'s workspace), which must
    /// be new or empty. `shown` is the folder as the dot named it. The write is judged first as
    /// the run's dot policy judges a `Write` of `<folder>/app.json`, so the dot copies only where
    /// it may write. A failure part way leaves nothing this call made. `Ok` is what the dot is
    /// told; `Err` the problem, in a sentence for the dot.
    pub async fn copy_to(
        &self,
        run_id: &str,
        name: &str,
        folder: PathBuf,
        shown: &str,
    ) -> std::result::Result<String, String> {
        let run = self.store.get_run(run_id).await.map_err(said)?;
        let dot = self.store.get_dot(&run.dot_id).await.map_err(said)?;
        // `folder` is confined, so canonical (on Windows with a verbatim prefix): judge the path
        // below the workspace, as a model names a file in its own workspace.
        let workspace = run.workspace_path.as_deref().map(std::path::Path::new);
        let target = folder.join("app.json");
        let target = workspace.and_then(|ws| {
            let canonical = std::fs::canonicalize(ws).ok();
            let rel = |ws: &std::path::Path| target.strip_prefix(ws).ok().map(PathBuf::from);
            canonical.as_deref().and_then(rel).or_else(|| rel(ws))
        });
        let allowed = target.is_some_and(|target| {
            let write = json!({ "file_path": target.to_string_lossy() });
            dot.spec.policy.resolve_in("Write", &write, workspace) == Action::Allow
        });
        if !allowed {
            return Err(format!(
                "your rules do not let you write to {shown}, so the app cannot be copied there"
            ));
        }
        let (item, config) = match self.get(name).await {
            Ok(found) => found,
            Err(Error::NotFound(_)) => return Err(format!("there is no app {name}")),
            Err(e) => return Err(said(e)),
        };
        let src = self.load(&item, &config).await.map_err(said)?;
        let in_use = format!("the folder {shown} is not empty: copy the app into a new folder");
        let is_file = format!("{shown} is a file: copy the app into a new folder");
        blocking(move || -> std::result::Result<(), String> {
            if folder.is_file() {
                return Err(is_file);
            }
            if let Ok(mut entries) = std::fs::read_dir(&folder) {
                if entries.next().is_some() {
                    return Err(in_use);
                }
            }
            // What this call makes, in order: the folders that were missing (the folder itself
            // when it was new) and the files it wrote.
            let mut made: Vec<PathBuf> = Vec::new();
            let written = (|| -> std::io::Result<()> {
                for (path, bytes) in src.files() {
                    let file = folder.join(path);
                    if let Some(parent) = file.parent() {
                        let mut missing = Vec::new();
                        let mut up = Some(parent);
                        while let Some(dir) = up.filter(|d| !d.exists()) {
                            missing.push(dir.to_path_buf());
                            up = dir.parent();
                        }
                        std::fs::create_dir_all(parent)?;
                        made.extend(missing.into_iter().rev());
                    }
                    made.push(file.clone());
                    std::fs::write(file, bytes)?;
                }
                Ok(())
            })();
            if let Err(e) = written {
                for path in made.iter().rev() {
                    let _ = std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path));
                }
                return Err(e.to_string());
            }
            Ok(())
        })
        .await
        .map_err(said)??;
        Ok(format!(
            "Copied version {} of the app {name} into {shown}. Change it there, keep its              migrations as they are and add new ones after them, then send the whole folder with              propose_app.",
            config.version
        ))
    }

    /// Deletes a decided proposal's files, and closes its previews.
    pub async fn drop_proposed(&self, id: &str) {
        self.previews.close_proposal(id);
        let dir = self.proposed_dir(id);
        let _ = blocking(move || std::fs::remove_dir_all(dir)).await;
    }

    pub(crate) fn proposal_changed(&self, p: &AppProposal) {
        let _ = self.bus.send(RuntimeEvent::AppProposalChanged {
            id: p.id.clone(),
            dot_id: p.dot_id.clone(),
        });
    }
}

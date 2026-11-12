//! What a maker's run does (dots-make-dots design sections 7 to 10), for the tools both engines
//! offer it. The tools reach it through the check-ins (`CheckIns::makers`), so the reach check
//! and the limits are enforced here, in the core, and not in the prompt (section 12).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::jobs::FirstJob;
use super::log::{MakingEventKind, NewMakingEvent};
use super::reach::{reach_new, Reach, Wanted};
use super::requests::{MakeRequest, NewMakeRequest, RequestKind, MAX_WAITING_REQUESTS};
use crate::dot_admin::DotAdmin;
use crate::events::{Bus, RuntimeEvent};
use crate::followups::FollowUps;
use crate::model::{Dot, DotSpec, Made, Making, Run, TriggerKind};
use crate::prompt::is_typed;
use crate::runner::Runner;
use crate::store::Store;
use crate::vault::{Consumer, GrantState, Vault};
use crate::{Error, Result};

/// The most characters of a made dot's purpose.
pub const MAX_PURPOSE: usize = 200;
/// The most characters of a job (design section 8).
pub const MAX_JOB: usize = 4000;
/// What is said of a purpose that is not one line of 1 to `MAX_PURPOSE` characters.
pub const PURPOSE_RULE: &str = "The purpose is one line of 1 to 200 characters.";
/// What is said of a first job that is not 1 to `MAX_JOB` characters.
pub const FIRST_JOB_RULE: &str = "The first job is 1 to 4000 characters.";
/// What a workflow step's run is told by every tool for making dots (design section 5).
pub const NOT_IN_A_STEP: &str =
    "A run that is a workflow step cannot use the tools for making dots.";
/// What a run that has ended is told by every tool for making dots.
pub const RUN_ENDED: &str = "This run has ended, so it cannot use the tools for making dots.";

/// What a maker's run asks for with `make_dot` (design section 7.1). In JSON it is one flat
/// object: `purpose`, `first_job` and the fields of `Wanted`. A field none of them has is
/// refused, so a call can not name "Can make dots", the account or the engine.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MakeDot {
    /// One line on what the dot is for, 1 to 200 characters, shown on cards and lists.
    pub purpose: String,
    #[serde(flatten)]
    pub wanted: Wanted,
    /// A first job for the new dot (design section 8), 1 to 4,000 characters when given. It
    /// starts when the dot is made: at once, or when the person allows the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_job: Option<String>,
}

// By hand: serde's `flatten` and `deny_unknown_fields` do not combine. The two fields of its
// own are taken out, and `Wanted` reads the rest and refuses what it does not have.
impl<'de> Deserialize<'de> for MakeDot {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        use serde::de::Error as _;
        let mut fields = serde_json::Map::<String, Value>::deserialize(d)?;
        let purpose = match fields.remove("purpose") {
            Some(Value::String(purpose)) => purpose,
            Some(Value::Null) | None => return Err(D::Error::missing_field("purpose")),
            Some(other) => return Err(D::Error::custom(format!("purpose is text, not {other}"))),
        };
        // A model writes `null` for a field it leaves out.
        let first_job = match fields.remove("first_job") {
            Some(Value::String(job)) => Some(job),
            Some(Value::Null) | None => None,
            Some(other) => return Err(D::Error::custom(format!("first_job is text, not {other}"))),
        };
        let wanted = serde_json::from_value(Value::Object(fields)).map_err(D::Error::custom)?;
        Ok(MakeDot {
            purpose,
            wanted,
            first_job,
        })
    }
}

/// What came of a `make_dot`.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum MakeOutcome {
    /// Created at once. `first_job` is what came of the first job, when one was given with
    /// the dot (design section 8).
    Made {
        dot: Dot,
        first_job: Option<FirstJob>,
    },
    /// Stored for the person; nothing was created (design section 7.1): something of it is
    /// beyond the maker's reach (`request.extras`), or a limit would be passed
    /// (`request.over`), or both. `maker_name` is the maker's name, for the words of the
    /// extras.
    Waiting {
        request: MakeRequest,
        maker_name: String,
    },
}

impl MakeOutcome {
    /// What the tool tells the maker's run (design section 7.1), the same on both engines.
    pub fn tool_text(&self) -> String {
        match self {
            Self::Made { dot, first_job } => {
                let mut text = format!(
                    "Made the dot \"{}\". It is on and ready; give it a job with give_job.",
                    dot.spec.name
                );
                if dot.spec.schedule.is_some() {
                    text.push_str(" It will also run on its schedule.");
                }
                if let Some(first_job) = first_job {
                    text.push_str(&first_job.sentence());
                }
                text
            }
            Self::Waiting {
                request,
                maker_name,
            } => {
                let extras: Vec<String> =
                    request.extras.iter().map(|e| e.line(maker_name)).collect();
                let more = format!("it asks for more than you have ({})", extras.join("; "));
                let why = match &request.over {
                    Some(over) if extras.is_empty() => format!("it would be the {}", over.line()),
                    Some(over) => format!("{more} and it would be the {}", over.line()),
                    None => more,
                };
                format!(
                    "Not made yet: \"{}\" needs the person's say-so, because {why}. The request \
                     is waiting for them. Do not wait for it and do not ask again: carry on with \
                     other work or end your turn, and you will be woken with their answer.",
                    request.name
                )
            }
        }
    }
}

/// Which limit a make would pass (design section 10), with the numbers the ask card's line
/// needs. In JSON the limit's number is `max`, since `limit` says which limit it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "limit", rename_all = "snake_case")]
pub enum OverLimit {
    /// It would be the `nth` dot this run of the maker made.
    PerRun {
        nth: u32,
        #[serde(rename = "max")]
        limit: u32,
    },
    /// It would be the `nth` made dot of the maker that exists.
    Total {
        nth: u32,
        #[serde(rename = "max")]
        limit: u32,
    },
}

/// `n` as "1st", "2nd", "3rd", "4th", "11th", "21st".
fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

impl OverLimit {
    /// The limit as one line of the ask card (design section 7.2).
    pub fn line(&self) -> String {
        match self {
            Self::PerRun { nth, limit } => {
                format!("{} dot in this run; the limit is {limit}", ordinal(*nth))
            }
            Self::Total { nth, limit } => {
                format!("{} made dot kept; the limit is {limit}", ordinal(*nth))
            }
        }
    }
}

/// `purpose` as it is kept, without the spaces around it, or `PURPOSE_RULE`.
pub(crate) fn checked_purpose(purpose: &str) -> Result<String> {
    let purpose = purpose.trim();
    if purpose.is_empty() || purpose.chars().count() > MAX_PURPOSE || purpose.contains(['\n', '\r'])
    {
        return Err(Error::Invalid(PURPOSE_RULE.into()));
    }
    Ok(purpose.to_string())
}

pub(super) struct Parts {
    pub(super) store: Store,
    pub(super) bus: Bus,
    pub(super) dot_admin: DotAdmin,
    pub(super) vault: Vault,
    /// Where runs' worktree copies are made (`RunnerConfig::worktrees_dir`).
    pub(super) worktrees_root: PathBuf,
    /// For the thread lock every writer that continues a chat holds, and the chats that are
    /// adding an app: a wake continues the maker's chat (design section 8).
    pub(super) follow_ups: Arc<FollowUps>,
    /// Announces a wake's run and wakes the dispatcher for it.
    pub(super) runner: Arc<Runner>,
    /// Held from counting a maker's dots until the new one is stored, so two calls of one run
    /// cannot both take the last place; and while the person's answer to a make request is
    /// carried out, so two answers, or an answer and a make, cannot both pass.
    pub(super) one_at_a_time: Mutex<()>,
    /// Held by the pass that follows the jobs (`Makers::settle_jobs`), so one runs at a time,
    /// and while a maker deletes a made dot whose open jobs it ends. Taken after
    /// `one_at_a_time` when both are held.
    pub(super) settling: Mutex<()>,
}

/// Everything a maker does to other dots, for the tools of both engines. Cheap to clone; the
/// clones are one service. The runtime makes it once, at start.
#[derive(Clone)]
pub struct Makers {
    pub(super) parts: Arc<Parts>,
}

/// A `Makers` that does not keep it alive: what the check-ins hold, since the makers hold the
/// dot admin, which holds the runner, which holds the check-ins.
pub(crate) struct WeakMakers(Weak<Parts>);

impl WeakMakers {
    /// The makers, while the runtime that made them exists.
    pub(crate) fn upgrade(&self) -> Option<Makers> {
        self.0.upgrade().map(|parts| Makers { parts })
    }
}

impl Makers {
    pub(crate) fn new(
        store: Store,
        bus: Bus,
        dot_admin: DotAdmin,
        vault: Vault,
        worktrees_root: PathBuf,
        follow_ups: Arc<FollowUps>,
        runner: Arc<Runner>,
    ) -> Makers {
        Makers {
            parts: Arc::new(Parts {
                store,
                bus,
                dot_admin,
                vault,
                worktrees_root,
                follow_ups,
                runner,
                one_at_a_time: Mutex::new(()),
                settling: Mutex::new(()),
            }),
        }
    }

    pub(crate) fn downgrade(&self) -> WeakMakers {
        WeakMakers(Arc::downgrade(&self.parts))
    }

    /// Where runs' worktree copies are made: no dot a maker makes may work in one (design
    /// section 6, folder rules).
    pub fn worktrees_root(&self) -> &Path {
        &self.parts.worktrees_root
    }

    /// A maker's run asks for a new dot (design section 7.1). `run_id` is the run that calls;
    /// its dot is the maker.
    ///
    /// `Made` when everything asked is within the maker's reach and its limits: the dot
    /// exists, in the maker's account and workspace, with who made it recorded, and the
    /// topic's log has a `made` entry; a first job given with it has started, or the outcome
    /// says why not (`FirstJob`). `Waiting` when something is beyond reach or a limit
    /// would be passed: nothing was created, a make request is stored for the person, and the
    /// topic's log has an `asked` entry. At most `MAX_WAITING_REQUESTS` of one maker wait; a
    /// further one is refused (`Invalid`).
    ///
    /// An error, with a sentence the model can act on, for what no answer of the person could
    /// allow: a dot that may not make dots, a workflow step's run, a run that has ended
    /// (`Invalid`), a dot that is not valid (`reach_new`), a Library item, variable or
    /// consumer that does not exist (`check_exists`), a name another dot has (`Conflict`), and
    /// a refusal of this PC's role in the maker's account (`NotAllowed`).
    pub async fn make_dot(&self, run_id: &str, ask: MakeDot) -> Result<MakeOutcome> {
        let p = &self.parts;
        let run = p.store.get_run(run_id).await?;
        let maker = p.store.get_dot(&run.dot_id).await?;
        may_make(&maker, &run)?;
        let purpose = checked_purpose(&ask.purpose)?;
        if let Some(job) = &ask.first_job {
            let job = job.trim();
            if job.is_empty() || job.chars().count() > MAX_JOB {
                return Err(Error::Invalid(FIRST_JOB_RULE.into()));
            }
        }
        let reach = reach_new(&maker, &ask.wanted, &p.worktrees_root)?;
        let _one_at_a_time = p.one_at_a_time.lock().await;
        // Before the limits and the extras: no request is kept for what can never be made.
        self.check_exists(&maker, &reach.spec).await?;
        self.check_name_free(&reach.spec.name).await?;
        let over = self.over_limit(&maker, &run.id).await?;
        if over.is_some() || !reach.extras.is_empty() {
            let request = self
                .store_request(&maker, &run, &purpose, &ask, &reach, over.as_ref())
                .await?;
            return Ok(MakeOutcome::Waiting {
                request,
                maker_name: maker.spec.name,
            });
        }
        let Reach {
            spec,
            create_folder,
            ..
        } = reach;
        let dot = self
            .create_made(
                &maker,
                &run.id,
                &run.root_run_id,
                spec,
                &purpose,
                create_folder,
            )
            .await?;
        // For the notification (design section 11): quiet when the person is watching.
        let _ = p.bus.send(RuntimeEvent::DotMade {
            dot_id: maker.id.clone(),
            root_run_id: run.root_run_id.clone(),
            maker_name: maker.spec.name.clone(),
            made_dot_id: dot.id.clone(),
            name: dot.spec.name.clone(),
            purpose: purpose.clone(),
            typed: self.started_by_typing(&run).await,
        });
        // The dot is made whatever comes of its first job (design section 8).
        let first_job = self
            .start_first_job(
                &maker,
                &run.id,
                &run.root_run_id,
                &dot,
                ask.first_job.as_deref(),
            )
            .await;
        Ok(MakeOutcome::Made { dot, first_job })
    }

    /// Stores the make request for a dot that is beyond reach or over a limit, writes the
    /// `asked` entry of the log, so the ask card shows in the topic in order with the other
    /// cards, and publishes `MakingUpdated` (design sections 7.1 and 7.2). Refuses it when
    /// the maker has `MAX_WAITING_REQUESTS` that the person has not answered.
    async fn store_request(
        &self,
        maker: &Dot,
        run: &Run,
        purpose: &str,
        ask: &MakeDot,
        reach: &Reach,
        over: Option<&OverLimit>,
    ) -> Result<MakeRequest> {
        let p = &self.parts;
        self.check_may_ask(maker).await?;
        let create_folder = reach
            .create_folder
            .as_ref()
            .map(|folder| folder.to_string_lossy().into_owned());
        let request = p
            .store
            .add_make_request(&NewMakeRequest {
                maker_dot_id: &maker.id,
                run_id: &run.id,
                root_run_id: &run.root_run_id,
                kind: RequestKind::Make,
                dot_id: None,
                purpose,
                spec: &reach.spec,
                without: &reach.without,
                extras: &reach.extras,
                over,
                create_folder: create_folder.as_deref(),
                first_job: ask.first_job.as_deref().map(str::trim),
                ask: None,
            })
            .await?;
        self.log_asked(&request, &maker.spec.name, None, &request.name)
            .await;
        Ok(request)
    }

    /// Refuses one more request of `maker` while it has `MAX_WAITING_REQUESTS` that the person
    /// has not answered (design section 7.2). Requests for a new dot and for a change count
    /// alike.
    pub(super) async fn check_may_ask(&self, maker: &Dot) -> Result<()> {
        let waiting = self
            .parts
            .store
            .count_waiting_make_requests(&maker.id)
            .await?;
        if waiting >= MAX_WAITING_REQUESTS {
            return Err(Error::Invalid(format!(
                "{} already has {MAX_WAITING_REQUESTS} requests waiting for the person. Wait to \
                 be woken with an answer before asking for more.",
                maker.spec.name
            )));
        }
        Ok(())
    }

    /// Writes the `asked` entry of the log for `request`, which was just stored, so the ask
    /// card shows in the topic in order with the other cards, and publishes `MakingUpdated`,
    /// and `MakeRequested` for the notification (design section 11). `maker` is the maker's
    /// name. `dot_id` and `dot_name` are the made dot a change is asked for, or `None` and the
    /// name asked for a new dot.
    pub(super) async fn log_asked(
        &self,
        request: &MakeRequest,
        maker: &str,
        dot_id: Option<&str>,
        dot_name: &str,
    ) {
        // The request waits whether or not its entry could be written, so a failure here is
        // logged and the ask still stands.
        let logged = self
            .parts
            .store
            .add_making_event(&NewMakingEvent {
                maker_dot_id: &request.maker_dot_id,
                run_id: &request.run_id,
                root_run_id: &request.root_run_id,
                kind: MakingEventKind::Asked,
                dot_id,
                dot_name,
                detail: json!({ "request_id": request.id }),
            })
            .await;
        if let Err(e) = logged {
            tracing::error!(request = %request.id, run = %request.run_id, "the request was not logged: {e}");
        }
        self.updated(&request.maker_dot_id, &request.root_run_id);
        let _ = self.parts.bus.send(RuntimeEvent::MakeRequested {
            request_id: request.id.clone(),
            dot_id: request.maker_dot_id.clone(),
            root_run_id: request.root_run_id.clone(),
            title: request.title(maker),
            name: request.name.clone(),
            purpose: request.purpose.clone(),
        });
    }

    /// Whether the person started the turn `run` belongs to by typing a message (design
    /// section 11): `run` carries a typed message (`prompt::is_typed`), or it resumes, after
    /// an approval, a run that does. A wake, a job, a scheduled run and an answer to a
    /// question are not typed.
    async fn started_by_typing(&self, run: &Run) -> bool {
        let mut at = run.clone();
        // A turn has far fewer resumes than this.
        for _ in 0..50 {
            if is_typed(&at) {
                return true;
            }
            let parent = match (at.trigger, at.parent_run_id.as_deref()) {
                (TriggerKind::Resume, Some(parent)) => parent,
                _ => return false,
            };
            at = match self.parts.store.get_run(parent).await {
                Ok(run) => run,
                Err(_) => return false,
            };
        }
        false
    }

    /// Refuses what `spec`, a dot a maker asks for, names that does not exist (design section
    /// 7.1): a Library item, a consumer and a variable. It is run again when the person
    /// answers a request, against the Library and the vault as they are then (section 7.2).
    ///
    /// A consumer exists when it is the shell or an MCP server the dot would use, named in its
    /// `uses` or come with a plugin it names. A variable is refused only when the vault says
    /// for sure that the maker's account has none of that name; when it cannot say (the account
    /// is offline, say), the name passes, as it does when a person saves a dot.
    pub(crate) async fn check_exists(&self, maker: &Dot, spec: &DotSpec) -> Result<()> {
        let p = &self.parts;
        p.store.check_uses_exist(&spec.uses).await?;
        if spec.variables.0.is_empty() {
            return Ok(());
        }
        let used: Vec<String> = p
            .store
            .used_mcp_servers(&spec.uses)
            .await?
            .into_iter()
            .map(|server| server.name)
            .collect();
        for key in spec.variables.0.keys() {
            if let Consumer::Mcp(server) = Consumer::parse(key)? {
                if !used.contains(&server) {
                    return Err(Error::Invalid(format!(
                        "The dot would not use an MCP server named '{server}', so it cannot be \
                         given variables for it. Name the server in uses, or leave those \
                         variables out."
                    )));
                }
            }
        }
        // The grants as the maker's account and workspace would resolve them: the made dot is
        // in both.
        let mut probe = maker.clone();
        probe.spec.variables = spec.variables.clone();
        for grant in p.vault.grant_statuses(&probe).await? {
            if grant.status == GrantState::NotDefined {
                return Err(Error::Invalid(format!(
                    "There is no variable named {}. Name one that exists, or leave it out and \
                     tell the person it is missing.",
                    grant.name
                )));
            }
        }
        Ok(())
    }

    /// Refuses a name another dot has, in any letter case (`Conflict`).
    pub(crate) async fn check_name_free(&self, name: &str) -> Result<()> {
        if self.parts.store.dot_name_taken(name.trim()).await? {
            return Err(Error::Conflict(format!(
                "A dot named {:?} already exists. Choose another name.",
                name.trim()
            )));
        }
        Ok(())
    }

    /// The limit one more dot of `maker`, made by its run `run_id`, would pass (design section
    /// 10); the run's limit first when it would pass both. The run's count is of the dots the
    /// run made, deleted ones too; the total is of the made dots that exist now.
    pub(crate) async fn over_limit(&self, maker: &Dot, run_id: &str) -> Result<Option<OverLimit>> {
        let store = &self.parts.store;
        let Making { per_run, total, .. } = maker.spec.making;
        let in_run = store.count_made_in_run(run_id).await?;
        if in_run >= per_run {
            return Ok(Some(OverLimit::PerRun {
                nth: in_run + 1,
                limit: per_run,
            }));
        }
        let kept = store.count_made_dots(&maker.id).await?;
        if kept >= total {
            return Ok(Some(OverLimit::Total {
                nth: kept + 1,
                limit: total,
            }));
        }
        Ok(None)
    }

    /// Creates the dot `spec` as one `maker` made in its run `run_id` of the topic
    /// `root_run_id`, writes the `made` entry of the log and publishes `MakingUpdated`. `spec`
    /// is already checked: within reach, or allowed by the person (design section 7.2); the
    /// limits are the caller's too. `purpose` is as `checked_purpose` gives it.
    ///
    /// Whatever `spec` says, the dot may not make dots and is enabled (section 6). It goes
    /// through `DotAdmin::create`, so the name, the Library items and this PC's rights in the
    /// maker's account are checked as for a dot the person creates. `create_folder` (of
    /// `Reach`) is made first, and removed again when the dot is not created.
    pub async fn create_made(
        &self,
        maker: &Dot,
        run_id: &str,
        root_run_id: &str,
        mut spec: DotSpec,
        purpose: &str,
        create_folder: Option<PathBuf>,
    ) -> Result<Dot> {
        let p = &self.parts;
        spec.making = Making::default();
        spec.enabled = true;
        // The folders that are made for it, deepest last.
        let mut new_folders: Vec<PathBuf> = Vec::new();
        if let Some(folder) = &create_folder {
            let mut missing = folder.as_path();
            while !missing.exists() {
                new_folders.push(missing.to_path_buf());
                match missing.parent() {
                    Some(parent) => missing = parent,
                    None => break,
                }
            }
            new_folders.reverse();
            tokio::fs::create_dir_all(folder).await?;
        }
        let made = Made {
            by_dot_id: Some(maker.id.clone()),
            by_name: maker.spec.name.clone(),
            run_id: run_id.to_string(),
            purpose: purpose.to_string(),
        };
        let created = p
            .dot_admin
            .create(
                spec,
                maker.org_id.clone(),
                Some(maker.workspace_id.clone()),
                Some(made),
            )
            .await;
        let dot = match created {
            Ok(dot) => dot,
            Err(e) => {
                // Only removes an empty folder, so nothing put there in between is lost.
                for folder in new_folders.iter().rev() {
                    let _ = tokio::fs::remove_dir(folder).await;
                }
                return Err(e);
            }
        };
        // The dot exists whether or not its entry could be written, so a failure here is
        // logged and the make still succeeds.
        let logged = p
            .store
            .add_making_event(&NewMakingEvent {
                maker_dot_id: &maker.id,
                run_id,
                root_run_id,
                kind: MakingEventKind::Made,
                dot_id: Some(&dot.id),
                dot_name: &dot.spec.name,
                detail: json!({ "purpose": purpose }),
            })
            .await;
        if let Err(e) = logged {
            tracing::error!(dot = %dot.id, run = %run_id, "the made dot was not logged: {e}");
        }
        self.updated(&maker.id, root_run_id);
        Ok(dot)
    }

    /// Says that the log of the topic `root_run_id` of the maker `maker_id` changed.
    pub(crate) fn updated(&self, maker_id: &str, root_run_id: &str) {
        let _ = self.parts.bus.send(RuntimeEvent::MakingUpdated {
            dot_id: maker_id.to_string(),
            root_run_id: root_run_id.to_string(),
        });
    }
}

/// Whether `run`, a run of `maker`, may use the tools for making dots (design section 5): the
/// dot's switch is on, the run is no workflow step's, and it has not ended. The same for every
/// one of the tools: making a dot, giving it a job, and listing, changing and deleting the dots
/// it made. What the dot's policy says of `MakeDots` is checked where the tool is called.
pub(crate) fn may_make(maker: &Dot, run: &Run) -> Result<()> {
    if !maker.spec.making.on {
        return Err(Error::Invalid(format!(
            "{} may not make dots. The person turns that on in the dot's settings.",
            maker.spec.name
        )));
    }
    // As for `propose_instructions` and `start_workflow`: a step's chat ends with the step, so
    // a wake or a card answer would have nowhere to land.
    if run.workflow_step_run_id.is_some() {
        return Err(Error::Invalid(NOT_IN_A_STEP.into()));
    }
    if run.status.is_terminal() {
        return Err(Error::Invalid(RUN_ENDED.into()));
    }
    Ok(())
}

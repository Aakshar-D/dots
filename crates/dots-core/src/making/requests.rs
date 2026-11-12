//! Make requests (dots-make-dots design sections 7.1, 7.2 and 10): a dot a maker asked for that
//! is beyond its reach or over a limit waits here for the person. Nothing of it exists until
//! they answer: allow it as asked, make it without the extras, or not make it. Every answer
//! wakes the maker.
//!
//! A change a maker asked for a dot it made that gives the dot more than the maker has waits
//! here too (section 9), as a request of the kind `change`: nothing of it is saved until the
//! person allows it, changes the dot without the extras, or does not change it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::jobs::FirstJob;
use super::log::{MakingEventKind, NewMakingEvent};
use super::manage::{ChangeDot, Standing, Stands};
use super::reach::Extra;
use super::service::{Makers, OverLimit};
use crate::accounts::Permission;
use crate::library::LibraryKind;
use crate::model::{Dot, DotSpec};
use crate::vault::{Consumer, GrantState};
use crate::{Error, Result};

/// The most requests of one maker that may wait for the person (design section 7.2).
pub const MAX_WAITING_REQUESTS: u32 = 3;
/// What an answer to a request that has its answer is told.
pub const ALREADY_ANSWERED: &str = "That request was already answered.";
/// What "Make it without these" is told for a request whose only problem is a limit.
pub const NOTHING_BEYOND_REACH: &str =
    "Nothing of this request is beyond reach, so it cannot be made without anything. Allow it, \
     or don't make it.";
/// What follows the reason when a request that names something that has gone is allowed.
pub const ONLY_REFUSE_LEFT: &str = "Only \"Don't make it\" is left.";
/// What the card says of a request marked gone whose reason was not kept.
pub const SOMETHING_GONE: &str = "Something this request names no longer exists.";
/// What follows the reason when a change request that names something that has gone is
/// allowed.
pub const ONLY_REFUSE_CHANGE_LEFT: &str = "Only \"Don't change it\" is left.";
/// What changing a dot without the extras is told when everything asked is beyond reach.
pub const NOTHING_WITHIN_REACH: &str =
    "Everything this request asks is beyond reach, so there is nothing to change without it. \
     Allow it, or don't change it.";

/// What Allow is told, and the card says, of a change request whose maker was narrowed since
/// it asked (`Makers::change_stands`). `maker` is the maker's name.
fn reach_shrank(maker: &str) -> String {
    format!("{maker}'s own settings changed since it asked. Don't change it, and let it ask again.")
}

str_enum!(RequestStatus {
    // The person has not answered.
    Waiting => "waiting",
    // Allow for this dot: the dot was made as asked.
    Allowed => "allowed",
    // Make it without these: the dot was made with the extras removed.
    MadeWithout => "made_without",
    // Don't make it.
    Refused => "refused",
    // Something it names has gone since it was asked: only Don't make it is left.
    Gone => "gone",
    // Dropped without an answer.
    Discarded => "discarded",
});

str_enum!(RequestKind {
    // A new dot (design section 7.1).
    Make => "make",
    // A change to a dot the maker made (design section 9).
    Change => "change",
});

impl RequestKind {
    /// What follows the reason when a request of this kind that names something that has gone
    /// is allowed.
    fn only_refuse_left(self) -> &'static str {
        match self {
            Self::Make => ONLY_REFUSE_LEFT,
            Self::Change => ONLY_REFUSE_CHANGE_LEFT,
        }
    }
}

str_enum!(MakeDecision {
    // Allow for this dot.
    Allow => "allow",
    // Make it without these.
    Without => "without",
    // Don't make it.
    Refuse => "refuse",
});

/// One make request, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakeRequest {
    pub id: String,
    /// The maker that asked.
    pub maker_dot_id: String,
    /// The maker's run that asked. A dot the answer makes records it as the run that made it.
    pub run_id: String,
    /// The maker's chat topic, where the ask card shows and the answer wakes the maker.
    pub root_run_id: String,
    pub kind: RequestKind,
    /// The made dot a change is asked for; `None` for a make.
    pub dot_id: Option<String>,
    /// The name of the dot asked for; for a change, the made dot's name as the change leaves
    /// it.
    pub name: String,
    /// One line on what the dot is for, as `checked_purpose` gives it; for a change, the new
    /// purpose, or the dot's own when the change names none.
    pub purpose: String,
    /// The dot as asked: what Allow makes. For a change, the made dot with the change on it
    /// as both were when it was asked: the card's lines come from it, and an answer puts `ask`
    /// on the dot as it is then instead (`Makers::change_stands`).
    pub spec: DotSpec,
    /// The same with the extras removed: what Make it without these makes.
    pub without: DotSpec,
    /// What lies beyond the maker's reach; empty when only a limit is passed.
    pub extras: Vec<Extra>,
    /// The limit that would be passed, with the numbers as they were when it was asked.
    pub over: Option<OverLimit>,
    /// The folder of `spec` to make first, when it lies below the maker's and does not exist
    /// (`Reach::create_folder`).
    pub create_folder: Option<String>,
    /// The first job given with the request.
    pub first_job: Option<String>,
    pub status: RequestStatus,
    /// What has gone, in one sentence for the person, when `status` is `gone`.
    pub gone_reason: Option<String>,
    pub created_at: String,
    /// When the person answered.
    pub decided_at: Option<String>,
    /// The dot an answer made, or the made dot an answer changed; `None` once that dot is
    /// deleted.
    pub made_dot_id: Option<String>,
    /// The change as the maker asked it, for a request of the kind `change`; `None` for a
    /// make.
    #[serde(default)]
    pub ask: Option<ChangeDot>,
}

/// Input for `Store::add_make_request`. The request is stored waiting, as of now.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMakeRequest<'a> {
    pub maker_dot_id: &'a str,
    pub run_id: &'a str,
    pub root_run_id: &'a str,
    pub kind: RequestKind,
    pub dot_id: Option<&'a str>,
    pub purpose: &'a str,
    pub spec: &'a DotSpec,
    pub without: &'a DotSpec,
    pub extras: &'a [Extra],
    pub over: Option<&'a OverLimit>,
    pub create_folder: Option<&'a str>,
    pub first_job: Option<&'a str>,
    /// The change as asked, for a request of the kind `change`.
    pub ask: Option<&'a ChangeDot>,
}

impl MakeRequest {
    /// The ask card's title (design sections 7.2 and 9). `maker` is the maker's name. A
    /// change's title names the made dot as the maker named it when it asked; the card names
    /// it as it is called now (`Makers::request_card`).
    pub fn title(&self, maker: &str) -> String {
        if self.kind == RequestKind::Change {
            let dot = self.ask.as_ref().map_or(self.name.as_str(), |ask| &ask.dot);
            change_title(maker, dot.trim())
        } else if self.extras.is_empty() {
            format!("{maker} wants to make one more dot")
        } else {
            format!("{maker} wants to make a dot with more than it has")
        }
    }

    /// The ask card's lines (design section 7.2): each extra in plain words, then the limit
    /// when one is passed.
    pub fn lines(&self, maker: &str) -> Vec<String> {
        let mut lines: Vec<String> = self.extras.iter().map(|e| e.line(maker)).collect();
        lines.extend(self.over.as_ref().map(OverLimit::line));
        lines
    }

    /// The folder to make before `without` is saved: `create_folder` when the folder asked
    /// for is no extra, so `without` still works in it. When the folder was the extra,
    /// `without` has a blank one and gets a new folder of its own.
    fn without_folder(&self) -> Option<PathBuf> {
        let same =
            !self.without.workdir.trim().is_empty() && self.without.workdir == self.spec.workdir;
        self.create_folder
            .as_ref()
            .filter(|_| same)
            .map(PathBuf::from)
    }
}

/// The title of the ask card for a change to the made dot named `dot` (design section 9).
fn change_title(maker: &str, dot: &str) -> String {
    format!("{maker} wants to give {dot} more than it has")
}

/// What the maker is woken with after the person's answer `decision` to `request`, a request
/// of the kind `change` (design section 9). `maker` is the maker's name.
fn change_answer_message(request: &MakeRequest, decision: MakeDecision, maker: &str) -> String {
    let name = &request.name;
    match decision {
        MakeDecision::Allow => {
            format!("The person allowed it: \"{name}\" was changed as you asked.")
        }
        MakeDecision::Without => {
            let extras: Vec<String> = request.extras.iter().map(|e| e.line(maker)).collect();
            format!(
                "The person chose to change \"{name}\" without what was beyond your reach. Left \
                 out: {}.",
                extras.join("; ")
            )
        }
        MakeDecision::Refuse => format!(
            "The person chose not to change \"{name}\". Do not ask again for the same thing."
        ),
    }
}

/// What the maker is woken with after the person's answer `decision` to `request` (design
/// sections 7.2 and 9). `maker` is the maker's name. `first_job` is what came of the first job
/// given with the request, when the answer made the dot: the message ends with its sentence
/// (`FirstJob::sentence`).
pub fn answer_message(
    request: &MakeRequest,
    decision: MakeDecision,
    maker: &str,
    first_job: Option<&FirstJob>,
) -> String {
    if request.kind == RequestKind::Change {
        return change_answer_message(request, decision, maker);
    }
    let name = &request.name;
    let mut message = match decision {
        MakeDecision::Allow => {
            format!("The person allowed it: the dot \"{name}\" was made as you asked.")
        }
        MakeDecision::Without => {
            let extras: Vec<String> = request.extras.iter().map(|e| e.line(maker)).collect();
            format!(
                "The person chose to make \"{name}\" without what was beyond your reach. It \
                 was made without: {}.",
                extras.join("; ")
            )
        }
        MakeDecision::Refuse => {
            return format!(
                "The person chose not to make \"{name}\". Do not ask again for the same thing; \
                 carry on without it, or tell the person what you cannot do."
            )
        }
    };
    if let Some(first_job) = first_job {
        message.push_str(&first_job.sentence());
    }
    message
}

/// What the ask card shows (design section 7.2), built from a request as things are now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakeRequestCard {
    pub request: MakeRequest,
    /// The maker's name now.
    pub maker_name: String,
    pub title: String,
    /// The extras in plain words, one per line, then the limit when one is passed.
    pub lines: Vec<String>,
    /// Whether Allow for this dot is on.
    pub can_allow: bool,
    /// Whether Make it without these is on. Off when the only problem is a limit.
    pub can_without: bool,
    /// Why Allow is off, for a request that has no answer yet: what this PC's role in the
    /// maker's account does not allow, or what has gone.
    pub blocked: Option<String>,
}

impl Makers {
    /// The ask card of a request (design section 7.2). A waiting request is checked again
    /// against the Library, the vault and the dots as they are now: when something it names has
    /// gone, or its name is taken, it is marked `gone`, `blocked` says what has gone and only
    /// Don't make it is left. A request once marked stays so.
    ///
    /// Allow needs this PC's right to change dots in the maker's account and workspace, and to
    /// give secrets when the dot is given variables; Make it without these needs the same for
    /// the dot without the extras, so it may be on when Allow is off.
    pub async fn request_card(&self, request_id: &str) -> Result<MakeRequestCard> {
        let p = &self.parts;
        // With the answers: a request is not marked gone while an answer to it makes its dot.
        let _one_at_a_time = p.one_at_a_time.lock().await;
        let mut request = p.store.get_make_request(request_id).await?;
        let maker = p.store.get_dot(&request.maker_dot_id).await?;
        if request.kind == RequestKind::Change {
            return self.change_card(request, maker).await;
        }
        if request.status == RequestStatus::Waiting {
            if let Some(reason) = self.gone_since(&maker, &request).await? {
                request = self.mark_gone(&request, &reason).await?;
            }
        }
        let (mut can_allow, mut can_without, mut blocked) = (false, false, None);
        match request.status {
            RequestStatus::Waiting => {
                match self.may_answer(&maker, &request.spec).await {
                    Ok(()) => can_allow = true,
                    Err(Error::NotAllowed(why)) => blocked = Some(why),
                    Err(e) => return Err(e),
                }
                if !request.extras.is_empty() {
                    can_without = match self.may_answer(&maker, &request.without).await {
                        Ok(()) => true,
                        Err(Error::NotAllowed(_)) => false,
                        Err(e) => return Err(e),
                    };
                }
            }
            RequestStatus::Gone => {
                blocked = Some(
                    request
                        .gone_reason
                        .clone()
                        .unwrap_or_else(|| SOMETHING_GONE.to_string()),
                );
            }
            _ => {}
        }
        Ok(MakeRequestCard {
            title: request.title(&maker.spec.name),
            lines: request.lines(&maker.spec.name),
            maker_name: maker.spec.name,
            request,
            can_allow,
            can_without,
            blocked,
        })
    }

    /// The ask card of a change request (design section 9), under the service's lock. Its
    /// title names the made dot as it is called now. A waiting request is held against things
    /// as they are now (`change_stands`): when the dot or something the change names has gone,
    /// it is marked `gone` and only Don't change it is left.
    ///
    /// Allow is on when the change as it would be saved now gives the dot nothing the card did
    /// not show, and this PC may change the dot, and give it secrets when its variables
    /// change. Changing it without the extras is on only when that would change something:
    /// when everything asked is beyond reach, the buttons are Allow and Don't change it.
    async fn change_card(&self, mut request: MakeRequest, maker: Dot) -> Result<MakeRequestCard> {
        let maker_name = maker.spec.name.clone();
        let mut title = request.title(&maker_name);
        let (mut can_allow, mut can_without, mut blocked) = (false, false, None);
        match request.status {
            RequestStatus::Waiting => match self.change_stands(&maker, &request).await? {
                Stands::Gone(reason) => {
                    request = self.mark_gone(&request, &reason).await?;
                    blocked = Some(reason);
                }
                Stands::Yes(standing) => {
                    title = change_title(&maker_name, &standing.dot.spec.name);
                    if standing.beyond_what_was_seen {
                        blocked = Some(reach_shrank(&maker_name));
                    } else {
                        match self.may_change(&standing.dot, &standing.reach.spec).await {
                            Ok(()) => can_allow = true,
                            Err(Error::NotAllowed(why)) => blocked = Some(why),
                            Err(e) => return Err(e),
                        }
                    }
                    if standing.without_changes_something() {
                        let without = &standing.reach.without;
                        can_without = match self.may_change(&standing.dot, without).await {
                            Ok(()) => true,
                            Err(Error::NotAllowed(_)) => false,
                            Err(e) => return Err(e),
                        };
                    }
                }
            },
            status => {
                // The dot's name now, while it exists.
                if let Some(dot_id) = &request.dot_id {
                    if let Ok(dot) = self.parts.store.get_dot(dot_id).await {
                        title = change_title(&maker_name, &dot.spec.name);
                    }
                }
                if status == RequestStatus::Gone {
                    blocked = Some(
                        request
                            .gone_reason
                            .clone()
                            .unwrap_or_else(|| SOMETHING_GONE.to_string()),
                    );
                }
            }
        }
        Ok(MakeRequestCard {
            title,
            lines: request.lines(&maker_name),
            maker_name,
            request,
            can_allow,
            can_without,
            blocked,
        })
    }

    /// `Ok` when this PC may save `spec` as the made dot `dot` (design sections 5 and 9), as
    /// `DotAdmin::update` checks it: the right to change dots in the dot's account and
    /// workspace, and to give secrets when its variables change. `NotAllowed` says which is
    /// missing.
    async fn may_change(&self, dot: &Dot, spec: &DotSpec) -> Result<()> {
        let admin = &self.parts.dot_admin;
        admin.require_dot(dot, Permission::EditDots).await?;
        if spec.variables != dot.spec.variables {
            admin.require_dot(dot, Permission::GrantEntries).await?;
        }
        Ok(())
    }

    /// The person answers a change request (design section 9), under the service's lock: the
    /// request as it is after, and the maker's name.
    ///
    /// Allow and changing it without the extras put the ask on the dot as it is now
    /// (`change_stands`), so what the person edited since the maker asked stays. When the dot
    /// or something the change names has gone, the request is marked `gone` and the answer is
    /// refused (`Conflict`). When the maker was narrowed since it asked, so that Allow would
    /// give the dot something the card did not show, Allow is refused (`Conflict`) and the
    /// request still waits, for a refusal. When everything asked is beyond reach there is
    /// nothing to change without it (`Invalid`). When the dot cannot be saved (this PC's
    /// role, a folder change while the dot has a run going), the request still waits and the
    /// error is returned. The topic's log gets a `changed` entry, as the asking run's, and
    /// then the `answered` one.
    async fn answer_change(
        &self,
        request: MakeRequest,
        decision: MakeDecision,
    ) -> Result<(MakeRequest, String)> {
        let p = &self.parts;
        let maker = p.store.get_dot(&request.maker_dot_id).await?;
        let changed = match decision {
            MakeDecision::Refuse => None,
            MakeDecision::Allow | MakeDecision::Without => {
                let standing = match self.change_stands(&maker, &request).await? {
                    Stands::Yes(standing) => standing,
                    Stands::Gone(reason) => {
                        self.mark_gone(&request, &reason).await?;
                        return Err(Error::Conflict(format!(
                            "{reason} {ONLY_REFUSE_CHANGE_LEFT}"
                        )));
                    }
                };
                if decision == MakeDecision::Allow && standing.beyond_what_was_seen {
                    return Err(Error::Conflict(reach_shrank(&maker.spec.name)));
                }
                if decision == MakeDecision::Without && !standing.without_changes_something() {
                    return Err(Error::Invalid(NOTHING_WITHIN_REACH.into()));
                }
                let without_folder = standing.without_folder();
                let Standing {
                    dot,
                    reach,
                    purpose,
                    ..
                } = standing;
                let (spec, folder) = match decision {
                    MakeDecision::Allow => (reach.spec, reach.create_folder),
                    _ => (reach.without, without_folder),
                };
                let (dot, _) = self
                    .apply_change(
                        &maker,
                        &dot,
                        spec,
                        purpose.as_deref(),
                        folder,
                        &request.run_id,
                        &request.root_run_id,
                    )
                    .await?;
                Some(dot)
            }
        };
        // The dot the answer is about, while it exists: the log's entry may only name a dot
        // that does.
        let dot = match (changed, &request.dot_id) {
            (Some(dot), _) => Some(dot),
            (None, Some(dot_id)) => p.store.get_dot(dot_id).await.ok(),
            (None, None) => None,
        };
        let dot_name = dot.as_ref().map_or(request.name.as_str(), |d| &d.spec.name);
        let changed_id = match decision {
            MakeDecision::Refuse => None,
            _ => dot.as_ref().map(|d| d.id.as_str()),
        };
        self.record_answer(
            &request,
            decision,
            changed_id,
            dot.as_ref().map(|d| d.id.as_str()),
            dot_name,
        )
        .await?;
        let request = p.store.get_make_request(&request.id).await?;
        Ok((request, maker.spec.name))
    }

    /// Writes the person's answer `decision` on `request`, then the `answered` entry of the
    /// log, and publishes `MakingUpdated`. `made_dot_id` is the dot the answer made or changed;
    /// `dot_id` and `dot_name` are the dot the log's entry is about.
    async fn record_answer(
        &self,
        request: &MakeRequest,
        decision: MakeDecision,
        made_dot_id: Option<&str>,
        dot_id: Option<&str>,
        dot_name: &str,
    ) -> Result<()> {
        let p = &self.parts;
        let status = match decision {
            MakeDecision::Allow => RequestStatus::Allowed,
            MakeDecision::Without => RequestStatus::MadeWithout,
            MakeDecision::Refuse => RequestStatus::Refused,
        };
        if !p
            .store
            .decide_make_request(&request.id, status, made_dot_id)
            .await?
        {
            // Every writer of a request's status holds the lock, so this is a request that
            // went with its maker in between.
            tracing::error!(request = %request.id, "the answered request could not be marked");
        }
        let logged = p
            .store
            .add_making_event(&NewMakingEvent {
                maker_dot_id: &request.maker_dot_id,
                run_id: &request.run_id,
                root_run_id: &request.root_run_id,
                kind: MakingEventKind::Answered,
                dot_id,
                dot_name,
                detail: json!({
                    "request_id": request.id,
                    "decision": decision.as_str(),
                    "dot_name": dot_name,
                }),
            })
            .await;
        if let Err(e) = logged {
            tracing::error!(request = %request.id, "the answer was not logged: {e}");
        }
        self.updated(&request.maker_dot_id, &request.root_run_id);
        Ok(())
    }

    /// The person answers a make request (design section 7.2), or a request to change a made
    /// dot (section 9, `answer_change`). Returns the request as it is after, and wakes the
    /// maker with what they chose.
    ///
    /// Only a waiting request can be allowed or made without, and one marked `gone` can only
    /// be refused; any other is `Conflict`. A request whose only problem is a limit cannot be
    /// made without anything (`Invalid`). Before a dot is made the request is checked again:
    /// when something it names has gone or its name is taken, it is marked `gone` and the
    /// answer is refused (`Conflict`, saying what has gone); when this PC's role does not allow
    /// the dot, the answer is refused (`NotAllowed`) and the request still waits. The reach
    /// check is not run again, and neither are the limits: Allow covers this one dot, also
    /// past a limit (section 10). When the dot cannot be created the request still waits and
    /// the error is returned.
    ///
    /// The maker's switch is not looked at: when the person turned "Can make dots" off after
    /// the request was stored, Allow and Make it without these still make the dot, since it
    /// is the person who decides, knowing both.
    pub async fn decide_request(
        &self,
        request_id: &str,
        decision: MakeDecision,
    ) -> Result<MakeRequest> {
        let (request, maker_name, first_job) = {
            // With `make_dot`: two answers, or an answer and a make, cannot both pass.
            let _one_at_a_time = self.parts.one_at_a_time.lock().await;
            self.answer(request_id, decision).await?
        };
        // After every lock is released: `wake` takes the thread lock itself.
        let message = answer_message(&request, decision, &maker_name, first_job.as_ref());
        if let Err(e) = self
            .wake(&request.maker_dot_id, &request.root_run_id, &message)
            .await
        {
            tracing::warn!(request = %request.id, "the maker was not woken with the answer: {e}");
        }
        Ok(request)
    }

    /// `decide_request` without the wake, under the service's lock: the request as it is
    /// after, the maker's name, and what came of the first job given with the request, when
    /// the answer made the dot (design section 7.2): it starts once the dot is made, as the
    /// job of the run that asked, in that run's topic.
    async fn answer(
        &self,
        request_id: &str,
        decision: MakeDecision,
    ) -> Result<(MakeRequest, String, Option<FirstJob>)> {
        let p = &self.parts;
        let request = p.store.get_make_request(request_id).await?;
        match (request.status, decision) {
            (RequestStatus::Waiting, _) | (RequestStatus::Gone, MakeDecision::Refuse) => {}
            (RequestStatus::Gone, _) => {
                let reason = request.gone_reason.as_deref().unwrap_or(SOMETHING_GONE);
                let left = request.kind.only_refuse_left();
                return Err(Error::Conflict(format!("{reason} {left}")));
            }
            _ => return Err(Error::Conflict(ALREADY_ANSWERED.into())),
        }
        if request.kind == RequestKind::Change {
            let (request, maker_name) = self.answer_change(request, decision).await?;
            return Ok((request, maker_name, None));
        }
        if decision == MakeDecision::Without && request.extras.is_empty() {
            return Err(Error::Invalid(NOTHING_BEYOND_REACH.into()));
        }
        let maker = p.store.get_dot(&request.maker_dot_id).await?;
        let made = match decision {
            MakeDecision::Refuse => None,
            MakeDecision::Allow | MakeDecision::Without => {
                if let Some(reason) = self.gone_since(&maker, &request).await? {
                    self.mark_gone(&request, &reason).await?;
                    return Err(Error::Conflict(format!("{reason} {ONLY_REFUSE_LEFT}")));
                }
                let (spec, folder) = match decision {
                    MakeDecision::Allow => (
                        request.spec.clone(),
                        request.create_folder.as_ref().map(PathBuf::from),
                    ),
                    _ => (request.without.clone(), request.without_folder()),
                };
                self.may_answer(&maker, &spec).await?;
                // The dot records the run that asked as the run that made it, though that
                // run may have ended, and its `made` entry is that run's. So it counts toward
                // that run's limit (`Store::count_made_in_run`) as well as the total, which is
                // intended: the run asked for it, and the person's answer is no way around
                // the count for what the run asks next.
                let dot = self
                    .create_made(
                        &maker,
                        &request.run_id,
                        &request.root_run_id,
                        spec,
                        &request.purpose,
                        folder,
                    )
                    .await?;
                Some(dot)
            }
        };
        // The dot is made whatever comes of its first job (design section 8).
        let first_job = match &made {
            Some(dot) => {
                self.start_first_job(
                    &maker,
                    &request.run_id,
                    &request.root_run_id,
                    dot,
                    request.first_job.as_deref(),
                )
                .await
            }
            None => None,
        };
        let made_id = made.as_ref().map(|dot| dot.id.as_str());
        self.record_answer(&request, decision, made_id, made_id, &request.name)
            .await?;
        let request = p.store.get_make_request(&request.id).await?;
        Ok((request, maker.spec.name, first_job))
    }

    /// `Ok` when this PC may create `spec` in the maker's account and workspace (design
    /// sections 5 and 7.2): the right to change dots, and to give secrets when `spec` gives
    /// variables. `NotAllowed` says which is missing.
    async fn may_answer(&self, maker: &Dot, spec: &DotSpec) -> Result<()> {
        let admin = &self.parts.dot_admin;
        let (org, workspace) = (maker.org_id.as_deref(), maker.workspace_id.as_str());
        admin
            .require_in(org, workspace, Permission::EditDots)
            .await?;
        if !spec.variables.0.is_empty() {
            admin
                .require_in(org, workspace, Permission::GrantEntries)
                .await?;
        }
        Ok(())
    }

    /// What `request` names that is not there now, in one sentence for the person; `None`
    /// when all of it is (design section 7.2): its name taken by a dot, a Library item, an
    /// MCP server its variables are for, or a variable.
    async fn gone_since(&self, maker: &Dot, request: &MakeRequest) -> Result<Option<String>> {
        if self.parts.store.dot_name_taken(request.name.trim()).await? {
            return Ok(Some(format!(
                "A dot named {:?} exists now.",
                request.name.trim()
            )));
        }
        self.gone_in(maker, &request.spec).await
    }

    /// What `spec`, the dot an answer would save, names that is not there now, in one
    /// sentence for the person; `None` when all of it is: a Library item, an MCP server its
    /// variables are for, or a variable.
    pub(super) async fn gone_in(&self, maker: &Dot, spec: &DotSpec) -> Result<Option<String>> {
        let p = &self.parts;
        for kind in LibraryKind::ALL {
            for name in spec.uses.names(*kind) {
                if !p.store.library_has(*kind, name).await? {
                    return Ok(Some(format!(
                        "The {} '{name}' is no longer in the Library.",
                        kind.label()
                    )));
                }
            }
        }
        if !spec.variables.0.is_empty() {
            let used = p.store.used_mcp_servers(&spec.uses).await?;
            for key in spec.variables.0.keys() {
                if let Ok(Consumer::Mcp(server)) = Consumer::parse(key) {
                    if !used.iter().any(|s| s.name == server) {
                        return Ok(Some(format!(
                            "The MCP server '{server}' is no longer one the dot would use."
                        )));
                    }
                }
            }
            let mut probe = maker.clone();
            probe.spec.variables = spec.variables.clone();
            for grant in p.vault.grant_statuses(&probe).await? {
                if grant.status == GrantState::NotDefined {
                    return Ok(Some(format!(
                        "The variable {} no longer exists.",
                        grant.name
                    )));
                }
            }
        }
        // The check a make meets, so nothing it refuses is passed here.
        match self.check_exists(maker, spec).await {
            Ok(()) => Ok(None),
            Err(Error::Invalid(_)) => Ok(Some(SOMETHING_GONE.to_string())),
            Err(e) => Err(e),
        }
    }

    /// Marks the waiting `request` gone, with `reason`, and says the topic's cards changed.
    /// Returns it as it is after.
    async fn mark_gone(&self, request: &MakeRequest, reason: &str) -> Result<MakeRequest> {
        let p = &self.parts;
        if p.store.mark_make_request_gone(&request.id, reason).await? {
            self.updated(&request.maker_dot_id, &request.root_run_id);
        }
        p.store.get_make_request(&request.id).await
    }
}

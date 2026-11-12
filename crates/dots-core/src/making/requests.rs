//! Make requests (dots-make-dots design sections 7.1, 7.2 and 10): a dot a maker asked for that
//! is beyond its reach or over a limit waits here for the person. Nothing of it exists until
//! they answer: allow it as asked, make it without the extras, or not make it. Every answer
//! wakes the maker.
//!
//! A change a maker asked for a dot it made that gives the dot more than the maker has waits
//! here too (section 9), as a request of the kind `change`: nothing of it is saved until the
//! person allows it, changes the dot without the extras, or does not change it.
//!
//! A job a maker gives a dot it made that holds more than the maker has waits here as well
//! (section 8), as a request of the kind `job`: it starts only when the person allows it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::jobs::FirstJob;
use super::log::{MakingEventKind, NewMakingEvent};
use super::manage::{change_lines, same_thing, schedule_words, ChangeDot, Standing, Stands};
use super::reach::{reach_new, spec_holds, Extra, Reach};
use super::service::{MakeDot, Makers, OverLimit};
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

/// What "without" is told for a job: a job has no part to start without the rest.
pub const NOTHING_WITHOUT_JOB: &str =
    "A job cannot be given without a part of it. Allow it, or don't give it.";
/// What follows the reason when a job request that names something that has gone is allowed.
pub const ONLY_REFUSE_JOB_LEFT: &str = "Only \"Don't give it\" is left.";

/// What changing a dot without the extras is told when the dot itself holds more than its maker
/// has: the change would steer that, so no part of it is saved alone.
pub const NOTHING_WITHOUT_HELD: &str =
    "This dot has more than its maker has, so the change cannot be saved without that. Allow \
     it, or don't change it.";

/// What Allow is told, and the card says, of a request that would now give something its card
/// did not show (`Makers::make_stands`, `Makers::change_stands`, `Makers::job_stands`).
/// `maker` is the maker's name. `gained` is the made dot's name when it was the dot that was
/// given more since the maker asked (`gained_since`); `None` when the maker was narrowed.
fn reach_shrank(kind: RequestKind, maker: &str, gained: Option<&str>) -> String {
    let dont = match kind {
        RequestKind::Make => "Don't make it",
        RequestKind::Change => "Don't change it",
        RequestKind::Job => "Don't give it",
    };
    match gained {
        Some(dot) => format!(
            "{dot} was given more since {maker} asked, so it has more than its maker can give. \
             {dont}, and let it ask again."
        ),
        None => {
            format!("{maker}'s own settings changed since it asked. {dont}, and let it ask again.")
        }
    }
}

/// `dot`, the made dot's name, when one of `now`, what a request for it would give now, was not
/// on the card of `request` and is something the dot was not given when it was asked
/// (`spec_holds`): the dot was given more since. `None` when every such extra is one the dot
/// had, so it is the maker that has less now.
fn gained_since(request: &MakeRequest, now: &[Extra], dot: &str) -> Option<String> {
    let seen = |extra: &Extra| request.extras.iter().any(|seen| same_thing(extra, seen));
    now.iter()
        .filter(|extra| !seen(extra))
        .any(|extra| !spec_holds(&request.spec, extra))
        .then(|| dot.to_string())
}

/// What the person is told of a job request whose made dot named `dot` is switched off.
fn switched_off(dot: &str) -> String {
    format!("\"{dot}\" is switched off. Turn it on to allow the job.")
}

/// What an answer that makes the dot is told of a request that passes a limit now that the
/// card it answered did not show: the card read again shows it.
fn limit_reached(over: &OverLimit) -> String {
    format!(
        "A limit was reached since the card was shown: it would be the {}. Look at the card \
         again, and answer again if you still want this dot.",
        over.line()
    )
}

/// The ask card's title for a new dot (design section 7.2), by what is beyond reach.
fn make_title(maker: &str, extras: &[Extra]) -> String {
    if extras.is_empty() {
        format!("{maker} wants to make one more dot")
    } else {
        format!("{maker} wants to make a dot with more than it has")
    }
}

/// The ask card's lines (design section 7.2): each extra in plain words, then the limit when
/// one is passed.
fn card_lines(maker: &str, extras: &[Extra], over: Option<&OverLimit>) -> Vec<String> {
    let mut lines: Vec<String> = extras.iter().map(|e| e.line(maker)).collect();
    lines.extend(over.map(OverLimit::line));
    lines
}

/// The folder to make before `reach.without` is saved: `create_folder` when the folder asked
/// for is no extra, so `without` still works in it. When the folder was the extra, `without`
/// has a blank one and gets a new folder of its own.
fn without_folder(reach: &Reach) -> Option<PathBuf> {
    let same =
        !reach.without.workdir.trim().is_empty() && reach.without.workdir == reach.spec.workdir;
    reach.create_folder.clone().filter(|_| same)
}

/// A make request held against the maker as it is now (`Makers::make_stands`).
struct MakeStanding {
    /// The dot asked for, put against the maker as it is now: `spec` is what Allow makes,
    /// `without` what Make it without these makes, and `extras` what the card shows.
    reach: Reach,
    /// The limit one more dot of the maker, made by the run that asked, would pass now.
    over: Option<OverLimit>,
    /// Allow would give the dot something beyond the maker's reach that the card the person
    /// saw did not show: the maker's own settings were narrowed since it asked.
    beyond_what_was_seen: bool,
}

/// Whether a make request can still be allowed.
#[allow(clippy::large_enum_variant)]
enum MakeStands {
    Yes(MakeStanding),
    /// What it names that is no longer there, in one sentence for the person.
    Gone(String),
}

/// Whether a job request can still be allowed (`Makers::job_stands`).
#[allow(clippy::large_enum_variant)]
enum JobStands {
    Yes {
        /// The made dot as it is now.
        dot: Dot,
        /// What it holds beyond the maker's reach as the maker is now: what the card shows.
        held: Vec<Extra>,
        /// It holds something the card the person saw did not show.
        beyond_what_was_seen: bool,
    },
    /// What has gone, in one sentence for the person.
    Gone(String),
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
    // A job for a dot the maker made that holds more than the maker has (design section 8).
    Job => "job",
});

impl RequestKind {
    /// What follows the reason when a request of this kind that names something that has gone
    /// is allowed.
    fn only_refuse_left(self) -> &'static str {
        match self {
            Self::Make => ONLY_REFUSE_LEFT,
            Self::Change => ONLY_REFUSE_CHANGE_LEFT,
            Self::Job => ONLY_REFUSE_JOB_LEFT,
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
    /// The made dot a change or a job is asked for; `None` for a make.
    pub dot_id: Option<String>,
    /// The name of the dot asked for; for a change, the made dot's name as the change leaves
    /// it; for a job, the made dot's name when it was asked.
    pub name: String,
    /// One line on what the dot is for, as `checked_purpose` gives it; for a change, the new
    /// purpose, or the dot's own when the change names none.
    pub purpose: String,
    /// The dot as asked: what Allow makes. For a change, the made dot with the change on it
    /// as both were when it was asked: the card's lines come from it, and an answer puts `ask`
    /// on the dot as it is then instead (`Makers::change_stands`). For a job, the made dot as
    /// it was when it was asked.
    pub spec: DotSpec,
    /// The same with the extras removed: what Make it without these makes.
    pub without: DotSpec,
    /// What lies beyond the maker's reach; empty when only a limit is passed. For a job, what
    /// the made dot held beyond it.
    pub extras: Vec<Extra>,
    /// The limit that would be passed, with the numbers as they were when it was asked.
    pub over: Option<OverLimit>,
    /// The folder of `spec` to make first, when it lies below the maker's and does not exist
    /// (`Reach::create_folder`).
    pub create_folder: Option<String>,
    /// The first job given with the request; for a request of the kind `job`, the job itself.
    pub first_job: Option<String>,
    pub status: RequestStatus,
    /// What has gone, in one sentence for the person, when `status` is `gone`.
    pub gone_reason: Option<String>,
    pub created_at: String,
    /// When the person answered.
    pub decided_at: Option<String>,
    /// The dot an answer made, or the made dot an answer changed or gave the job to; `None`
    /// once that dot is deleted.
    pub made_dot_id: Option<String>,
    /// The change as the maker asked it, for a request of the kind `change`; `None` for a
    /// make.
    #[serde(default)]
    pub ask: Option<ChangeDot>,
    /// The new dot as the maker asked it, for a request of the kind `make`: the card and the
    /// answer put it against the maker as it is then (`Makers::make_stands`). `None` for a
    /// change, and for a request stored before it was kept. The card shows what comes of it,
    /// so it does not cross to the UI.
    #[serde(skip)]
    pub make_ask: Option<MakeDot>,
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
    /// The new dot as asked, for a request of the kind `make`.
    pub make_ask: Option<&'a MakeDot>,
}

impl MakeRequest {
    /// The ask card's title (design sections 7.2 and 9). `maker` is the maker's name. A
    /// change's title names the made dot as the maker named it when it asked; the card names
    /// it as it is called now (`Makers::request_card`).
    pub fn title(&self, maker: &str) -> String {
        match self.kind {
            RequestKind::Change => {
                let dot = self.ask.as_ref().map_or(self.name.as_str(), |ask| &ask.dot);
                change_title(maker, dot.trim())
            }
            RequestKind::Job => job_title(maker, &self.name),
            RequestKind::Make => make_title(maker, &self.extras),
        }
    }

    /// The ask card's lines as the request was stored (design section 7.2): each extra in
    /// plain words, then the limit when one is passed. The card's own lines are of things as
    /// they are when it is read (`MakeRequestCard::lines`).
    pub fn lines(&self, maker: &str) -> Vec<String> {
        card_lines(maker, &self.extras, self.over.as_ref())
    }

    /// The request as it was stored, as the reach check gives it.
    fn stored_reach(&self) -> Reach {
        Reach {
            spec: self.spec.clone(),
            without: self.without.clone(),
            extras: self.extras.clone(),
            create_folder: self.create_folder.as_ref().map(PathBuf::from),
        }
    }
}

/// The title of the ask card for a change to the made dot named `dot` (design section 9).
fn change_title(maker: &str, dot: &str) -> String {
    format!("{maker} wants to give {dot} more than it has")
}

/// The title of the ask card for a change that gives the made dot named `dot` nothing beyond
/// reach itself, while the dot holds more than its maker has (design section 9): the person
/// decides what such a dot is told and where it works.
fn held_title(maker: &str, dot: &str) -> String {
    format!("{maker} wants to change {dot}, which has more than {maker} has")
}

/// The title of the ask card for a job for the made dot named `dot`, which holds more than its
/// maker has (design section 8).
fn job_title(maker: &str, dot: &str) -> String {
    format!("{maker} wants to give a job to {dot}, which has more than {maker} has")
}

/// What came of the person's answer to a request, under the service's lock (`Makers::answer`):
/// what the maker is woken with comes from it (`answer_message`).
pub struct Answer {
    /// The request as it is after.
    pub request: MakeRequest,
    /// The maker's name.
    pub maker_name: String,
    /// What came of the first job given with a make request, when the answer made the dot.
    pub first_job: Option<FirstJob>,
    /// What "without" left out, as the request stood when it was answered.
    pub left_out: Vec<Extra>,
    /// The made dot's name as it is now, for a job request.
    pub dot_name: String,
}

/// What the maker is woken with after the person's answer `decision` to `request`, a request
/// of the kind `change` (design section 9). `maker` is the maker's name; `left_out` what
/// "without" left out.
fn change_answer_message(
    request: &MakeRequest,
    decision: MakeDecision,
    maker: &str,
    left_out: &[Extra],
) -> String {
    let name = &request.name;
    match decision {
        MakeDecision::Allow => {
            format!("The person allowed it: \"{name}\" was changed as you asked.")
        }
        MakeDecision::Without => {
            let extras: Vec<String> = left_out.iter().map(|e| e.line(maker)).collect();
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

/// What the maker is woken with after the person's answer `decision` to a job request for the
/// made dot named `dot` (design section 8).
fn job_answer_message(decision: MakeDecision, dot: &str) -> String {
    match decision {
        MakeDecision::Refuse => format!(
            "The person chose not to give \"{dot}\" the job. Do not give it again; carry on \
             without it, or tell the person what you cannot do."
        ),
        _ => format!(
            "The person allowed the job for \"{dot}\": it has started. Do not wait for it; you \
             will be woken with its result."
        ),
    }
}

/// What the maker is woken with after the person's answer `decision` (design sections 7.2, 8
/// and 9). The maker's name is `answer.maker_name`. What came of the first job given with a
/// make request, when the answer made the dot, ends the message (`FirstJob::sentence`). For
/// "without", the message names what was left out as the request stood when it was answered.
pub fn answer_message(answer: &Answer, decision: MakeDecision) -> String {
    let (request, maker) = (&answer.request, answer.maker_name.as_str());
    match request.kind {
        RequestKind::Change => {
            return change_answer_message(request, decision, maker, &answer.left_out)
        }
        RequestKind::Job => return job_answer_message(decision, &answer.dot_name),
        RequestKind::Make => {}
    }
    let name = &request.name;
    let mut message = match decision {
        MakeDecision::Allow => {
            format!("The person allowed it: the dot \"{name}\" was made as you asked.")
        }
        MakeDecision::Without => {
            let extras: Vec<String> = answer.left_out.iter().map(|e| e.line(maker)).collect();
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
    if let Some(first_job) = &answer.first_job {
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
    /// The dot's name now: the new dot asked for, or the made dot a change or a job is for, as
    /// it is called now.
    #[serde(default)]
    pub name: String,
    pub title: String,
    /// The extras in plain words, one per line, then the limit when one is passed: as things
    /// are when the card is read, against the maker as it is then.
    pub lines: Vec<String>,
    /// For a change, every setting the ask names, one short line each ("New instructions",
    /// "New name: Feeds", "Adds the skill 'pdf'"), so Allow covers nothing the card did not
    /// say. Empty for a new dot, whose card shows the dot itself.
    #[serde(default)]
    pub asked: Vec<String>,
    /// The instructions in full: those of the new dot, or the new ones a change gives the dot.
    /// `None` for a change that does not name them.
    #[serde(default)]
    pub instructions: Option<String>,
    /// The schedule in plain words (`schedule_words`): that of the new dot, or the one a
    /// change gives the dot, "no schedule" when it clears it. `None` for a new dot without
    /// one and for a change that does not name it.
    #[serde(default)]
    pub schedule: Option<String>,
    /// The limit line `lines` ends with, when a limit would be passed now (`OverLimit::line`):
    /// the answer to this card sends it back, so an answer that makes the dot is refused when
    /// a limit was reached since the card was read (`Makers::decide_request`).
    #[serde(default)]
    pub limit: Option<String>,
    /// Whether Allow for this dot is on.
    pub can_allow: bool,
    /// Whether Make it without these is on. Off when the only problem is a limit.
    pub can_without: bool,
    /// Why Allow is off, for a request that has no answer yet: what this PC's role in the
    /// maker's account does not allow, or what has gone.
    pub blocked: Option<String>,
}

/// A make request, or a request to change a made dot, that waits for the person, as the Inbox
/// lists it (design section 11): what the Inbox sorts, links and titles it by. The whole card,
/// with its buttons, is `Makers::request_card`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMakeRequest {
    /// The request's id.
    pub id: String,
    /// The maker that asked.
    pub dot_id: String,
    /// The maker's name.
    pub dot_name: String,
    /// The maker's chat topic, where the ask card shows.
    pub root_run_id: String,
    pub kind: RequestKind,
    /// The name of the dot asked for, or of the made dot a change or a job is for, as it is
    /// called now.
    pub name: String,
    pub purpose: String,
    /// The ask card's title (`MakeRequest::title`).
    pub title: String,
    /// The ask card's lines (`MakeRequest::lines`).
    pub lines: Vec<String>,
    /// `waiting`, or `gone` for one that only "Don't make it" can still answer.
    pub status: RequestStatus,
    pub created_at: String,
}

impl PendingMakeRequest {
    /// The Inbox's item for the ask card `card`: its title and lines are the card's, so the
    /// Inbox and the card always say the same.
    pub fn of(card: &MakeRequestCard) -> Self {
        let request = &card.request;
        Self {
            id: request.id.clone(),
            dot_id: request.maker_dot_id.clone(),
            dot_name: card.maker_name.clone(),
            root_run_id: request.root_run_id.clone(),
            kind: request.kind,
            name: card.name.clone(),
            purpose: request.purpose.clone(),
            title: card.title.clone(),
            lines: card.lines.clone(),
            status: request.status,
            created_at: request.created_at.clone(),
        }
    }

    /// The Inbox's item for `request`, whose maker is named `maker`, as it was stored: for
    /// when its card cannot be read just now.
    pub fn stored(request: &MakeRequest, maker: &str) -> Self {
        Self {
            id: request.id.clone(),
            dot_id: request.maker_dot_id.clone(),
            dot_name: maker.to_string(),
            root_run_id: request.root_run_id.clone(),
            kind: request.kind,
            name: request.name.clone(),
            purpose: request.purpose.clone(),
            title: request.title(maker),
            lines: request.lines(maker),
            status: request.status,
            created_at: request.created_at.clone(),
        }
    }
}

impl Makers {
    /// The ask cards of every request the person has not answered, of every maker, newest
    /// first: the Inbox's section (design section 11). Each is checked again as `request_card`
    /// checks it. A request that went while the list was read is left out.
    pub async fn waiting_cards(&self) -> Result<Vec<MakeRequestCard>> {
        let mut cards = Vec::new();
        let waiting = self.parts.store.list_make_requests(None, true).await?;
        for request in waiting.iter().rev() {
            match self.request_card(&request.id).await {
                Ok(card) => cards.push(card),
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(cards)
    }

    /// The ask card of a request (design section 7.2), judged against the maker as it is when
    /// the person looks. A waiting request is checked again against the Library, the vault, the
    /// folders and the dots as they are now: when something it names has gone, or its name is
    /// taken, it is marked `gone`, `blocked` says what has gone and only Don't make it is left.
    /// A request once marked stays so.
    ///
    /// What the maker asked is put against the maker as it is now (`make_stands`): the lines,
    /// the title and Make it without these come from that, and the limit line from the limits
    /// as they are now (`limit`). Reading a card writes no limit: the answer to it sends back
    /// the limit line the card showed. When the maker was narrowed since it asked, so that
    /// Allow would give the dot something the request did not show at first, Allow is off and
    /// `blocked` says so.
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
        match request.kind {
            RequestKind::Change => return self.change_card(request, maker).await,
            RequestKind::Job => return self.job_request_card(request, maker).await,
            RequestKind::Make => {}
        }
        let maker_name = maker.spec.name.clone();
        let mut asked = request.stored_reach();
        let mut over = request.over.clone();
        let (mut can_allow, mut can_without, mut blocked) = (false, false, None);
        if request.status == RequestStatus::Waiting {
            match self.make_stands(&maker, &request).await? {
                MakeStands::Gone(reason) => request = self.mark_gone(&request, &reason).await?,
                MakeStands::Yes(standing) => {
                    over = standing.over.clone();
                    if standing.beyond_what_was_seen {
                        blocked = Some(reach_shrank(RequestKind::Make, &maker_name, None));
                    } else {
                        match self.may_answer(&maker, &standing.reach.spec).await {
                            Ok(()) => can_allow = true,
                            Err(Error::NotAllowed(why)) => blocked = Some(why),
                            Err(e) => return Err(e),
                        }
                    }
                    if !standing.reach.extras.is_empty() {
                        let without = &standing.reach.without;
                        can_without = match self.may_answer(&maker, without).await {
                            Ok(()) => true,
                            Err(Error::NotAllowed(_)) => false,
                            Err(e) => return Err(e),
                        };
                    }
                    asked = standing.reach;
                }
            }
        }
        if request.status == RequestStatus::Gone {
            blocked = Some(
                request
                    .gone_reason
                    .clone()
                    .unwrap_or_else(|| SOMETHING_GONE.to_string()),
            );
        }
        Ok(MakeRequestCard {
            title: make_title(&maker_name, &asked.extras),
            lines: card_lines(&maker_name, &asked.extras, over.as_ref()),
            limit: over.as_ref().map(OverLimit::line),
            asked: Vec::new(),
            instructions: Some(asked.spec.instructions.clone()),
            schedule: asked.spec.schedule.as_ref().map(schedule_words),
            name: request.name.clone(),
            maker_name,
            request,
            can_allow,
            can_without,
            blocked,
        })
    }

    /// Holds the make request `request` of `maker` against things as they are now (design
    /// section 7.2), for its card and for the person's answer: what has gone since it was
    /// asked (`gone_since`), the dot the maker asked put against the maker as it is now
    /// (`reach_new`), and the limits as they are now (`over_limit`).
    ///
    /// The extras found now are all covered by Allow when each of them was among those the
    /// request was stored with (`same_thing`). When one is new, because the maker's own
    /// settings were narrowed since it asked, `beyond_what_was_seen` is set. A request stored
    /// before the ask itself was kept is held as it was stored.
    async fn make_stands(&self, maker: &Dot, request: &MakeRequest) -> Result<MakeStands> {
        if let Some(reason) = self.gone_since(maker, request).await? {
            return Ok(MakeStands::Gone(reason));
        }
        let reach = match &request.make_ask {
            Some(ask) => match reach_new(maker, &ask.wanted, &self.parts.worktrees_root) {
                Ok(reach) => reach,
                // A dot the ask no longer fits, as a folder outside the maker's that went.
                Err(Error::Invalid(_)) => return Ok(MakeStands::Gone(SOMETHING_GONE.into())),
                Err(e) => return Err(e),
            },
            None => request.stored_reach(),
        };
        let over = self.over_limit(maker, &request.run_id).await?;
        let seen = |extra: &Extra| request.extras.iter().any(|seen| same_thing(extra, seen));
        let beyond_what_was_seen = !reach.extras.iter().all(seen);
        Ok(MakeStands::Yes(MakeStanding {
            reach,
            over,
            beyond_what_was_seen,
        }))
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
        let mut lines = request.lines(&maker_name);
        // What the ask names, on the dot as it was asked; on the dot as it is now while the
        // request waits.
        let named = |ask: Option<&ChangeDot>, before: &DotSpec, after: &DotSpec| match ask {
            Some(ask) => change_lines(ask, before, after, &maker_name),
            None => Vec::new(),
        };
        let mut asked = named(request.ask.as_ref(), &request.spec, &request.spec);
        // The dot as it is called now, while it exists; else as the maker named it.
        let mut dot_name = request
            .ask
            .as_ref()
            .map_or(request.name.as_str(), |ask| ask.dot.trim())
            .to_string();
        let (mut can_allow, mut can_without, mut blocked) = (false, false, None);
        match request.status {
            RequestStatus::Waiting => match self.change_stands(&maker, &request).await? {
                Stands::Gone(reason) => {
                    request = self.mark_gone(&request, &reason).await?;
                    // The dot's name now, while it exists, as on every later read.
                    if let Some(dot_id) = &request.dot_id {
                        if let Ok(dot) = self.parts.store.get_dot(dot_id).await {
                            title = change_title(&maker_name, &dot.spec.name);
                            dot_name = dot.spec.name;
                        }
                    }
                    blocked = Some(reason);
                }
                Stands::Yes(standing) => {
                    let name = &standing.dot.spec.name;
                    dot_name = name.clone();
                    title = if standing.held && standing.reach.extras.is_empty() {
                        held_title(&maker_name, name)
                    } else {
                        change_title(&maker_name, name)
                    };
                    lines = card_lines(&maker_name, &standing.extras, None);
                    asked = named(
                        request.ask.as_ref(),
                        &standing.dot.spec,
                        &standing.reach.spec,
                    );
                    if standing.beyond_what_was_seen {
                        let gained = gained_since(&request, &standing.extras, name);
                        blocked = Some(reach_shrank(
                            RequestKind::Change,
                            &maker_name,
                            gained.as_deref(),
                        ));
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
                        dot_name = dot.spec.name;
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
        let wanted = request.ask.as_ref().map(|ask| &ask.wanted);
        let instructions = wanted.and_then(|w| w.instructions.clone());
        let schedule = wanted.and_then(|w| w.schedule.as_ref()).map(|s| match s {
            Some(schedule) => schedule_words(schedule),
            None => "no schedule".to_string(),
        });
        Ok(MakeRequestCard {
            title,
            lines,
            limit: None,
            asked,
            instructions,
            schedule,
            name: dot_name,
            maker_name,
            request,
            can_allow,
            can_without,
            blocked,
        })
    }

    /// Holds the job request `request` of `maker` against things as they are now (design
    /// section 8), for its card and for the person's answer: the made dot as it is now, and
    /// what it holds beyond the maker's reach as the maker is now (`held_now`).
    /// `beyond_what_was_seen` is set when it holds something the card the person saw did not
    /// show. `Gone` when the made dot no longer exists or is no longer this maker's.
    async fn job_stands(&self, maker: &Dot, request: &MakeRequest) -> Result<JobStands> {
        let gone = || {
            Ok(JobStands::Gone(format!(
                "{} no longer exists.",
                request.name
            )))
        };
        let Some(dot_id) = &request.dot_id else {
            return Ok(JobStands::Gone(SOMETHING_GONE.to_string()));
        };
        let dot = match self.parts.store.get_dot(dot_id).await {
            Ok(dot) => dot,
            Err(Error::NotFound(_)) => return gone(),
            Err(e) => return Err(e),
        };
        let makers = dot.made.as_ref().and_then(|made| made.by_dot_id.as_deref());
        if makers != Some(maker.id.as_str()) {
            return gone();
        }
        let held = self.held_now(maker, &dot).await?;
        let seen = |extra: &Extra| request.extras.iter().any(|seen| same_thing(extra, seen));
        let beyond_what_was_seen = !held.iter().all(seen);
        Ok(JobStands::Yes {
            dot,
            held,
            beyond_what_was_seen,
        })
    }

    /// The ask card of a job request (design section 8), under the service's lock. Its title
    /// names the made dot as it is called now, its lines are what the dot holds beyond the
    /// maker's reach, and the job itself is `request.first_job`, in full. There is no
    /// "without": Allow starts the job as asked, Don't give it wakes the maker. Allow is on
    /// when the dot holds nothing the card did not show, is switched on, and this PC may run
    /// it.
    async fn job_request_card(
        &self,
        mut request: MakeRequest,
        maker: Dot,
    ) -> Result<MakeRequestCard> {
        let maker_name = maker.spec.name.clone();
        let mut name = request.name.clone();
        if let Some(dot_id) = &request.dot_id {
            if let Ok(dot) = self.parts.store.get_dot(dot_id).await {
                name = dot.spec.name;
            }
        }
        let mut lines = request.lines(&maker_name);
        let (mut can_allow, mut blocked) = (false, None);
        match request.status {
            RequestStatus::Waiting => match self.job_stands(&maker, &request).await? {
                JobStands::Gone(reason) => {
                    request = self.mark_gone(&request, &reason).await?;
                    blocked = Some(reason);
                }
                JobStands::Yes {
                    dot,
                    held,
                    beyond_what_was_seen,
                } => {
                    lines = card_lines(&maker_name, &held, None);
                    if beyond_what_was_seen {
                        let gained = gained_since(&request, &held, &name);
                        blocked = Some(reach_shrank(
                            RequestKind::Job,
                            &maker_name,
                            gained.as_deref(),
                        ));
                    } else if !dot.spec.enabled {
                        blocked = Some(switched_off(&name));
                    } else {
                        match self.parts.runner.require_run(&dot).await {
                            Ok(()) => can_allow = true,
                            Err(Error::NotAllowed(why)) => blocked = Some(why),
                            Err(e) => return Err(e),
                        }
                    }
                }
            },
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
            title: job_title(&maker_name, &name),
            lines,
            limit: None,
            asked: Vec::new(),
            instructions: None,
            schedule: None,
            name,
            maker_name,
            request,
            can_allow,
            can_without: false,
            blocked,
        })
    }

    /// The person answers a job request (design section 8), under the service's lock: the
    /// request as it is after, the maker's name and the made dot's name now.
    ///
    /// Allow starts the job as the maker gave it, as a job of the run that asked, in that
    /// run's topic (`start_job`), without the loop guard: the person gave it. It is refused
    /// (`Conflict`) when the dot has gone (the request is marked `gone`), holds something the
    /// card did not show, or is switched off; and as `start_job` refuses, while the last job
    /// from that topic to the dot has not ended. Then the request still waits. There is
    /// nothing to give without (`Invalid`).
    async fn answer_job(
        &self,
        request: MakeRequest,
        decision: MakeDecision,
    ) -> Result<(MakeRequest, String, String)> {
        let p = &self.parts;
        let maker = p.store.get_dot(&request.maker_dot_id).await?;
        let mut dot = match &request.dot_id {
            Some(dot_id) => p.store.get_dot(dot_id).await.ok(),
            None => None,
        };
        match decision {
            MakeDecision::Refuse => {}
            MakeDecision::Without => return Err(Error::Invalid(NOTHING_WITHOUT_JOB.into())),
            MakeDecision::Allow => {
                let (made, held, beyond) = match self.job_stands(&maker, &request).await? {
                    JobStands::Yes {
                        dot,
                        held,
                        beyond_what_was_seen,
                    } => (dot, held, beyond_what_was_seen),
                    JobStands::Gone(reason) => {
                        self.mark_gone(&request, &reason).await?;
                        return Err(Error::Conflict(format!("{reason} {ONLY_REFUSE_JOB_LEFT}")));
                    }
                };
                let name = &made.spec.name;
                if beyond {
                    let gained = gained_since(&request, &held, name);
                    return Err(Error::Conflict(reach_shrank(
                        RequestKind::Job,
                        &maker.spec.name,
                        gained.as_deref(),
                    )));
                }
                if !made.spec.enabled {
                    return Err(Error::Conflict(switched_off(name)));
                }
                let message = request.first_job.as_deref().unwrap_or_default();
                self.start_job(
                    &maker,
                    &request.run_id,
                    &request.root_run_id,
                    &made,
                    message,
                    false,
                )
                .await?;
                dot = Some(made);
            }
        }
        let dot_name = dot
            .as_ref()
            .map_or(request.name.clone(), |d| d.spec.name.clone());
        let dot_id = dot.as_ref().map(|d| d.id.as_str());
        let given = match decision {
            MakeDecision::Allow => dot_id,
            _ => None,
        };
        self.record_answer(&request, decision, given, dot_id, &dot_name)
            .await?;
        let given = given.map(str::to_string);
        let request = self.answered(&request, decision, given).await?;
        Ok((request, maker.spec.name, dot_name))
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
    ) -> Result<(MakeRequest, String, Vec<Extra>)> {
        let p = &self.parts;
        let maker = p.store.get_dot(&request.maker_dot_id).await?;
        let mut left_out = Vec::new();
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
                    let name = &standing.dot.spec.name;
                    let gained = gained_since(&request, &standing.extras, name);
                    return Err(Error::Conflict(reach_shrank(
                        RequestKind::Change,
                        &maker.spec.name,
                        gained.as_deref(),
                    )));
                }
                if decision == MakeDecision::Without && standing.held {
                    return Err(Error::Invalid(NOTHING_WITHOUT_HELD.into()));
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
                if decision == MakeDecision::Without {
                    left_out = reach.extras.clone();
                }
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
        let changed_id = changed_id.map(str::to_string);
        self.record_answer(
            &request,
            decision,
            changed_id.as_deref(),
            dot.as_ref().map(|d| d.id.as_str()),
            dot_name,
        )
        .await?;
        let request = self.answered(&request, decision, changed_id).await?;
        Ok((request, maker.spec.name, left_out))
    }

    /// `request` as it is after the person's answer `decision`, which made or changed the dot
    /// `made_dot_id`. When its row went in between, with its maker, the request as answered is
    /// built here: the answer was carried out, so it is not reported as not found.
    async fn answered(
        &self,
        request: &MakeRequest,
        decision: MakeDecision,
        made_dot_id: Option<String>,
    ) -> Result<MakeRequest> {
        match self.parts.store.get_make_request(&request.id).await {
            Err(Error::NotFound(_)) => Ok(MakeRequest {
                status: match decision {
                    MakeDecision::Allow => RequestStatus::Allowed,
                    MakeDecision::Without => RequestStatus::MadeWithout,
                    MakeDecision::Refuse => RequestStatus::Refused,
                },
                decided_at: Some(crate::util::now()),
                made_dot_id,
                ..request.clone()
            }),
            stored => stored,
        }
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
    /// the dot, the answer is refused (`NotAllowed`) and the request still waits. What the maker
    /// asked is put against the maker as it is now, as the card does (`make_stands`): Allow is
    /// refused (`Conflict`) when that gives the dot something the request did not show, and
    /// the request still waits. `limit` is the limit line of the card the person answered
    /// (`MakeRequestCard::limit`), or `None` when it showed none: an answer that makes the dot
    /// is refused (`Conflict`) when it passes a limit now that is not that line, and the card
    /// read again shows it. Allow covers this one dot, also past a limit the card showed
    /// (section 10). When the dot cannot be created the request still waits and the error is
    /// returned. A job request (section 8) is answered by `answer_job`.
    ///
    /// The maker's switch is not looked at: when the person turned "Can make dots" off after
    /// the request was stored, Allow and Make it without these still make the dot, since it
    /// is the person who decides, knowing both.
    pub async fn decide_request(
        &self,
        request_id: &str,
        decision: MakeDecision,
        limit: Option<&str>,
    ) -> Result<MakeRequest> {
        let answer = {
            // With `make_dot`: two answers, or an answer and a make, cannot both pass.
            let _one_at_a_time = self.parts.one_at_a_time.lock().await;
            self.answer(request_id, decision, limit).await?
        };
        // After every lock is released: `wake` takes the thread lock itself.
        let message = answer_message(&answer, decision);
        let request = answer.request;
        if let Err(e) = self
            .wake(&request.maker_dot_id, &request.root_run_id, &message)
            .await
        {
            tracing::warn!(request = %request.id, "the maker was not woken with the answer: {e}");
        }
        Ok(request)
    }

    /// `decide_request` without the wake, under the service's lock (`Answer`): with what came
    /// of the first job given with the request, when the answer made the dot (design section
    /// 7.2): it starts once the dot is made, as the job of the run that asked, in that run's
    /// topic.
    async fn answer(
        &self,
        request_id: &str,
        decision: MakeDecision,
        limit: Option<&str>,
    ) -> Result<Answer> {
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
        match request.kind {
            RequestKind::Change => {
                let (request, maker_name, left_out) = self.answer_change(request, decision).await?;
                let dot_name = request.name.clone();
                return Ok(Answer {
                    request,
                    maker_name,
                    first_job: None,
                    left_out,
                    dot_name,
                });
            }
            RequestKind::Job => {
                let (request, maker_name, dot_name) = self.answer_job(request, decision).await?;
                return Ok(Answer {
                    request,
                    maker_name,
                    first_job: None,
                    left_out: Vec::new(),
                    dot_name,
                });
            }
            RequestKind::Make => {}
        }
        let maker = p.store.get_dot(&request.maker_dot_id).await?;
        let mut left_out = Vec::new();
        let made = match decision {
            MakeDecision::Refuse => None,
            MakeDecision::Allow | MakeDecision::Without => {
                // Against the maker and the limits as they are now, as the card shows them.
                let standing = match self.make_stands(&maker, &request).await? {
                    MakeStands::Yes(standing) => standing,
                    MakeStands::Gone(reason) => {
                        self.mark_gone(&request, &reason).await?;
                        return Err(Error::Conflict(format!("{reason} {ONLY_REFUSE_LEFT}")));
                    }
                };
                if decision == MakeDecision::Without && standing.reach.extras.is_empty() {
                    return Err(Error::Invalid(NOTHING_BEYOND_REACH.into()));
                }
                if decision == MakeDecision::Allow && standing.beyond_what_was_seen {
                    return Err(Error::Conflict(reach_shrank(
                        RequestKind::Make,
                        &maker.spec.name,
                        None,
                    )));
                }
                // A limit passed now that the card the person answered did not show: the card
                // read again shows it, and the person answers again.
                if let Some(over) = &standing.over {
                    if limit != Some(over.line().as_str()) {
                        self.updated(&request.maker_dot_id, &request.root_run_id);
                        return Err(Error::Conflict(limit_reached(over)));
                    }
                }
                let without_folder = without_folder(&standing.reach);
                let reach = standing.reach;
                if decision == MakeDecision::Without {
                    left_out = reach.extras.clone();
                }
                let (spec, folder) = match decision {
                    MakeDecision::Allow => (reach.spec, reach.create_folder),
                    _ => (reach.without, without_folder),
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
        let made_id = made_id.map(str::to_string);
        let request = self.answered(&request, decision, made_id).await?;
        let dot_name = request.name.clone();
        Ok(Answer {
            request,
            maker_name: maker.spec.name,
            first_job,
            left_out,
            dot_name,
        })
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
    /// when all of it is (design section 7.2): its name taken by a dot, a folder outside the
    /// maker's that it asked for, a Library item, an MCP server its variables are for, or a
    /// variable.
    async fn gone_since(&self, maker: &Dot, request: &MakeRequest) -> Result<Option<String>> {
        if self.parts.store.dot_name_taken(request.name.trim()).await? {
            return Ok(Some(format!(
                "A dot named {:?} exists now.",
                request.name.trim()
            )));
        }
        // A folder outside is never made for a dot: it must still be there.
        for extra in &request.extras {
            if let Extra::Folder {
                path,
                below_maker: false,
            } = extra
            {
                if !Path::new(path).is_dir() {
                    return Ok(Some(format!("The folder {path} no longer exists.")));
                }
            }
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

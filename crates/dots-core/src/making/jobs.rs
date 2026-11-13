//! Jobs (dots-make-dots design section 8): a maker gives a job to a dot it made, which starts a
//! chat run of that dot, and is woken once when the job ends. One job covers the run it starts,
//! the runs that resume that run after approvals, and, when a run ends its turn by asking the
//! person a question, the run that carries their answer, and so on until a run ends with no
//! question waiting. Follow-ups the made dot schedules for later are not part of it.
//!
//! A job is followed from the store alone (`Makers::settle_jobs`): each pass reads where every
//! open job's last run stands and moves, parks or ends the job. So nothing is lost when Dots
//! restarts or an event is missed, and a pass may run any number of times.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::log::{MakingEventKind, NewMakingEvent};
use super::reach::Extra;
use super::requests::{MakeRequest, NewMakeRequest, RequestKind};
use super::service::{Makers, MAX_JOB};
use crate::limits::{run_is_background, BACKGROUND, STOPPED_BY_LIMIT};
use crate::model::{ApprovalStatus, Dot, NewRun, Run, RunStatus, TriggerKind, WaitReason};
use crate::prompt::{is_typed, JOB_FROM};
use crate::spend::{Ledger, STOPPED_BY_CAP};
use crate::util::new_id;
use crate::{Error, Result};

/// The most characters of a job's final text a wake carries (design section 8); a longer one
/// is cut there, with `RESULT_CUT` after it.
pub const MAX_JOB_RESULT: usize = 8000;
/// The most jobs given in one chat topic of a maker between two messages the person typed
/// (design section 8, the loop guard). First jobs count.
pub const MAX_JOBS_BETWEEN_TYPED: u32 = 6;
/// The key of a job run's payload that holds the job's id: the run's message is a maker's, not
/// one the person typed.
pub const JOB_ID: &str = "job_id";
/// What `give_job` is told for a job that is not 1 to `MAX_JOB` characters.
pub const JOB_RULE: &str = "The job is 1 to 4000 characters.";
/// What follows a final text that was cut at `MAX_JOB_RESULT` characters.
pub const RESULT_CUT: &str = "[cut: the full text is in the run]";
/// What a wake says of a job whose last run left no final text.
pub const NO_FINAL_TEXT: &str = "It left no final message.";
/// What `list_made_dots` says of a made dot with an open job from the conversation that asks.
pub const JOB_WORKING: &str = "working on a job from this conversation";
/// What `list_made_dots` says of a made dot whose job from the conversation that asks waits
/// for the person's answer.
pub const JOB_WAITING: &str = "waiting for the person's answer";
/// What a first job that started adds to the tool's text and to the wake with the person's
/// answer (design sections 7.1 and 7.2).
pub const FIRST_JOB_STARTED: &str =
    " Its first job has started; you will be woken with the result.";

/// The most characters of a failed run's error a wake carries.
const MAX_ERROR: usize = 300;
/// The key in `LogEvery` of a pass's check of the jobs whose runs may be held.
const HELD_CHECK: &str = "jobs: held check";
/// The most runs one pass follows a job through: far more than a turn has.
const MAX_HOPS: usize = 200;

str_enum!(JobStatus {
    // Its run is queued, running or awaiting an approval.
    Working => "working",
    // Its last run ended with a question waiting for the person.
    WaitingAnswer => "waiting_answer",
    // It ended; the maker's wake is stored.
    Ended => "ended",
});

str_enum!(JobEnd {
    // Its last run finished.
    Succeeded => "succeeded",
    // Its last run failed: an error, the time limit, a spending cap, Claude's usage limit.
    Failed => "failed",
    // The person stopped its run, or dismissed the question it waited on.
    Cancelled => "cancelled",
    // Its maker deleted the made dot before it ended.
    Deleted => "deleted",
});

/// One job, as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakingJob {
    pub id: String,
    /// The maker that gave it.
    pub maker_dot_id: String,
    /// The maker's run that gave it; for a first job, the run that asked for the dot.
    pub maker_run_id: String,
    /// The maker's chat topic it was given in, where its cards show and the wake goes.
    pub maker_root_run_id: String,
    /// The made dot.
    pub dot_id: String,
    /// The made dot's chat topic the job runs in, as it was when the job started.
    pub dot_root_run_id: Option<String>,
    /// The run the job started.
    pub first_run_id: Option<String>,
    /// The run the job is at: it moves forward as the job goes on.
    pub last_run_id: Option<String>,
    pub message: String,
    pub status: JobStatus,
    /// How it ended; `None` until it has.
    pub ended_how: Option<JobEnd>,
    pub created_at: String,
    pub ended_at: Option<String>,
    /// The wake that tells the maker, stored when the job ends.
    pub wake_id: Option<String>,
}

/// Input for `Store::start_making_job`. The job is stored `working`, as of now.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMakingJob<'a> {
    /// The job's id, made beforehand: its run's payload names it.
    pub id: &'a str,
    pub maker_dot_id: &'a str,
    pub maker_run_id: &'a str,
    pub maker_root_run_id: &'a str,
    pub dot_id: &'a str,
    pub message: &'a str,
}

/// What came of the first job given with a dot (design sections 7.1 and 8). The dot is made
/// either way.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum FirstJob {
    Started(MakingJob),
    /// Why it was not started, in a sentence for the maker: the loop guard, or what failed.
    NotStarted(String),
}

impl FirstJob {
    /// What it adds to the tool's text and to the wake with the person's answer: it starts
    /// with a space.
    pub fn sentence(&self) -> String {
        match self {
            Self::Started(_) => FIRST_JOB_STARTED.to_string(),
            Self::NotStarted(why) => format!(" Its first job was not started: {why}"),
        }
    }
}

/// What the job past the loop guard is told (design section 8). `maker` is the maker's name.
pub fn guard_refusal(maker: &str) -> String {
    format!(
        "{maker} has given {MAX_JOBS_BETWEEN_TYPED} jobs since the person last wrote here. Stop \
         and report to the person what is done and what is left; they can tell you to carry on."
    )
}

/// What `give_job` tells the maker's run when the job was taken (design section 8).
pub fn gave_text(dot: &str) -> String {
    format!(
        "Gave the job to \"{dot}\". It has started (or waits behind its current run). Do not \
         wait for it: carry on with other work or end your turn, and you will be woken with its \
         result."
    )
}

/// What `give_job` tells the maker's run when the job waits for the person (design section 8):
/// the made dot named `dot` holds `held`, more than the maker has. `maker` is the maker's name.
pub fn job_waits_text(dot: &str, maker: &str, held: &[Extra]) -> String {
    let held: Vec<String> = held.iter().map(|e| e.line(maker)).collect();
    format!(
        "Not started yet: the job for \"{dot}\" needs the person's say-so, because \"{dot}\" has \
         more than you have ({}). The request is waiting for them. Do not wait for it and do not \
         give the job again: carry on with other work or end your turn, and you will be woken \
         with their answer.",
        held.join("; ")
    )
}

/// What stands in a wake for a value of a variable the made dot holds beyond its maker's reach.
pub const HIDDEN: &str = "[hidden]";

/// `text` with each of `values` (`Makers::held_values`, longest first) replaced by `HIDDEN`.
fn hide(text: &str, values: &[String]) -> String {
    let mut out = text.to_string();
    for value in values {
        if out.contains(value.as_str()) {
            out = out.replace(value.as_str(), HIDDEN);
        }
    }
    out
}

/// What a wake says in place of the job's result when the values the made dot holds beyond its
/// maker's reach cannot be read (`Makers::values_to_hide`): nothing could be hidden, so nothing
/// of the made dot's text goes back.
pub const WITHHELD: &str = "Its final message and error text were left out: Dots could not check \
                            them for the values this dot holds beyond yours.";

/// What the maker is woken with when a job of the dot `dot` ended as `how` and its result is
/// left out (`WITHHELD`).
pub fn withheld_message(dot: &str, how: &str) -> String {
    format!("\"{dot}\" {how}.\n\n{WITHHELD}")
}

/// How a job ended whose last run parked on a call the person refused, with no note.
pub const REFUSED: &str = "was stopped: the person refused what it asked to do";

/// What the maker is woken with, once, when the run of a job given to the dot `dot` waits:
/// `why` says what it waits for, as the run's waiting line does.
pub fn held_message(dot: &str, why: &str) -> String {
    format!(
        "\"{dot}\" has not started the job: {why}. It starts by itself when it may; you will be \
         woken when the job ends."
    )
}

/// What a wake says of a job whose made dot the maker deleted before it ended.
pub fn deleted_message(dot: &str) -> String {
    format!("\"{dot}\" was deleted before the job ended.")
}

/// Whether the chat topic whose runs are `thread` is unattended (design section 8): the person
/// never typed in it, and its first run is background (`limits::run_is_background`), as a
/// schedule's or a watcher's is. A topic the person started with Run now, or one an app
/// started, is watched. The jobs given in an unattended topic, and its wakes, are background.
pub(crate) fn unattended(thread: &[Run]) -> bool {
    let root = thread
        .iter()
        .find(|run| run.id == run.root_run_id)
        .or(thread.first());
    !thread.iter().any(is_typed) && root.is_some_and(run_is_background)
}

/// What a job for the made dot named `name` is refused with while `earlier`, the last job from
/// the same topic of the maker to that dot, has not ended: a topic is one line of runs, so the
/// next job cannot be queued behind it. `None` when there is none or it ended.
fn still_open(name: &str, earlier: Option<&MakingJob>) -> Option<Error> {
    match earlier.map(|job| job.status) {
        Some(JobStatus::Working) => Some(Error::Invalid(format!(
            "\"{name}\" is still working on the last job from this conversation. You will be \
             woken when it ends."
        ))),
        Some(JobStatus::WaitingAnswer) => Some(Error::Invalid(format!(
            "\"{name}\" is waiting for the person's answer on the last job from this \
             conversation. You will be woken when it ends."
        ))),
        Some(JobStatus::Ended) | None => None,
    }
}

/// An open job whose queued run may wait for a cap or for Claude's usage limit: what one pass of
/// `settle_jobs` gathers, so the maker is told after the pass with the facts read once
/// (`Makers::tell_held`).
pub(super) struct MaybeHeld {
    job: MakingJob,
    dot_name: String,
    run: Run,
}

/// How a job whose last run is `run`, which has ended, ended: for the store, in words for the
/// maker, and whether the run's error says more than the words do.
fn run_end(run: &Run) -> (JobEnd, &'static str, bool) {
    let error = run.error.as_deref().unwrap_or_default();
    match run.status {
        RunStatus::Cancelled => (JobEnd::Cancelled, "was stopped", false),
        RunStatus::Failed if run.quiet_stopped => {
            (JobEnd::Failed, "was stopped after it went quiet", false)
        }
        RunStatus::Failed if error.starts_with(STOPPED_BY_CAP) => {
            (JobEnd::Failed, "was stopped by a spend cap", false)
        }
        RunStatus::Failed if error.starts_with(STOPPED_BY_LIMIT) => {
            (JobEnd::Failed, "was stopped by Claude's usage limit", false)
        }
        RunStatus::Failed => (JobEnd::Failed, "failed", true),
        // A run that parked on an approval the person refused ends as succeeded too.
        _ => (JobEnd::Succeeded, "finished the job", false),
    }
}

/// What the maker is woken with when a job ends (design section 8): the made dot's name and
/// how the job ended, a failed run's `error` on one line, cut short, then the last run's final
/// text, cut at `MAX_JOB_RESULT` characters with a note, or that there is none. The error and
/// the final text are quoted line by line (`prompt::quoted`): CLI and MCP errors can carry tool
/// or model text.
pub fn ended_message(
    dot: &str,
    how: &str,
    error: Option<&str>,
    final_text: Option<&str>,
) -> String {
    let mut message = format!("\"{dot}\" {how}.\n\n");
    let error = error
        .map(|e| e.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|e| !e.is_empty());
    if let Some(error) = error {
        let mut cut: String = error.chars().take(MAX_ERROR).collect();
        if error.chars().count() > MAX_ERROR {
            cut.push('…');
        }
        message.push_str("The run failed. Its error text:\n");
        message.push_str(&crate::prompt::quoted(&cut));
        message.push('\n');
    }
    match final_text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            // Quoted, every line: the made dot's model wrote it, and it goes into the maker's
            // prompt, so no line of it may pass for a section or for Dots' own words.
            message.push_str("Its final message:\n");
            let cut: String = text.chars().take(MAX_JOB_RESULT).collect();
            message.push_str(crate::prompt::quoted(&cut).trim_end_matches('\n'));
            if text.chars().count() > MAX_JOB_RESULT {
                message.push('\n');
                message.push_str(RESULT_CUT);
            }
        }
        None => message.push_str(NO_FINAL_TEXT),
    }
    message
}

impl Makers {
    /// A maker's run gives a job to a dot it made (design section 8). `run_id` is the run that
    /// calls; its dot is the maker. `dot_name` names the made dot, in any letter case.
    /// Returns what the tool tells the run, at once: the job's run is queued, and the maker is
    /// woken when the job ends.
    ///
    /// When the made dot holds anything beyond the maker's reach as the maker is now
    /// (`held_now`, the check `change_dot` makes), the job does not start: a request of the
    /// kind `job` is stored for the person, with the job and what the dot holds, and the topic's
    /// log has an `asked` entry (`job_waits_text`). It counts toward the
    /// `MAX_WAITING_REQUESTS` of the maker, only one may wait per made dot, and it counts
    /// toward the loop guard only once the person allows it and it starts.
    ///
    /// An error, with a sentence the model can act on: what `make_dot` refuses a run for, a
    /// dot the maker did not make (the same sentence for any other dot), a job that is not 1 to
    /// 4,000 characters, a made dot that is switched off, a job while the last one from this
    /// conversation to the dot has not ended (before any request is stored), and what
    /// `start_job` refuses. The maker is judged as it is once the service's lock is held.
    pub async fn give_job(&self, run_id: &str, dot_name: &str, message: &str) -> Result<String> {
        self.give_job_as(run_id, dot_name, message, None).await
    }

    /// `give_job` for a call checked for the made dot `bound` (`tools::checked_input`):
    /// refused with `manage::not_the_same_dot`, and no job given or asked for, when the name
    /// no longer finds that dot. A request it stores is for that dot, by its id.
    pub(crate) async fn give_job_as(
        &self,
        run_id: &str,
        dot_name: &str,
        message: &str,
        bound: Option<&str>,
    ) -> Result<String> {
        let p = &self.parts;
        self.caller(run_id).await?;
        // With the makes: two jobs of one run cannot both take the last place of the guard.
        let _one_at_a_time = p.one_at_a_time.lock().await;
        // As they are once the lock is held: the run may have ended, the switch turned off, or
        // the person narrowed the maker while this call waited, as for `change_dot`.
        let (run, maker) = self.caller(run_id).await?;
        let made = self.made_dot_as(&maker, dot_name, bound).await?;
        let message = message.trim();
        if message.is_empty() || message.chars().count() > MAX_JOB {
            return Err(Error::Invalid(JOB_RULE.into()));
        }
        // A chat run of a dot that is off would start as any other: the person switched it
        // off, so a maker does not run it.
        if !made.spec.enabled {
            return Err(Error::Invalid(format!(
                "\"{}\" is switched off. The person turns it on.",
                made.spec.name
            )));
        }
        // Before a request is stored: a job that could not start when the person allows it
        // gets no card. `start_job` checks again under the thread lock.
        let earlier = p
            .store
            .latest_making_job(&run.root_run_id, &made.id)
            .await?;
        if let Some(refusal) = still_open(&made.spec.name, earlier.as_ref()) {
            return Err(refusal);
        }
        // The person's own edits do not take the service's lock, so one may land between this
        // check and the job's insert in `start_job`. That is no hole: the job then stands as one
        // given just before the edit, which the edit would not have stopped either, and every
        // later job is judged on the edit. Making every edit wait for the makes is not worth it.
        // Unlike `change_dot`, nothing is saved over the edit: the job's run reads the dot as it
        // is when it starts.
        let held = self.held_now(&maker, &made).await?;
        if !held.is_empty() {
            self.store_job_request(&maker, &run, &made, message, &held)
                .await?;
            return Ok(job_waits_text(&made.spec.name, &maker.spec.name, &held));
        }
        self.start_job(&maker, &run.id, &run.root_run_id, &made, message, true)
            .await?;
        Ok(gave_text(&made.spec.name))
    }

    /// Stores the job `message` for `made`, which holds `held` beyond the reach of `maker`, as
    /// a request for the person (design section 8), and writes its `asked` entry. Refused while
    /// a job request for the dot waits, and while the maker has `MAX_WAITING_REQUESTS`.
    async fn store_job_request(
        &self,
        maker: &Dot,
        run: &Run,
        made: &Dot,
        message: &str,
        held: &[Extra],
    ) -> Result<MakeRequest> {
        let p = &self.parts;
        let waiting = p.store.list_make_requests(Some(&maker.id), true).await?;
        if waiting.iter().any(|request| {
            request.kind == RequestKind::Job && request.dot_id.as_deref() == Some(made.id.as_str())
        }) {
            return Err(Error::Invalid(format!(
                "A job for \"{}\" is already waiting for the person. You will be woken with \
                 their answer.",
                made.spec.name
            )));
        }
        self.check_may_ask(maker).await?;
        let purpose = made.made.as_ref().map_or("", |m| m.purpose.as_str());
        let request = p
            .store
            .add_make_request(&NewMakeRequest {
                maker_dot_id: &maker.id,
                run_id: &run.id,
                root_run_id: &run.root_run_id,
                kind: RequestKind::Job,
                dot_id: Some(&made.id),
                purpose,
                spec: &made.spec,
                without: &made.spec,
                extras: held,
                over: None,
                create_folder: None,
                first_job: Some(message),
                ask: None,
                make_ask: None,
            })
            .await?;
        self.log_asked(&request, &maker.spec.name, Some(&made.id), &made.spec.name)
            .await;
        Ok(request)
    }

    /// Starts a job for the dot `made` of `maker`, given by the maker's run `maker_run_id` in
    /// its chat topic `maker_root_run_id`: for `give_job`, and for a first job when `make_dot`
    /// or an allowed request carried one. `message` is trimmed and within `MAX_JOB`. The caller
    /// holds the service's lock, so the guard's count and the new job are one step.
    ///
    /// The job's run is a chat run of the made dot that is not typed; its payload names the
    /// job (`JOB_ID`) and the maker (`prompt::JOB_FROM`), so its prompt says who sent it. It is
    /// background for the usage holds when the maker's run was, or when the maker's topic has
    /// no message the person typed (`limits::BACKGROUND`). Jobs
    /// from one topic of the maker go to one topic of the made dot: the run goes on where the
    /// last job from this topic ended, as the person's next message there would; when there is
    /// none, or that topic is gone or busy with something else, it starts a topic of its own.
    /// The run, the job and the `job_started` entry of the log are stored in one transaction,
    /// under the thread lock every writer that continues a chat holds.
    ///
    /// Refused (`Invalid`) for the job past the loop guard, when `guarded`, and while the last
    /// job from this topic to this dot has not ended: a topic is one line of runs, so the next
    /// job cannot be queued behind it. A job the person allowed is not `guarded`: the person
    /// gave it. Refused (`NotAllowed`) when this PC's role may not run the made dot.
    pub(crate) async fn start_job(
        &self,
        maker: &Dot,
        maker_run_id: &str,
        maker_root_run_id: &str,
        made: &Dot,
        message: &str,
        guarded: bool,
    ) -> Result<MakingJob> {
        let p = &self.parts;
        let name = &made.spec.name;
        if guarded && self.jobs_since_typed(maker_root_run_id).await? >= MAX_JOBS_BETWEEN_TYPED {
            return Err(Error::Invalid(guard_refusal(&maker.spec.name)));
        }
        p.runner.require_run(made).await?;
        // As the maker's run counted; a run that is gone gave the job in the person's sight.
        // An unattended topic (`unattended`) stays so however its later runs started: a wake
        // there, and the jobs it gives, are background too (design section 8).
        let background = match p.store.get_run(maker_run_id).await {
            Ok(run) => run_is_background(&run),
            Err(Error::NotFound(_)) => false,
            Err(e) => return Err(e),
        } || self.topic_unattended(maker_root_run_id).await?;
        let lock = p.follow_ups.thread_lock();
        let one_thread = lock.lock().await;
        let earlier = p
            .store
            .latest_making_job(maker_root_run_id, &made.id)
            .await?;
        if let Some(refusal) = still_open(name, earlier.as_ref()) {
            return Err(refusal);
        }
        let mut new = match self.topic_to_go_on(made, earlier.as_ref()).await? {
            Some((first, end)) => {
                crate::checkins::continuation(&first, &end, message.to_string(), None)
            }
            None => {
                let mut new = NewRun::new(&made.id, TriggerKind::Chat);
                new.payload = Some(json!({ "message": message }));
                new
            }
        };
        let id = new_id();
        if let Some(payload) = new.payload.as_mut() {
            payload[JOB_ID] = id.clone().into();
            payload[JOB_FROM] = maker.spec.name.clone().into();
            if background {
                payload[BACKGROUND] = true.into();
            }
        }
        let job = NewMakingJob {
            id: &id,
            maker_dot_id: &maker.id,
            maker_run_id,
            maker_root_run_id,
            dot_id: &made.id,
            message,
        };
        let (job, run) = p.store.start_making_job(&job, name, &new).await?;
        drop(one_thread);
        tracing::info!(job = %job.id, run = %run.id, maker = %maker.id, "a job was given");
        p.runner.enqueued(&run);
        self.updated(&maker.id, maker_root_run_id);
        Ok(job)
    }

    /// The first job given with the dot `made`, started when it is given: what came of it, or
    /// `None` without one. The dot stays made whatever comes of the job.
    pub(super) async fn start_first_job(
        &self,
        maker: &Dot,
        maker_run_id: &str,
        maker_root_run_id: &str,
        made: &Dot,
        message: Option<&str>,
    ) -> Option<FirstJob> {
        let message = message?.trim();
        let started = self
            .start_job(maker, maker_run_id, maker_root_run_id, made, message, true)
            .await;
        Some(match started {
            Ok(job) => FirstJob::Started(job),
            Err(Error::Invalid(why) | Error::Conflict(why) | Error::NotAllowed(why)) => {
                FirstJob::NotStarted(why)
            }
            // The store's or the disk's own words are for the log, not the model.
            Err(e) => {
                tracing::warn!(dot = %made.id, run = %maker_run_id, "the first job was not started: {e}");
                FirstJob::NotStarted(format!(
                    "{} Give it the job with give_job.",
                    super::tools::COULD_NOT
                ))
            }
        })
    }

    /// How many jobs were started in the maker's chat topic `root_run_id` since the person
    /// last typed a message in it (design section 8): since the newest run of the topic that
    /// carries a typed message was queued, or since the topic's start when it has none, as a
    /// topic a schedule or another background trigger started. Answers to questions and to
    /// cards, and wakes, are not typed.
    async fn jobs_since_typed(&self, root_run_id: &str) -> Result<u32> {
        let store = &self.parts.store;
        let thread = store.list_thread(root_run_id).await?;
        let since = thread
            .iter()
            .filter(|run| is_typed(run))
            .map(|run| run.queued_at.as_str())
            .max();
        store.count_jobs_started(root_run_id, since).await
    }

    /// Whether the chat topic the run `run_id` is in now is unattended (`unattended`). A run
    /// that is gone is in no topic: `false`.
    async fn topic_unattended(&self, run_id: &str) -> Result<bool> {
        let store = &self.parts.store;
        let run = match store.get_run(run_id).await {
            Ok(run) => run,
            Err(Error::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        let thread = store.list_thread(&run.root_run_id).await?;
        Ok(unattended(&thread))
    }

    /// The topic of `made` the next job from a topic of the maker goes on in, as its first run
    /// and its end: the topic the job `earlier`, the last from there, ran in, while that topic
    /// exists, is the made dot's, has no run going and is not adding an app. The caller holds
    /// the thread lock.
    async fn topic_to_go_on(
        &self,
        made: &Dot,
        earlier: Option<&MakingJob>,
    ) -> Result<Option<(Run, Run)>> {
        let p = &self.parts;
        let Some(topic) = earlier.and_then(|job| job.dot_root_run_id.as_deref()) else {
            return Ok(None);
        };
        // The topic that run is in now: the person may have moved it since.
        let first = match p.store.get_run(topic).await {
            Ok(run) if run.dot_id == made.id => run,
            Ok(_) | Err(Error::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        if p.follow_ups.deciding(&first.root_run_id) {
            return Ok(None);
        }
        let mut thread = p.store.list_thread(&first.root_run_id).await?;
        if thread.iter().any(|run| !run.status.is_terminal()) {
            return Ok(None);
        }
        let end = thread.pop().unwrap_or_else(|| first.clone());
        Ok(Some((first, end)))
    }

    /// Follows every job that has not ended (design section 8). Called when a run ends, when
    /// the person answers or dismisses a question, at startup, and by tests. For each open job,
    /// by where its last run stands:
    ///
    /// - It has not ended (queued, running, awaiting an approval): nothing, except that a
    ///   queued run that waits for a spending cap or Claude's usage limit is told to the maker
    ///   once (`held_message`), after the pass (`tell_held`), when the maker's wake could start.
    /// - It ended and a run goes on with its work in the same turn (`Store::run_continued_by`):
    ///   the job moves to that run.
    /// - It ended and the newest question a run of the job asked waits for the person: the job
    ///   waits (`waiting_answer`, a `job_waiting` entry), and the maker is not woken.
    /// - That question was answered by a run the job has not been at: the job moves to it.
    /// - That question was dismissed: the job ends as stopped.
    /// - Otherwise the job ends as its last run did; a run the person's refusal closed, with
    ///   no note, ends it as stopped (`REFUSED`).
    ///
    /// Ending a job stores its wake in the same transaction (`Store::end_making_job`), so of
    /// any number of passes one ends it and the maker is woken once. One pass runs at a time.
    /// The error of the first job that could not be followed is returned after the others were.
    pub async fn settle_jobs(&self) -> Result<()> {
        let p = &self.parts;
        let mut ended = false;
        let mut failed = None;
        {
            let _one_pass = p.settling.lock().await;
            let mut held = Vec::new();
            for job in p.store.open_making_jobs(None).await? {
                let id = job.id.clone();
                match self.settle_job(job, &mut held).await {
                    Ok(woke) => {
                        p.logged.forget(&id);
                        ended |= woke;
                    }
                    Err(e) => {
                        // Every pass tries it again: logged once, then at most hourly.
                        if p.logged.due(&id, std::time::Instant::now()) {
                            tracing::warn!(job = %id, "the job could not be followed: {e}");
                        }
                        failed.get_or_insert(e);
                    }
                }
            }
            match self.tell_held(held).await {
                Ok(told) => {
                    // A new failure is logged at once.
                    p.logged.forget(HELD_CHECK);
                    ended |= told;
                }
                Err(e) => {
                    if p.logged.due(HELD_CHECK, std::time::Instant::now()) {
                        tracing::warn!("the held jobs could not be checked: {e}");
                    }
                    failed.get_or_insert(e);
                }
            }
        }
        // After the pass's lock is released: delivery takes the thread lock.
        if ended {
            if let Err(e) = self.deliver_wakes().await {
                tracing::warn!("a job's wake waits; delivering it failed: {e}");
            }
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// One open job of a pass (`settle_jobs`, and a delete of its made dot, which holds
    /// `settling`). `true` when it stored a wake here: it ended. A working job whose run is
    /// queued and may wait for a cap is added to `held`, for the pass to tell (`tell_held`).
    pub(super) async fn settle_job(
        &self,
        mut job: MakingJob,
        held: &mut Vec<MaybeHeld>,
    ) -> Result<bool> {
        let store = &self.parts.store;
        let dot = match store.get_dot(&job.dot_id).await {
            Ok(dot) => dot,
            // The made dot went since the list was read, and the job with it.
            Err(Error::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        let name = dot.spec.name.as_str();
        for _ in 0..MAX_HOPS {
            let last = match job.last_run_id.as_deref() {
                Some(id) => match store.get_run(id).await {
                    Ok(run) => Some(run),
                    Err(Error::NotFound(_)) => None,
                    Err(e) => return Err(e),
                },
                None => None,
            };
            let Some(last) = last else {
                // Runs go only with their dot, and the job with them; kept so that a job
                // without a run still ends.
                let gone = ended_message(name, "failed", Some("its run no longer exists"), None);
                return self.end_job(&job, name, JobEnd::Failed, &gone).await;
            };
            if !last.status.is_terminal() {
                // A run that waits for a cap may wait for hours: the maker is told once.
                if last.status == RunStatus::Queued && job.status == JobStatus::Working {
                    held.push(MaybeHeld {
                        job,
                        dot_name: name.to_string(),
                        run: last,
                    });
                }
                return Ok(false);
            }
            if let Some(next) = store.run_continued_by(&last.id).await? {
                if !store.move_making_job(&job.id, &last.id, &next.id).await? {
                    return Ok(false);
                }
                job.last_run_id = Some(next.id);
                job.status = JobStatus::Working;
                continue;
            }
            // The questions of the job's runs: those the topic's runs asked since the job's
            // first run was queued.
            let since = match job.first_run_id.as_deref() {
                Some(id) => store.get_run(id).await.ok().map(|run| run.queued_at),
                None => None,
            }
            .unwrap_or_else(|| job.created_at.clone());
            let asked = store
                .latest_question_since(&last.root_run_id, &since)
                .await?;
            let final_text = last.summary.as_deref();
            match asked {
                Some(question) if question.waits() => {
                    if job.status == JobStatus::Working
                        && store.set_making_job_waiting(&job.id).await?
                    {
                        let detail = json!({ "job_id": job.id, "run_id": last.id });
                        self.log_job(
                            &job,
                            MakingEventKind::JobWaiting,
                            Some(&dot.id),
                            name,
                            detail,
                        )
                        .await;
                    }
                    return Ok(false);
                }
                Some(question) if question.dismissed_at.is_some() => {
                    // Nothing can answer it any more, so nothing would end the job.
                    let said = match self.values_to_hide(&job, &dot).await {
                        Some(hidden) => {
                            let text = final_text.map(|text| hide(text, &hidden));
                            ended_message(name, "was stopped", None, text.as_deref())
                        }
                        None => withheld_message(name, "was stopped"),
                    };
                    return self.end_job(&job, name, JobEnd::Cancelled, &said).await;
                }
                Some(question) => {
                    let answer = match question.answer_run_id.as_deref() {
                        Some(id) if !self.was_at(&last, id).await? => store.get_run(id).await.ok(),
                        _ => None,
                    };
                    if let Some(answer) = answer {
                        if !store.move_making_job(&job.id, &last.id, &answer.id).await? {
                            return Ok(false);
                        }
                        job.last_run_id = Some(answer.id);
                        job.status = JobStatus::Working;
                        continue;
                    }
                }
                None => {}
            }
            let (how, words, with_error) = if self.refused_without_a_note(&last).await? {
                (JobEnd::Cancelled, REFUSED, false)
            } else {
                run_end(&last)
            };
            let said = match self.values_to_hide(&job, &dot).await {
                Some(hidden) => {
                    let error = last.error.as_deref().filter(|_| with_error);
                    let error = error.map(|error| hide(error, &hidden));
                    let text = final_text.map(|text| hide(text, &hidden));
                    ended_message(name, words, error.as_deref(), text.as_deref())
                }
                None => withheld_message(name, words),
            };
            return self.end_job(&job, name, how, &said).await;
        }
        Err(Error::Other(format!(
            "job {} went through {MAX_HOPS} runs in one pass",
            job.id
        )))
    }

    /// `held_values` for the job `job` of `made`, or `None` when they cannot be read (a vault or
    /// store error): logged, and the wake then leaves the result out (`WITHHELD`), so the job
    /// still ends and its maker is woken rather than the job staying open.
    async fn values_to_hide(&self, job: &MakingJob, made: &Dot) -> Option<Vec<String>> {
        match self.held_values(&job.maker_dot_id, made).await {
            Ok(values) => Some(values),
            Err(e) => {
                tracing::warn!(
                    job = %job.id,
                    "the dot's own values could not be read, so the wake leaves its result out: {e}"
                );
                None
            }
        }
    }

    /// The values of the variables `made` holds beyond the reach of its maker `maker_id` as the
    /// maker is now (`held_now`), as the vault gives them now: the values its runs were given,
    /// which the maker's wake must not carry (design section 8; `hide`). In each spelling the
    /// vault's redactor knows, longest first. A value shorter than the redactor scrubs
    /// (`MIN_REDACT_BYTES`) is left, as everywhere else; a variable that does not resolve now
    /// has no value to hide.
    async fn held_values(&self, maker_id: &str, made: &Dot) -> Result<Vec<String>> {
        use crate::vault::redact::{spellings, MIN_REDACT_BYTES};
        let p = &self.parts;
        let maker = match p.store.get_dot(maker_id).await {
            Ok(maker) => maker,
            // No maker, no wake to keep the values from.
            Err(Error::NotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let scope = crate::vault::DotScope::of(made);
        let mut values = Vec::new();
        for extra in self.held_now(&maker, made).await? {
            let Extra::Variable { consumer, name } = extra else {
                continue;
            };
            let Ok(consumer) = crate::vault::Consumer::parse(&consumer) else {
                continue;
            };
            let names = [name];
            let resolved = p
                .vault
                .resolve(&scope, &made.spec.variables, &consumer, &names)
                .await?;
            if let Ok(found) = resolved {
                for (_, value) in found.pairs() {
                    if value.len() >= MIN_REDACT_BYTES {
                        values.extend(spellings(value));
                    }
                }
            }
        }
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        Ok(values)
    }

    /// Whether the ended run `last` was closed by the person's refusal of a call it parked on,
    /// with no note: then no run goes on after it, and the store closed it as succeeded.
    async fn refused_without_a_note(&self, last: &Run) -> Result<bool> {
        if last.status != RunStatus::Succeeded {
            return Ok(false);
        }
        let approvals = self.parts.store.approvals_for_run(&last.id).await?;
        Ok(approvals
            .iter()
            .any(|a| a.parked && a.status == ApprovalStatus::Denied))
    }

    /// Tells each maker once, with a wake, that its open job of the jobs `held` has not
    /// started: the job's queued run waits for a spending cap or for Claude's usage limit
    /// (design section 8). The job stays open. Why each run waits, and the caps, are read once
    /// for all of them. `true` when a wake was stored here.
    ///
    /// The maker is told only when that wake could start: not while a cap holds the maker too,
    /// as the cap it shares with the dots it made does; when the wake would be background
    /// (`unattended`), not while Claude's usage hold or the role hold would hold it
    /// (`background_wake_held`); and in a topic the person watches, not while this PC may not
    /// run the maker, as the wake then waits (`wake::deliver_wake`). Such a wake would wait as
    /// long as the job or the role, keep the maker's topic busy meanwhile, and arrive when it is
    /// no longer true. The job is then left untold, and may be told on a later pass.
    async fn tell_held(&self, held: Vec<MaybeHeld>) -> Result<bool> {
        let p = &self.parts;
        let mut untold = Vec::with_capacity(held.len());
        for one in held {
            if !p.store.making_job_held_told(&one.job.id).await? {
                untold.push(one);
            }
        }
        if untold.is_empty() {
            return Ok(false);
        }
        let mut runs: Vec<Run> = untold.iter().map(|one| one.run.clone()).collect();
        p.runner.fill_wait_reasons(&mut runs).await?;
        let mut ledger = Ledger::load(&p.store).await?;
        let mut told = false;
        for (one, run) in untold.iter().zip(runs) {
            let MaybeHeld { job, dot_name, .. } = one;
            let usage_held = run.wait_reason == Some(WaitReason::UsageHold);
            let why = match run.wait_reason {
                Some(WaitReason::SpendCap) => run
                    .wait_detail
                    .unwrap_or_else(|| WaitReason::SpendCap.text().to_string()),
                Some(WaitReason::UsageHold) => "it is held for Claude's usage limit".to_string(),
                _ => continue,
            };
            let unattended = self.topic_unattended(&job.maker_root_run_id).await?;
            if unattended && usage_held {
                continue;
            }
            if ledger.hit_for(&p.store, &job.maker_dot_id).await?.is_some() {
                continue;
            }
            if unattended && self.background_wake_held(&job.maker_dot_id).await? {
                continue;
            }
            if !unattended && self.role_holds(&job.maker_dot_id).await? {
                continue;
            }
            let said = held_message(dot_name, &why);
            if p.store.tell_making_job_held(&job.id, &said).await? {
                tracing::info!(job = %job.id, run = %run.id, "a job's maker was told its run waits");
                told = true;
            }
        }
        Ok(told)
    }

    /// Whether a background wake of the maker `maker_id` would be held now: by Claude's usage
    /// hold for a Claude maker, or by the role hold (this PC may not run the maker, or its
    /// policy is past the workspace's guardrails ceiling; members spec section 6.3). A maker
    /// that is gone holds nothing.
    async fn background_wake_held(&self, maker_id: &str) -> Result<bool> {
        let p = &self.parts;
        let maker = match p.store.get_dot(maker_id).await {
            Ok(maker) => maker,
            Err(Error::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        if maker.spec.engine == crate::model::EngineKind::Claude
            && p.runner.usage_holds_background()
        {
            return Ok(true);
        }
        match p.runner.require_run(&maker).await {
            Ok(()) => {}
            Err(Error::NotAllowed(_)) => return Ok(true),
            Err(e) => return Err(e),
        }
        Ok(p.runner.ceiling_hold(&maker).await.is_some())
    }

    /// Whether this PC may not run the maker `maker_id` now (`Runner::require_run`): a wake in
    /// a topic the person watches then waits (`wake::deliver_wake`). A maker that is gone holds
    /// nothing.
    async fn role_holds(&self, maker_id: &str) -> Result<bool> {
        let p = &self.parts;
        let maker = match p.store.get_dot(maker_id).await {
            Ok(maker) => maker,
            Err(Error::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        match p.runner.require_run(&maker).await {
            Ok(()) => Ok(false),
            Err(Error::NotAllowed(_)) => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// Whether the job that is at the run `last` has been at the run `run_id`: `last` itself,
    /// or a run before it in its line. A run that sends a message again comes after the run
    /// whose message it sends (`retry_of`); any other after its parent.
    async fn was_at(&self, last: &Run, run_id: &str) -> Result<bool> {
        let store = &self.parts.store;
        let mut at = last.clone();
        for _ in 0..MAX_HOPS {
            if at.id == run_id {
                return Ok(true);
            }
            let Some(before) = at.retry_of.clone().or(at.parent_run_id.clone()) else {
                return Ok(false);
            };
            at = match store.get_run(&before).await {
                Ok(run) => run,
                Err(Error::NotFound(_)) => return Ok(false),
                Err(e) => return Err(e),
            };
        }
        Ok(false)
    }

    /// Ends the open `job` of the made dot named `dot_name` as `how` and stores the wake that
    /// tells the maker `said`, then writes the `job_ended` entry of the log. `false` when
    /// another pass had ended it.
    async fn end_job(
        &self,
        job: &MakingJob,
        dot_name: &str,
        how: JobEnd,
        said: &str,
    ) -> Result<bool> {
        let store = &self.parts.store;
        let Some(ended) = store.end_making_job(&job.id, how, said).await? else {
            return Ok(false);
        };
        tracing::info!(job = %job.id, how = how.as_str(), "a job ended");
        let detail = json!({
            "job_id": ended.id,
            "run_id": ended.last_run_id,
            "how": how.as_str(),
        });
        self.log_job(
            &ended,
            MakingEventKind::JobEnded,
            Some(&ended.dot_id),
            dot_name,
            detail,
        )
        .await;
        Ok(true)
    }

    /// Writes an entry about `job` to the log of the maker's topic it was given in, as the
    /// run's that gave it, and publishes `MakingUpdated`. The job stands as it is whether or
    /// not the entry could be written, so a failure is only logged.
    pub(super) async fn log_job(
        &self,
        job: &MakingJob,
        kind: MakingEventKind,
        dot_id: Option<&str>,
        dot_name: &str,
        detail: serde_json::Value,
    ) {
        let logged = self
            .parts
            .store
            .add_making_event(&NewMakingEvent {
                maker_dot_id: &job.maker_dot_id,
                run_id: &job.maker_run_id,
                root_run_id: &job.maker_root_run_id,
                kind,
                dot_id,
                dot_name,
                detail,
            })
            .await;
        if let Err(e) = logged {
            tracing::error!(job = %job.id, kind = kind.as_str(), "the job's entry was not logged: {e}");
        }
        self.updated(&job.maker_dot_id, &job.maker_root_run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended_run(status: RunStatus, error: Option<&str>, quiet_stopped: bool) -> Run {
        serde_json::from_value(json!({
            "id": "R", "dot_id": "D", "root_run_id": "R", "parent_run_id": null,
            "trigger": "chat", "payload": null, "status": status.as_str(),
            "session_id": null, "workspace_path": null, "branch": null, "base_commit": null,
            "summary": null, "error": error, "tokens_in": 0, "tokens_out": 0,
            "queued_at": "2026-11-11T09:00:00.000Z", "started_at": null, "ended_at": null,
            "reviewed_at": null, "retry_of": null, "cost_usd": null, "hold_released": false,
            "workflow_step_run_id": null, "tokens_cache_read": 0, "quiet_since": null,
            "quiet_limit_secs": null, "quiet_stopped": quiet_stopped,
        }))
        .unwrap()
    }

    #[test]
    fn each_way_a_run_ends_has_its_words() {
        let cases = [
            (
                RunStatus::Succeeded,
                None,
                false,
                JobEnd::Succeeded,
                "finished the job",
                false,
            ),
            (
                RunStatus::Cancelled,
                None,
                false,
                JobEnd::Cancelled,
                "was stopped",
                false,
            ),
            (
                RunStatus::Failed,
                Some("boom"),
                false,
                JobEnd::Failed,
                "failed",
                true,
            ),
            (
                RunStatus::Failed,
                Some("timeout after 600s"),
                false,
                JobEnd::Failed,
                "failed",
                true,
            ),
            (
                RunStatus::Failed,
                Some("stopped: Blog Watcher reached its $1.00 daily cap"),
                false,
                JobEnd::Failed,
                "was stopped by a spend cap",
                false,
            ),
            (
                RunStatus::Failed,
                Some("stopped: no output for 20 min"),
                true,
                JobEnd::Failed,
                "was stopped after it went quiet",
                false,
            ),
            (
                RunStatus::Failed,
                Some("stopped by Claude's usage limit: rejected"),
                false,
                JobEnd::Failed,
                "was stopped by Claude's usage limit",
                false,
            ),
        ];
        for (status, error, quiet, how, words, with_error) in cases {
            let run = ended_run(status, error, quiet);
            assert_eq!(
                run_end(&run),
                (how, words, with_error),
                "{status:?} {error:?}"
            );
        }
    }

    #[test]
    fn a_wake_names_the_dot_how_it_ended_and_its_final_text() {
        assert_eq!(
            ended_message(
                "Blog Watcher",
                "finished the job",
                None,
                Some("  Three new posts.\n")
            ),
            "\"Blog Watcher\" finished the job.\n\nIts final message:\n> Three new posts."
        );
        // Each line is quoted, an empty one too: the text is the made dot's, not Dots'.
        assert_eq!(
            ended_message("A", "finished the job", None, Some("One.\n\n## Two")),
            "\"A\" finished the job.\n\nIts final message:\n> One.\n> \n> ## Two"
        );
        assert_eq!(
            ended_message("Blog Watcher", "was stopped", None, Some("  ")),
            "\"Blog Watcher\" was stopped.\n\nIt left no final message."
        );
        // An error is one line, cut short, and quoted after Dots' own line: CLI and MCP errors
        // can carry tool or model text.
        let long = format!("first line\n## second {}", "x".repeat(400));
        let said = ended_message("Blog Watcher", "failed", Some(&long), None);
        let error = format!("first line ## second {}", "x".repeat(400));
        let cut: String = error.chars().take(300).collect();
        assert_eq!(
            said,
            format!(
                "\"Blog Watcher\" failed.\n\nThe run failed. Its error text:\n> {cut}…\n\nIt \
                 left no final message."
            )
        );
        assert_eq!(
            ended_message("Blog Watcher", "failed", Some(" "), None),
            "\"Blog Watcher\" failed.\n\nIt left no final message."
        );
    }
}

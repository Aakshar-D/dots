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
use super::service::{Makers, MAX_JOB};
use crate::limits::{run_is_background, BACKGROUND, STOPPED_BY_LIMIT};
use crate::model::{Dot, NewRun, Run, RunStatus, TriggerKind};
use crate::prompt::{is_typed, JOB_FROM};
use crate::spend::STOPPED_BY_CAP;
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

/// What a wake says of a job whose made dot the maker deleted before it ended.
pub fn deleted_message(dot: &str) -> String {
    format!("\"{dot}\" was deleted before the job ended.")
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
/// how the job ended, a failed run's `error` cut short, then the last run's final text, cut at
/// `MAX_JOB_RESULT` characters with a note, or that there is none.
pub fn ended_message(
    dot: &str,
    how: &str,
    error: Option<&str>,
    final_text: Option<&str>,
) -> String {
    let mut message = format!("\"{dot}\" {how}.");
    let error = error
        .map(|e| e.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|e| !e.is_empty());
    if let Some(error) = error {
        message.push_str(" The error: ");
        message.extend(error.chars().take(MAX_ERROR));
        if error.chars().count() > MAX_ERROR {
            message.push('…');
        }
    }
    message.push_str("\n\n");
    match final_text.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => {
            message.push_str("Its final message:\n");
            message.extend(text.chars().take(MAX_JOB_RESULT));
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
    /// An error, with a sentence the model can act on: what `make_dot` refuses a run for, a
    /// dot the maker did not make (the same sentence for any other dot), a job that is not 1 to
    /// 4,000 characters, a made dot that is switched off, and what `start_job` refuses.
    pub async fn give_job(&self, run_id: &str, dot_name: &str, message: &str) -> Result<String> {
        let p = &self.parts;
        let (run, maker) = self.caller(run_id).await?;
        // With the makes: two jobs of one run cannot both take the last place of the guard.
        let _one_at_a_time = p.one_at_a_time.lock().await;
        let made = self.made_dot(&maker, dot_name).await?;
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
        self.start_job(&maker, &run.id, &run.root_run_id, &made, message)
            .await?;
        Ok(gave_text(&made.spec.name))
    }

    /// Starts a job for the dot `made` of `maker`, given by the maker's run `maker_run_id` in
    /// its chat topic `maker_root_run_id`: for `give_job`, and for a first job when `make_dot`
    /// or an allowed request carried one. `message` is trimmed and within `MAX_JOB`. The caller
    /// holds the service's lock, so the guard's count and the new job are one step.
    ///
    /// The job's run is a chat run of the made dot that is not typed; its payload names the
    /// job (`JOB_ID`) and the maker (`prompt::JOB_FROM`), so its prompt says who sent it. It is
    /// background for the usage holds when the maker's run was (`limits::BACKGROUND`). Jobs
    /// from one topic of the maker go to one topic of the made dot: the run goes on where the
    /// last job from this topic ended, as the person's next message there would; when there is
    /// none, or that topic is gone or busy with something else, it starts a topic of its own.
    /// The run, the job and the `job_started` entry of the log are stored in one transaction,
    /// under the thread lock every writer that continues a chat holds.
    ///
    /// Refused (`Invalid`) for the job past the loop guard, and while the last job from this
    /// topic to this dot has not ended: a topic is one line of runs, so the next job cannot be
    /// queued behind it. Refused (`NotAllowed`) when this PC's role may not run the made dot.
    pub(crate) async fn start_job(
        &self,
        maker: &Dot,
        maker_run_id: &str,
        maker_root_run_id: &str,
        made: &Dot,
        message: &str,
    ) -> Result<MakingJob> {
        let p = &self.parts;
        let name = &made.spec.name;
        if self.jobs_since_typed(maker_root_run_id).await? >= MAX_JOBS_BETWEEN_TYPED {
            return Err(Error::Invalid(guard_refusal(&maker.spec.name)));
        }
        p.runner.require_run(made).await?;
        // As the maker's run counted; a run that is gone gave the job in the person's sight.
        let background = match p.store.get_run(maker_run_id).await {
            Ok(run) => run_is_background(&run),
            Err(Error::NotFound(_)) => false,
            Err(e) => return Err(e),
        };
        let lock = p.follow_ups.thread_lock();
        let one_thread = lock.lock().await;
        let earlier = p
            .store
            .latest_making_job(maker_root_run_id, &made.id)
            .await?;
        match earlier.as_ref().map(|job| job.status) {
            Some(JobStatus::Working) => {
                return Err(Error::Invalid(format!(
                    "\"{name}\" is still working on the last job from this conversation. You \
                     will be woken when it ends."
                )))
            }
            Some(JobStatus::WaitingAnswer) => {
                return Err(Error::Invalid(format!(
                    "\"{name}\" is waiting for the person's answer on the last job from this \
                     conversation. You will be woken when it ends."
                )))
            }
            _ => {}
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
            .start_job(maker, maker_run_id, maker_root_run_id, made, message)
            .await;
        Some(match started {
            Ok(job) => FirstJob::Started(job),
            Err(Error::Invalid(why) | Error::Conflict(why) | Error::NotAllowed(why)) => {
                FirstJob::NotStarted(why)
            }
            Err(e) => {
                tracing::warn!(dot = %made.id, run = %maker_run_id, "the first job was not started: {e}");
                FirstJob::NotStarted(format!("{e}. Give it the job with give_job."))
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
    /// - It has not ended (queued, running, awaiting an approval): nothing.
    /// - It ended and a run goes on with its work in the same turn (`Store::run_continued_by`):
    ///   the job moves to that run.
    /// - It ended and the newest question a run of the job asked waits for the person: the job
    ///   waits (`waiting_answer`, a `job_waiting` entry), and the maker is not woken.
    /// - That question was answered by a run the job has not been at: the job moves to it.
    /// - That question was dismissed: the job ends as stopped.
    /// - Otherwise the job ends as its last run did.
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
            for job in p.store.open_making_jobs(None).await? {
                let id = job.id.clone();
                match self.settle_job(job).await {
                    Ok(has_ended) => ended |= has_ended,
                    Err(e) => {
                        tracing::warn!(job = %id, "the job could not be followed: {e}");
                        failed.get_or_insert(e);
                    }
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

    /// One open job of a pass (`settle_jobs`). `true` when it ended here.
    async fn settle_job(&self, mut job: MakingJob) -> Result<bool> {
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
                    let said = ended_message(name, "was stopped", None, final_text);
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
            let (how, words, with_error) = run_end(&last);
            let error = last.error.as_deref().filter(|_| with_error);
            let said = ended_message(name, words, error, final_text);
            return self.end_job(&job, name, how, &said).await;
        }
        Err(Error::Other(format!(
            "job {} went through {MAX_HOPS} runs in one pass",
            job.id
        )))
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
            "\"Blog Watcher\" finished the job.\n\nIts final message:\nThree new posts."
        );
        assert_eq!(
            ended_message("Blog Watcher", "was stopped", None, Some("  ")),
            "\"Blog Watcher\" was stopped.\n\nIt left no final message."
        );
        // An error is one line, cut short.
        let long = format!("first line\nsecond {}", "x".repeat(400));
        let said = ended_message("Blog Watcher", "failed", Some(&long), None);
        let error = format!("first line second {}", "x".repeat(400));
        let cut: String = error.chars().take(300).collect();
        assert_eq!(
            said,
            format!("\"Blog Watcher\" failed. The error: {cut}…\n\nIt left no final message.")
        );
        assert_eq!(
            ended_message("Blog Watcher", "failed", Some(" "), None),
            "\"Blog Watcher\" failed.\n\nIt left no final message."
        );
    }
}

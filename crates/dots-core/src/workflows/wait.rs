//! Wait steps (general workflows spec section 7): the workflow waits for a time, with no model
//! and no one to ask. When the step begins, the time it waits until is worked out once and
//! kept on its step run (`decision: {"until": <RFC 3339>}`), so a restart changes nothing and
//! a time that passed while Dots was closed ends the wait on the first pass after. A waiting
//! run's clock stands still: the wait counts no minutes against the budget.

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeDelta, TimeZone, Utc};
use serde_json::json;

use super::Workflows;
use crate::model::{
    StepRun, StepRunChange, StepRunStatus, WorkflowRun, WorkflowRunChange, WorkflowRunStatus,
};
use crate::util::timestamp;
use crate::workflow::{date_time_of, time_of_day, Step, StepKind, Wait};
use crate::{Error, Result};

/// Whether the workflow run is at a wait step now: running, with a wait as its current step.
pub(super) fn waits(wr: &WorkflowRun) -> bool {
    wr.status == WorkflowRunStatus::Running
        && wr
            .current_step
            .as_deref()
            .and_then(|id| wr.definition.step(id))
            .is_some_and(|step| matches!(step.kind, StepKind::Wait { .. }))
}

/// The moment `local`, a date and time of this PC, falls on. A time the clocks skip (the hour
/// lost in spring) is the moment the skip ends; one they repeat is its first.
fn moment<Tz: TimeZone>(zone: &Tz, local: NaiveDateTime) -> DateTime<Utc> {
    let mut at = local;
    for _ in 0..4 {
        if let Some(found) = zone.from_local_datetime(&at).earliest() {
            return found.with_timezone(&Utc);
        }
        at += TimeDelta::minutes(30);
    }
    Utc.from_utc_datetime(&local)
}

/// The time the wait `wait` of `wr` ends, for a step that begins at `now`, in the zone `zone`
/// (this PC's); `Err` says why it has none.
pub(super) fn until<Tz: TimeZone>(
    wait: &Wait,
    wr: &WorkflowRun,
    now: DateTime<Utc>,
    zone: &Tz,
) -> std::result::Result<DateTime<Utc>, String> {
    match wait {
        Wait::Minutes { minutes } => Ok(now + TimeDelta::minutes(i64::from(*minutes))),
        Wait::Time { at, weekday } => {
            let Some(time) = time_of_day(at) else {
                return Err(format!("{at:?} is not a time of day"));
            };
            let today = now.with_timezone(zone).date_naive();
            // The next such time after now: today's, or a later day's, at most eight days on.
            for days in 0..=8 {
                let date = today + TimeDelta::days(days);
                if weekday.is_some_and(|w| w != date.weekday()) {
                    continue;
                }
                let found = moment(zone, date.and_time(time));
                if found > now {
                    return Ok(found);
                }
            }
            Err(format!("no {at} comes in the next days"))
        }
        Wait::Input { input } => {
            let value = wr.inputs.get(input).map(|v| v.trim()).unwrap_or_default();
            if value.is_empty() {
                return Err(format!("the input \"{input}\" holds no date and time"));
            }
            match date_time_of(value) {
                Some(local) => Ok(moment(zone, local)),
                None => Err(format!(
                    "the input \"{input}\" is not a date and time: {value:?}"
                )),
            }
        }
    }
}

/// The time the wait step run `step_run` waits until, as its begin kept it.
fn kept(step_run: &StepRun) -> Option<DateTime<Utc>> {
    let until = step_run.decision.as_ref()?.get("until")?.as_str()?;
    DateTime::parse_from_rfc3339(until)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

impl Workflows {
    /// Begins the wait step `step` as its next attempt: a step run that stays `Running` until
    /// its time, kept on it. A wait with no time to wait for (an input left empty) fails, and
    /// the run waits for the person.
    pub(super) async fn begin_wait(&self, wr: &WorkflowRun, step: &Step) -> Result<()> {
        let StepKind::Wait { wait } = &step.kind else {
            return Err(Error::Other(format!("step {} is not a wait", step.id)));
        };
        let step_run = self
            .step_run_to_begin(wr, step, StepRunStatus::Running)
            .await?;
        let wr = self
            .store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    current_step: Some(Some(step.id.clone())),
                    ..Default::default()
                },
            )
            .await?;
        match until(wait, &wr, self.clock.now(), &Local) {
            Ok(until) => {
                self.store
                    .set_step_run(
                        &step_run.id,
                        &StepRunChange {
                            decision: Some(Some(json!({"until": timestamp(until)}))),
                            ..Default::default()
                        },
                    )
                    .await?;
                self.due_at(until);
                self.updated(&self.store.get_workflow_run(&wr.id).await?);
                Ok(())
            }
            Err(error) => self.fail_step(&wr, step, &step_run, &error).await,
        }
    }

    /// Watches a wait step run: it succeeds once its time has come, and the workflow moves on.
    /// One whose time was never kept (a begin cut off between its writes) begins again.
    pub(super) async fn watch_wait(
        &self,
        wr: &WorkflowRun,
        step: &Step,
        step_run: &StepRun,
    ) -> Result<()> {
        let Some(until) = kept(step_run) else {
            return self.begin_wait(wr, step).await;
        };
        if self.clock.now() < until {
            self.due_at(until);
            return Ok(());
        }
        self.store
            .set_step_run(
                &step_run.id,
                &StepRunChange {
                    status: Some(StepRunStatus::Succeeded),
                    ..Default::default()
                },
            )
            .await?;
        let wr = self.store.get_workflow_run(&wr.id).await?;
        self.move_on(&wr, step).await
    }

    /// Notes that a wait ends at `until`, so the loop passes then and not a tick later.
    fn due_at(&self, until: DateTime<Utc>) {
        let mut due = self.due.lock().unwrap_or_else(|e| e.into_inner());
        if due.is_none_or(|d| until < d) {
            *due = Some(until);
        }
    }

    /// How long the loop may sleep before a wait ends: `None` when none is waited for. Takes
    /// the note, which the next pass writes again for the waits still on.
    pub(super) fn until_due(&self) -> Option<std::time::Duration> {
        let due = self.due.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        let left = (due - self.clock.now()).num_milliseconds().max(0) as u64;
        Some(std::time::Duration::from_millis(left))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveDate, Weekday};

    use super::*;

    /// A zone five and a half hours ahead of UTC, with no clock changes.
    fn zone() -> FixedOffset {
        FixedOffset::east_opt(5 * 3600 + 1800).unwrap()
    }

    /// The moment `date` `h`:`m` in `zone()`.
    fn at(date: (i32, u32, u32), h: u32, m: u32) -> DateTime<Utc> {
        let local = NaiveDate::from_ymd_opt(date.0, date.1, date.2)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap();
        moment(&zone(), local)
    }

    fn time(at: &str, weekday: Option<Weekday>) -> Wait {
        Wait::Time {
            at: at.into(),
            weekday,
        }
    }

    #[test]
    fn a_time_of_day_is_the_next_one_after_now() {
        let wr = blank();
        // Monday 2026-11-23, 08:00 local.
        let now = at((2026, 11, 23), 8, 0);
        let u = |w: &Wait| until(w, &wr, now, &zone()).unwrap();
        // Later today, and tomorrow for a time that has passed or is now.
        assert_eq!(u(&time("09:00", None)), at((2026, 11, 23), 9, 0));
        assert_eq!(u(&time("07:30", None)), at((2026, 11, 24), 7, 30));
        assert_eq!(u(&time("08:00", None)), at((2026, 11, 24), 8, 0));
        // The next Tuesday; today's Monday when its time is still to come, else next week's.
        assert_eq!(
            u(&time("09:00", Some(Weekday::Tue))),
            at((2026, 11, 24), 9, 0)
        );
        assert_eq!(
            u(&time("09:00", Some(Weekday::Mon))),
            at((2026, 11, 23), 9, 0)
        );
        assert_eq!(
            u(&time("07:00", Some(Weekday::Mon))),
            at((2026, 11, 30), 7, 0)
        );
        assert_eq!(
            u(&Wait::Minutes { minutes: 90 }),
            now + TimeDelta::minutes(90)
        );
        assert!(until(&time("25:00", None), &wr, now, &zone()).is_err());
    }

    #[test]
    fn an_input_s_date_and_time_is_read_as_this_pc_s() {
        let mut wr = blank();
        let now = at((2026, 11, 23), 8, 0);
        let wait = Wait::Input {
            input: "publish_at".into(),
        };
        assert_eq!(
            until(&wait, &wr, now, &zone()).unwrap_err(),
            "the input \"publish_at\" holds no date and time"
        );
        wr.inputs
            .insert("publish_at".into(), "2026-11-25T09:00".into());
        assert_eq!(
            until(&wait, &wr, now, &zone()).unwrap(),
            at((2026, 11, 25), 9, 0)
        );
        // A time that has passed is given as it is: the wait ends at once.
        wr.inputs
            .insert("publish_at".into(), "2026-11-20T09:00".into());
        assert!(until(&wait, &wr, now, &zone()).unwrap() < now);
        wr.inputs.insert("publish_at".into(), "next week".into());
        assert_eq!(
            until(&wait, &wr, now, &zone()).unwrap_err(),
            "the input \"publish_at\" is not a date and time: \"next week\""
        );
    }

    /// A workflow run with nothing in it but what `until` reads.
    fn blank() -> WorkflowRun {
        serde_json::from_value(json!({
            "id": "wr", "workflow_id": null, "team_id": "t",
            "definition": {"trigger": {"kind": "manual"}, "steps": []},
            "project_id": null,
            "project": {"name": "Files", "repo_path": "C:/tmp/files"},
            "task": "t", "trigger": "manual", "origin_run_id": null, "test": false,
            "status": "running", "waiting_on": null, "current_step": null,
            "budget": {}, "send_backs": 0, "send_back": null, "spent_runs": 0,
            "spent_tokens": 0, "spent_cache_read": 0, "active_ms": 0, "error": null,
            "started_at": "2026-11-23T00:00:00.000Z", "ended_at": null, "steps": [],
            "inputs": {}
        }))
        .unwrap()
    }
}

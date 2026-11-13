//! The makers' log, what a make is checked against before anything is saved, the make requests
//! that wait for the person, the makers' wakes, and the jobs makers give to the dots they made
//! (dots-make-dots design sections 7.1, 7.2, 8, 10 and 11).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::dots::{dot_from_row, DOT_COLUMNS};
use super::library::check_uses;
use super::runs::{insert_run, run_from_row, run_in, RUN_COLUMNS};
use super::Store;
use crate::library::{LibraryKind, Uses};
use crate::making::jobs::{JobEnd, JobStatus, MakingJob, NewMakingJob};
use crate::making::log::{MakingEvent, MakingEventKind, NewMakingEvent};
use crate::making::reach::Narrowed;
use crate::making::requests::{MakeRequest, NewMakeRequest, RequestKind, RequestStatus};
use crate::making::wake::MakingWake;
use crate::model::{Dot, Message, NewRun, Run};
use crate::util::{new_id, now};
use crate::{Error, Result};

const COLUMNS: &str =
    "id, maker_dot_id, run_id, root_run_id, kind, dot_id, dot_name, detail, created_at";

fn event_from_row(row: &SqliteRow) -> Result<MakingEvent> {
    Ok(MakingEvent {
        id: row.try_get("id")?,
        maker_dot_id: row.try_get("maker_dot_id")?,
        run_id: row.try_get("run_id")?,
        root_run_id: row.try_get("root_run_id")?,
        kind: MakingEventKind::parse(&row.try_get::<String, _>("kind")?)?,
        dot_id: row.try_get("dot_id")?,
        dot_name: row.try_get("dot_name")?,
        detail: serde_json::from_str(&row.try_get::<String, _>("detail")?)?,
        created_at: row.try_get("created_at")?,
    })
}

const WAKE_COLUMNS: &str =
    "id, maker_dot_id, root_run_id, message, created_at, run_id, delivered_at";

fn wake_from_row(row: &SqliteRow) -> Result<MakingWake> {
    Ok(MakingWake {
        id: row.try_get("id")?,
        maker_dot_id: row.try_get("maker_dot_id")?,
        root_run_id: row.try_get("root_run_id")?,
        message: row.try_get("message")?,
        created_at: row.try_get("created_at")?,
        run_id: row.try_get("run_id")?,
        delivered_at: row.try_get("delivered_at")?,
    })
}

const REQUEST_COLUMNS: &str = "id, maker_dot_id, run_id, root_run_id, kind, dot_id, name, \
     purpose, spec, without, extras, over, create_folder, first_job, status, gone_reason, \
     created_at, decided_at, made_dot_id, ask, make_ask";

fn request_from_row(row: &SqliteRow) -> Result<MakeRequest> {
    let over: Option<String> = row.try_get("over")?;
    let ask: Option<String> = row.try_get("ask")?;
    let make_ask: Option<String> = row.try_get("make_ask")?;
    Ok(MakeRequest {
        id: row.try_get("id")?,
        maker_dot_id: row.try_get("maker_dot_id")?,
        run_id: row.try_get("run_id")?,
        root_run_id: row.try_get("root_run_id")?,
        kind: RequestKind::parse(&row.try_get::<String, _>("kind")?)?,
        dot_id: row.try_get("dot_id")?,
        name: row.try_get("name")?,
        purpose: row.try_get("purpose")?,
        spec: serde_json::from_str(&row.try_get::<String, _>("spec")?)?,
        without: serde_json::from_str(&row.try_get::<String, _>("without")?)?,
        extras: serde_json::from_str(&row.try_get::<String, _>("extras")?)?,
        over: over.as_deref().map(serde_json::from_str).transpose()?,
        create_folder: row.try_get("create_folder")?,
        first_job: row.try_get("first_job")?,
        status: RequestStatus::parse(&row.try_get::<String, _>("status")?)?,
        gone_reason: row.try_get("gone_reason")?,
        created_at: row.try_get("created_at")?,
        decided_at: row.try_get("decided_at")?,
        made_dot_id: row.try_get("made_dot_id")?,
        ask: ask.as_deref().map(serde_json::from_str).transpose()?,
        make_ask: make_ask.as_deref().map(serde_json::from_str).transpose()?,
    })
}

/// The make requests whose rows could not be read, each logged once.
static UNREADABLE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Logs once that the request row `id` could not be read, and why.
fn log_unreadable(id: &str, e: &Error) {
    let Ok(mut logged) = UNREADABLE.lock() else {
        return;
    };
    if !logged.iter().any(|seen| seen == id) {
        logged.push(id.to_string());
        tracing::error!(request = %id, "a make request could not be read and is left out: {e}");
    }
}

/// The statuses of a request the person has not answered: one that waits, and one marked gone,
/// which waits for its "Don't make it".
const UNANSWERED: &str = "status IN ('waiting', 'gone')";

/// Make requests (dots-make-dots design sections 7.1 and 7.2).
impl Store {
    /// Stores a make request, waiting, as of now. Its name is the name of `new.spec`. An error
    /// when the maker does not exist.
    pub async fn add_make_request(&self, new: &NewMakeRequest<'_>) -> Result<MakeRequest> {
        let request = MakeRequest {
            id: new_id(),
            maker_dot_id: new.maker_dot_id.to_string(),
            run_id: new.run_id.to_string(),
            root_run_id: new.root_run_id.to_string(),
            kind: new.kind,
            dot_id: new.dot_id.map(str::to_string),
            name: new.spec.name.trim().to_string(),
            purpose: new.purpose.to_string(),
            spec: new.spec.clone(),
            without: new.without.clone(),
            extras: new.extras.to_vec(),
            over: new.over.cloned(),
            create_folder: new.create_folder.map(str::to_string),
            first_job: new.first_job.map(str::to_string),
            status: RequestStatus::Waiting,
            gone_reason: None,
            created_at: now(),
            decided_at: None,
            made_dot_id: None,
            ask: new.ask.cloned(),
            make_ask: new.make_ask.cloned(),
        };
        let over = match &request.over {
            Some(over) => Some(serde_json::to_string(over)?),
            None => None,
        };
        let ask = match &request.ask {
            Some(ask) => Some(serde_json::to_string(ask)?),
            None => None,
        };
        let make_ask = match &request.make_ask {
            Some(ask) => Some(serde_json::to_string(ask)?),
            None => None,
        };
        sqlx::query(
            "INSERT INTO make_requests (id, maker_dot_id, run_id, root_run_id, kind, dot_id, \
             name, purpose, spec, without, extras, over, create_folder, first_job, status, \
             created_at, ask, make_ask) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&request.id)
        .bind(&request.maker_dot_id)
        .bind(&request.run_id)
        .bind(&request.root_run_id)
        .bind(request.kind.as_str())
        .bind(&request.dot_id)
        .bind(&request.name)
        .bind(&request.purpose)
        .bind(serde_json::to_string(&request.spec)?)
        .bind(serde_json::to_string(&request.without)?)
        .bind(serde_json::to_string(&request.extras)?)
        .bind(over)
        .bind(&request.create_folder)
        .bind(&request.first_job)
        .bind(request.status.as_str())
        .bind(&request.created_at)
        .bind(ask)
        .bind(make_ask)
        .execute(&self.pool)
        .await?;
        Ok(request)
    }

    /// The request `id`, answered or not.
    pub async fn get_make_request(&self, id: &str) -> Result<MakeRequest> {
        let row = sqlx::query(&format!(
            "SELECT {REQUEST_COLUMNS} FROM make_requests WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("make request {id}")))?;
        request_from_row(&row)
    }

    /// The requests of the maker `maker_dot_id`, or of every maker with `None`, oldest first.
    /// With `waiting_only`, those the person has not answered: the ones that wait and the ones
    /// marked gone, which still need their "Don't make it". A row that cannot be read is left
    /// out and logged, so it does not hide the others.
    pub async fn list_make_requests(
        &self,
        maker_dot_id: Option<&str>,
        waiting_only: bool,
    ) -> Result<Vec<MakeRequest>> {
        let unanswered = if waiting_only { UNANSWERED } else { "1" };
        let rows = sqlx::query(&format!(
            "SELECT {REQUEST_COLUMNS} FROM make_requests \
             WHERE (?1 IS NULL OR maker_dot_id = ?1) AND {unanswered} \
             ORDER BY created_at, rowid"
        ))
        .bind(maker_dot_id)
        .fetch_all(&self.pool)
        .await?;
        let mut requests = Vec::with_capacity(rows.len());
        for row in &rows {
            match request_from_row(row) {
                Ok(request) => requests.push(request),
                Err(e) => log_unreadable(&row.try_get::<String, _>("id").unwrap_or_default(), &e),
            }
        }
        Ok(requests)
    }

    /// How many requests of the maker the person has not answered: what the limit of waiting
    /// requests counts (dots-make-dots design section 7.2). One marked gone counts until it is
    /// refused, since its card is still before the person.
    pub async fn count_waiting_make_requests(&self, maker_dot_id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM make_requests WHERE maker_dot_id = ? AND {UNANSWERED}"
        ))
        .bind(maker_dot_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
    }

    /// Writes the person's answer on the request `id`: `status`, now as the time, and the dot
    /// the answer made. Only a request that waits is moved, or one marked gone when `status`
    /// is `refused`. `false`, with nothing written, for any other.
    pub async fn decide_make_request(
        &self,
        id: &str,
        status: RequestStatus,
        made_dot_id: Option<&str>,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE make_requests SET status = ?1, decided_at = ?2, made_dot_id = ?3 \
             WHERE id = ?4 AND (status = 'waiting' OR (status = 'gone' AND ?1 = 'refused'))",
        )
        .bind(status.as_str())
        .bind(now())
        .bind(made_dot_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Marks the waiting request `id` gone: something it names is no longer there, and
    /// `reason` says what, in one sentence for the person. `false` when it did not wait.
    pub async fn mark_make_request_gone(&self, id: &str, reason: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE make_requests SET status = 'gone', gone_reason = ? \
             WHERE id = ? AND status = 'waiting'",
        )
        .bind(reason)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Whether the Library has an item of `kind` named `name` that a dot can name in its
    /// `uses`: one of its own, not an MCP server that comes with a plugin (`check_uses`).
    pub async fn library_has(&self, kind: LibraryKind, name: &str) -> Result<bool> {
        Ok(sqlx::query(
            "SELECT 1 FROM library_items WHERE kind = ? AND name = ? AND owner_id IS NULL",
        )
        .bind(kind.as_str())
        .bind(name)
        .fetch_optional(&self.pool)
        .await?
        .is_some())
    }
}

/// The makers' wakes (dots-make-dots design sections 7.2 and 8).
impl Store {
    /// Stores a wake of the chat topic `root_run_id` of the maker `maker_dot_id`, waiting, as
    /// of now. An error when the maker does not exist.
    pub async fn add_making_wake(
        &self,
        maker_dot_id: &str,
        root_run_id: &str,
        message: &str,
    ) -> Result<MakingWake> {
        let wake = MakingWake {
            id: new_id(),
            maker_dot_id: maker_dot_id.to_string(),
            root_run_id: root_run_id.to_string(),
            message: message.to_string(),
            created_at: now(),
            run_id: None,
            delivered_at: None,
        };
        sqlx::query(
            "INSERT INTO making_wakes (id, maker_dot_id, root_run_id, message, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&wake.id)
        .bind(&wake.maker_dot_id)
        .bind(&wake.root_run_id)
        .bind(&wake.message)
        .bind(&wake.created_at)
        .execute(&self.pool)
        .await?;
        Ok(wake)
    }

    /// The wake `id`, waiting or not.
    pub async fn get_making_wake(&self, id: &str) -> Result<MakingWake> {
        let row = sqlx::query(&format!(
            "SELECT {WAKE_COLUMNS} FROM making_wakes WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("wake {id}")))?;
        wake_from_row(&row)
    }

    /// The wakes that wait, of every maker, oldest first. Of two stored in the same
    /// millisecond, the one stored first comes first (`rowid`).
    pub async fn waiting_making_wakes(&self) -> Result<Vec<MakingWake>> {
        let rows = sqlx::query(&format!(
            "SELECT {WAKE_COLUMNS} FROM making_wakes WHERE delivered_at IS NULL \
             ORDER BY created_at, rowid"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(wake_from_row).collect()
    }

    /// Delivers the waiting wake `id`: stores `new`, the run that carries its message, and
    /// marks the wake with that run, in one transaction, so a wake has a run or waits and never
    /// both. The run is read in that transaction too, so a delivered wake always gives its run
    /// back to be announced. `None`, with nothing written, when the wake no longer waits or is
    /// gone with its maker.
    pub async fn deliver_making_wake(&self, id: &str, new: &NewRun) -> Result<Option<Run>> {
        let mut tx = self.pool.begin().await?;
        // The guarded update goes first: it takes the write lock, so of two deliveries of one
        // wake the second finds it delivered and stores no run.
        let res = sqlx::query(
            "UPDATE making_wakes SET delivered_at = ? WHERE id = ? AND delivered_at IS NULL",
        )
        .bind(now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        let run_id = insert_run(&mut tx, new).await?;
        sqlx::query("UPDATE making_wakes SET run_id = ? WHERE id = ?")
            .bind(&run_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let run = run_in(&mut tx, &run_id).await?;
        tx.commit().await?;
        Ok(Some(run))
    }

    /// Drops the waiting wake `id`: it no longer waits and no run delivered it. `false` when
    /// it did not wait.
    pub async fn drop_making_wake(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE making_wakes SET delivered_at = ? WHERE id = ? AND delivered_at IS NULL",
        )
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
}

const JOB_COLUMNS: &str = "id, maker_dot_id, maker_run_id, maker_root_run_id, dot_id, \
     dot_root_run_id, first_run_id, last_run_id, message, status, ended_how, created_at, \
     ended_at, wake_id";

fn job_from_row(row: &SqliteRow) -> Result<MakingJob> {
    let ended_how: Option<String> = row.try_get("ended_how")?;
    Ok(MakingJob {
        id: row.try_get("id")?,
        maker_dot_id: row.try_get("maker_dot_id")?,
        maker_run_id: row.try_get("maker_run_id")?,
        maker_root_run_id: row.try_get("maker_root_run_id")?,
        dot_id: row.try_get("dot_id")?,
        dot_root_run_id: row.try_get("dot_root_run_id")?,
        first_run_id: row.try_get("first_run_id")?,
        last_run_id: row.try_get("last_run_id")?,
        message: row.try_get("message")?,
        status: JobStatus::parse(&row.try_get::<String, _>("status")?)?,
        ended_how: ended_how.as_deref().map(JobEnd::parse).transpose()?,
        created_at: row.try_get("created_at")?,
        ended_at: row.try_get("ended_at")?,
        wake_id: row.try_get("wake_id")?,
    })
}

/// The jobs makers give to the dots they made (dots-make-dots design section 8).
impl Store {
    /// Starts a job: stores `run`, the made dot's run that carries it, the job itself, at that
    /// run and `working`, and the `job_started` entry of the maker's log, in one transaction.
    /// So a job has its run from the start, and the log, which the loop guard counts, has every
    /// job that was started. `new.id` is the job's id, which the run's payload names already;
    /// `dot_name` is the made dot's name, for the log. Returns the job and its run.
    pub async fn start_making_job(
        &self,
        new: &NewMakingJob<'_>,
        dot_name: &str,
        run: &NewRun,
    ) -> Result<(MakingJob, Run)> {
        let created_at = now();
        let mut tx = self.pool.begin().await?;
        let run_id = insert_run(&mut tx, run).await?;
        let root = run.root_run_id.clone().unwrap_or_else(|| run_id.clone());
        sqlx::query(&format!(
            "INSERT INTO making_jobs ({JOB_COLUMNS}) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL, NULL)"
        ))
        .bind(new.id)
        .bind(new.maker_dot_id)
        .bind(new.maker_run_id)
        .bind(new.maker_root_run_id)
        .bind(new.dot_id)
        .bind(&root)
        .bind(&run_id)
        .bind(&run_id)
        .bind(new.message)
        .bind(JobStatus::Working.as_str())
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
        let detail = serde_json::json!({
            "job_id": new.id,
            "message": new.message,
            "run_id": run_id,
        });
        sqlx::query(&format!(
            "INSERT INTO making_events ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(new_id())
        .bind(new.maker_dot_id)
        .bind(new.maker_run_id)
        .bind(new.maker_root_run_id)
        .bind(MakingEventKind::JobStarted.as_str())
        .bind(new.dot_id)
        .bind(dot_name)
        .bind(detail.to_string())
        .bind(&created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((
            self.get_making_job(new.id).await?,
            self.get_run(&run_id).await?,
        ))
    }

    /// The job `id`, ended or not.
    pub async fn get_making_job(&self, id: &str) -> Result<MakingJob> {
        let row = sqlx::query(&format!(
            "SELECT {JOB_COLUMNS} FROM making_jobs WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("job {id}")))?;
        job_from_row(&row)
    }

    /// The jobs given in the maker's chat topic `maker_root_run_id`, oldest first; of two
    /// stored in the same millisecond, the one stored first comes first (`rowid`).
    pub async fn making_jobs_of_topic(&self, maker_root_run_id: &str) -> Result<Vec<MakingJob>> {
        let rows = sqlx::query(&format!(
            "SELECT {JOB_COLUMNS} FROM making_jobs WHERE maker_root_run_id = ? \
             ORDER BY created_at, rowid"
        ))
        .bind(maker_root_run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(job_from_row).collect()
    }

    /// The jobs that have not ended, of every maker, oldest first; with `dot_id`, only those
    /// given to that made dot.
    pub async fn open_making_jobs(&self, dot_id: Option<&str>) -> Result<Vec<MakingJob>> {
        let rows = sqlx::query(&format!(
            "SELECT {JOB_COLUMNS} FROM making_jobs \
             WHERE status != 'ended' AND (?1 IS NULL OR dot_id = ?1) ORDER BY created_at, rowid"
        ))
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(job_from_row).collect()
    }

    /// The newest job given to the made dot `dot_id` in the maker's chat topic
    /// `maker_root_run_id`, ended or not: the next job from that topic goes on in its topic of
    /// the made dot (dots-make-dots design section 8).
    pub async fn latest_making_job(
        &self,
        maker_root_run_id: &str,
        dot_id: &str,
    ) -> Result<Option<MakingJob>> {
        let row = sqlx::query(&format!(
            "SELECT {JOB_COLUMNS} FROM making_jobs WHERE maker_root_run_id = ? AND dot_id = ? \
             ORDER BY created_at DESC, rowid DESC LIMIT 1"
        ))
        .bind(maker_root_run_id)
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(job_from_row).transpose()
    }

    /// How many jobs were started in the maker's chat topic `maker_root_run_id` at or after
    /// `since`, or ever with `None`: what the loop guard counts (dots-make-dots design section
    /// 8). It counts the log, not the jobs, so a job whose made dot was deleted still counts.
    pub async fn count_jobs_started(
        &self,
        maker_root_run_id: &str,
        since: Option<&str>,
    ) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM making_events WHERE root_run_id = ?1 \
             AND kind = 'job_started' AND (?2 IS NULL OR created_at >= ?2)",
        )
        .bind(maker_root_run_id)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
    }

    /// Moves the open job `id` on from its run `from` to the run `to`, which continues it, and
    /// marks it `working`. `false`, with nothing written, when the job has ended or is no
    /// longer at `from`.
    pub async fn move_making_job(&self, id: &str, from: &str, to: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE making_jobs SET last_run_id = ?, status = 'working' \
             WHERE id = ? AND status != 'ended' AND last_run_id = ?",
        )
        .bind(to)
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Marks the working job `id` as waiting for the person's answer. `false`, with nothing
    /// written, when it was not `working`.
    pub async fn set_making_job_waiting(&self, id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE making_jobs SET status = 'waiting_answer' WHERE id = ? AND status = 'working'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Stores the person's Send again of the run `carry_from`, which carried the job `job_id`
    /// (dots-make-dots design section 8): `open` while the job has not ended, `ended` once it
    /// has or is gone, as the job is read in the transaction that stores the run. The files of
    /// `carry_from` move to it. With `end_making_job`, which does not end a job whose run a
    /// stored run sends again, the job either goes on in the new run or ended before it.
    pub async fn create_resent_job_run(
        &self,
        open: &NewRun,
        ended: &NewRun,
        job_id: &str,
        carry_from: &str,
    ) -> Result<Run> {
        // Immediate: the job is read under the write lock the insert takes.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM making_jobs WHERE id = ?")
                .bind(job_id)
                .fetch_optional(&mut *tx)
                .await?;
        let new = match status.as_deref() {
            Some(status) if status != JobStatus::Ended.as_str() => open,
            _ => ended,
        };
        let id = insert_run(&mut tx, new).await?;
        sqlx::query("UPDATE chat_attachments SET run_id = ? WHERE run_id = ?")
            .bind(&id)
            .bind(carry_from)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_run(&id).await
    }

    /// Ends the open job `id` as `how` and stores the wake that tells its maker, `message`
    /// for the maker's topic the job was given in, in one transaction: a job that has ended
    /// has exactly one wake. The guarded update goes first, so of two that end one job the
    /// second finds it ended. `None`, with nothing written, when the job had ended or is gone,
    /// and when a run goes on with the work of the job's run (`run_continued_by`), as one the
    /// person sent again since that run was read: the next pass moves the job to it.
    pub async fn end_making_job(
        &self,
        id: &str,
        how: JobEnd,
        message: &str,
    ) -> Result<Option<MakingJob>> {
        let ended_at = now();
        let wake_id = new_id();
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE making_jobs SET status = 'ended', ended_how = ?, ended_at = ?, wake_id = ? \
             WHERE id = ? AND status != 'ended' AND NOT EXISTS (SELECT 1 FROM runs \
             WHERE (runs.parent_run_id = making_jobs.last_run_id \
             AND runs.trigger_kind = 'resume') OR runs.retry_of = making_jobs.last_run_id)",
        )
        .bind(how.as_str())
        .bind(&ended_at)
        .bind(&wake_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        sqlx::query(
            "INSERT INTO making_wakes (id, maker_dot_id, root_run_id, message, created_at) \
             SELECT ?, maker_dot_id, maker_root_run_id, ?, ? FROM making_jobs WHERE id = ?",
        )
        .bind(&wake_id)
        .bind(message)
        .bind(&ended_at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_making_job(id).await.map(Some)
    }

    /// Whether the maker of the job `id` was told that its run waits for a cap
    /// (`tell_making_job_held`), ever: its `held_told_at` is set. `false` for a job that is
    /// gone.
    pub async fn making_job_held_told(&self, id: &str) -> Result<bool> {
        let told: Option<Option<String>> =
            sqlx::query_scalar("SELECT held_told_at FROM making_jobs WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(matches!(told, Some(Some(_))))
    }

    /// Tells the maker of the open job `id` once that its run waits (design section 8): marks
    /// the job told (`held_told_at`) and stores the wake `message` for the maker's topic the
    /// job was given in, in one transaction. `false`, with nothing written, when the job was
    /// told already, has ended or is gone. A job is told once, ever: `held_told_at` is never
    /// cleared, so a later run of the job held again, as a run resumed after an approval may
    /// be, is not told.
    pub async fn tell_making_job_held(&self, id: &str, message: &str) -> Result<bool> {
        let at = now();
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE making_jobs SET held_told_at = ? \
             WHERE id = ? AND status != 'ended' AND held_told_at IS NULL",
        )
        .bind(&at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO making_wakes (id, maker_dot_id, root_run_id, message, created_at) \
             SELECT ?, maker_dot_id, maker_root_run_id, ?, ? FROM making_jobs WHERE id = ?",
        )
        .bind(new_id())
        .bind(message)
        .bind(&at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The run that goes on with the work of the ended run `run_id` in the same turn, if there
    /// is one: the run that resumes it after an approval (`resume_parked` stores it as it
    /// closes the parked run), or the run that sends its message again (`retry_of`). The
    /// newest, when a message was sent again more than once.
    pub async fn run_continued_by(&self, run_id: &str) -> Result<Option<Run>> {
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs \
             WHERE (parent_run_id = ?1 AND trigger_kind = 'resume') OR retry_of = ?1 \
             ORDER BY queued_at DESC, id DESC LIMIT 1"
        ))
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(run_from_row).transpose()
    }

    /// The newest question a run of the chat topic `root_run_id` asked the person, among the
    /// runs queued at or after `since`: the question a job that runs in that topic since then
    /// waits on, answered, dismissed or open.
    pub async fn latest_question_since(
        &self,
        root_run_id: &str,
        since: &str,
    ) -> Result<Option<Message>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT m.id FROM messages m JOIN runs r ON r.id = m.run_id \
             WHERE m.kind = 'question' AND r.root_run_id = ? AND r.queued_at >= ? \
             ORDER BY m.created_at DESC, m.rowid DESC LIMIT 1",
        )
        .bind(root_run_id)
        .bind(since)
        .fetch_optional(&self.pool)
        .await?;
        match id {
            Some(id) => self.get_message(&id).await.map(Some),
            None => Ok(None),
        }
    }
}

impl Store {
    /// Writes one entry of the makers' log, as of now.
    pub async fn add_making_event(&self, new: &NewMakingEvent<'_>) -> Result<MakingEvent> {
        let event = MakingEvent {
            id: new_id(),
            maker_dot_id: new.maker_dot_id.to_string(),
            run_id: new.run_id.to_string(),
            root_run_id: new.root_run_id.to_string(),
            kind: new.kind,
            dot_id: new.dot_id.map(str::to_string),
            dot_name: new.dot_name.to_string(),
            detail: new.detail.clone(),
            created_at: now(),
        };
        sqlx::query(&format!(
            "INSERT INTO making_events ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(&event.id)
        .bind(&event.maker_dot_id)
        .bind(&event.run_id)
        .bind(&event.root_run_id)
        .bind(event.kind.as_str())
        .bind(&event.dot_id)
        .bind(&event.dot_name)
        .bind(serde_json::to_string(&event.detail)?)
        .bind(&event.created_at)
        .execute(&self.pool)
        .await?;
        Ok(event)
    }

    /// The log of one chat topic of a maker, oldest first: its cards in the order things
    /// happened (dots-make-dots design section 11). Of two written in the same millisecond, the
    /// one stored first comes first (`rowid`).
    pub async fn list_making_events(&self, root_run_id: &str) -> Result<Vec<MakingEvent>> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM making_events WHERE root_run_id = ? \
             ORDER BY created_at, rowid"
        ))
        .bind(root_run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// The makers' chat topics that show something of the dot `dot_id`, each as its maker and
    /// its first run: the topics whose log names the dot (it was made, changed or given a job
    /// there, or a change of it was asked), and, when the dot is a maker, its own topics with a
    /// request the person has not answered. Read before the dot is deleted, they are the
    /// topics to tell afterwards (dots-make-dots design section 11).
    pub async fn making_topics_of(&self, dot_id: &str) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(&format!(
            "SELECT maker_dot_id, root_run_id FROM making_events WHERE dot_id = ?1 \
             UNION \
             SELECT maker_dot_id, root_run_id FROM make_requests \
             WHERE (maker_dot_id = ?1 OR dot_id = ?1) AND {UNANSWERED} \
             ORDER BY 1, 2"
        ))
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("maker_dot_id")?, row.try_get("root_run_id")?)))
            .collect()
    }

    /// How many dots the run `run_id` made: what a maker's limit for one run counts
    /// (dots-make-dots design section 10). It counts the log, so a made dot that was deleted
    /// since still counts, and the dots that record the run as the one that made them and have
    /// no entry in the log, so a dot whose entry was never written, because something failed
    /// after the dot was stored, counts too.
    pub async fn count_made_in_run(&self, run_id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM making_events WHERE run_id = ?1 AND kind = 'made') \
             + (SELECT COUNT(*) FROM dots d WHERE d.made_in_run = ?1 AND NOT EXISTS \
                (SELECT 1 FROM making_events e WHERE e.kind = 'made' AND e.dot_id = d.id))",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
    }

    /// The new folder of its own the made dot `dot_id` was made with, as the `made` entry of the
    /// log keeps it; `None` when it was made with a folder it was given, or the entry has none.
    pub async fn made_own_folder(&self, dot_id: &str) -> Result<Option<String>> {
        let folder: Option<Option<String>> = sqlx::query_scalar(
            "SELECT json_extract(detail, '$.folder') FROM making_events \
             WHERE dot_id = ? AND kind = 'made' ORDER BY created_at, rowid LIMIT 1",
        )
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(folder.flatten())
    }

    /// Refuses a name of `uses` that the Library does not have, as saving a dot with them
    /// would (`Invalid`), without saving anything: so a make that could never be allowed is
    /// refused before a request for it is kept (dots-make-dots design section 7.1).
    pub async fn check_uses_exist(&self, uses: &Uses) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        check_uses(&mut conn, uses).await
    }

    /// Whether a dot has `name`, in any letter case: what saving a new dot of that name would
    /// refuse.
    pub async fn dot_name_taken(&self, name: &str) -> Result<bool> {
        Ok(
            sqlx::query("SELECT 1 FROM dots WHERE name = ? COLLATE NOCASE")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?
                .is_some(),
        )
    }

    /// Whether a dot other than `id` has `name`, in any letter case: what renaming the dot `id`
    /// to that name would refuse. A dot may take its own name in another case.
    pub async fn dot_name_taken_by_another(&self, name: &str, id: &str) -> Result<bool> {
        Ok(
            sqlx::query("SELECT 1 FROM dots WHERE name = ? COLLATE NOCASE AND id != ?")
                .bind(name)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .is_some(),
        )
    }
}

/// The dots a maker made, as its tools name and show them (dots-make-dots design section 9).
impl Store {
    /// The dot named `name`, in any letter case, whose recorded maker is `maker_id`; `None`
    /// when no dot has that name or the dot is not one this maker made.
    pub async fn made_dot_named(&self, maker_id: &str, name: &str) -> Result<Option<Dot>> {
        let row = sqlx::query(&format!(
            "SELECT {DOT_COLUMNS} FROM dots WHERE name = ? COLLATE NOCASE AND made_by_dot_id = ?"
        ))
        .bind(name)
        .bind(maker_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(dot_from_row).transpose()
    }

    /// Writes the maker's one line on what the made dot `dot_id` is for. Who made it stays as
    /// it is. `NotFound` for an unknown dot.
    pub async fn set_made_purpose(&self, dot_id: &str, purpose: &str) -> Result<()> {
        let res = sqlx::query("UPDATE dots SET made_purpose = ? WHERE id = ?")
            .bind(purpose)
            .bind(dot_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {dot_id}")));
        }
        Ok(())
    }

    /// What the person narrowed on the made dot `dot_id` since its maker last set it
    /// (`Narrowed`, dots-make-dots design section 9); empty when nothing was. `NotFound` for a
    /// dot that does not exist.
    pub async fn made_narrowed(&self, dot_id: &str) -> Result<Vec<Narrowed>> {
        let kept: Option<Option<String>> =
            sqlx::query_scalar("SELECT made_narrowed FROM dots WHERE id = ?")
                .bind(dot_id)
                .fetch_optional(&self.pool)
                .await?;
        match kept {
            None => Err(Error::NotFound(format!("dot {dot_id}"))),
            Some(None) => Ok(Vec::new()),
            Some(Some(text)) => Ok(serde_json::from_str(&text)?),
        }
    }

    /// Keeps `narrowed` as what the person narrowed on the made dot `dot_id`; an empty list
    /// clears it.
    pub async fn set_made_narrowed(&self, dot_id: &str, narrowed: &[Narrowed]) -> Result<()> {
        let text = if narrowed.is_empty() {
            None
        } else {
            Some(serde_json::to_string(narrowed)?)
        };
        let res = sqlx::query("UPDATE dots SET made_narrowed = ? WHERE id = ?")
            .bind(text)
            .bind(dot_id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("dot {dot_id}")));
        }
        Ok(())
    }

    /// The run of `dot_id` that ended last, if any has ended.
    pub async fn last_ended_run(&self, dot_id: &str) -> Result<Option<Run>> {
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM runs WHERE dot_id = ? \
             AND status IN ('succeeded', 'failed', 'cancelled') AND ended_at IS NOT NULL \
             ORDER BY ended_at DESC, id DESC LIMIT 1"
        ))
        .bind(dot_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(run_from_row).transpose()
    }
}

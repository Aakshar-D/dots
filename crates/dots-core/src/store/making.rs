//! The makers' log, what a make is checked against before anything is saved, the make requests
//! that wait for the person, and the makers' wakes (dots-make-dots design sections 7.1, 7.2, 8,
//! 10 and 11).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::dots::{dot_from_row, DOT_COLUMNS};
use super::library::check_uses;
use super::runs::{insert_run, run_from_row, RUN_COLUMNS};
use super::Store;
use crate::library::{LibraryKind, Uses};
use crate::making::log::{MakingEvent, MakingEventKind, NewMakingEvent};
use crate::making::requests::{MakeRequest, NewMakeRequest, RequestKind, RequestStatus};
use crate::making::wake::MakingWake;
use crate::model::{Dot, NewRun, Run};
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
     created_at, decided_at, made_dot_id, ask";

fn request_from_row(row: &SqliteRow) -> Result<MakeRequest> {
    let over: Option<String> = row.try_get("over")?;
    let ask: Option<String> = row.try_get("ask")?;
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
    })
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
        };
        let over = match &request.over {
            Some(over) => Some(serde_json::to_string(over)?),
            None => None,
        };
        let ask = match &request.ask {
            Some(ask) => Some(serde_json::to_string(ask)?),
            None => None,
        };
        sqlx::query(
            "INSERT INTO make_requests (id, maker_dot_id, run_id, root_run_id, kind, dot_id, \
             name, purpose, spec, without, extras, over, create_folder, first_job, status, \
             created_at, ask) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    /// marked gone, which still need their "Don't make it".
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
        rows.iter().map(request_from_row).collect()
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
    /// both. `None`, with nothing written, when the wake no longer waits or is gone with its
    /// maker.
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
        tx.commit().await?;
        self.get_run(&run_id).await.map(Some)
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

    /// How many dots the run `run_id` made: what a maker's limit for one run counts
    /// (dots-make-dots design section 10). It counts the log, not the dots, so a made dot that
    /// was deleted since still counts.
    pub async fn count_made_in_run(&self, run_id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM making_events WHERE run_id = ? AND kind = 'made'",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
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

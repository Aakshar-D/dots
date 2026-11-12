//! The makers' log, what a make is checked against before anything is saved, and the makers'
//! wakes (dots-make-dots design sections 7.1, 8, 10 and 11).

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::library::check_uses;
use super::runs::insert_run;
use super::Store;
use crate::library::Uses;
use crate::making::log::{MakingEvent, MakingEventKind, NewMakingEvent};
use crate::making::wake::MakingWake;
use crate::model::{NewRun, Run};
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
}

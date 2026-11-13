//! The `topics` rows (topic tools spec section 4): a topic's title, its archive mark and the
//! mark that its next run starts a fresh session; and the moves and merges that rewrite which
//! topic runs belong to.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::Row;

use super::runs::{line_order, run_from_row, RUN_COLUMNS};
use super::Store;
use crate::model::{Run, TopicInfo};
use crate::util::{now, timestamp};
use crate::{Error, Result};

/// The longest title a topic takes, in characters.
pub const MAX_TOPIC_TITLE: usize = 100;
/// What a move of a topic's first message is told.
pub const MOVE_ROOT: &str =
    "this is the topic's first message; to move the whole topic, merge it into another";
/// What a move or merge of a topic that is still working is told.
pub const TOPIC_BUSY: &str = "a topic that is still working cannot be changed; wait until its \
    runs have ended, or answer what it waits for";

const TOPIC_COLUMNS: &str = "root_run_id, dot_id, title, archived_at";

fn topic_from_row(row: &SqliteRow) -> Result<TopicInfo> {
    Ok(TopicInfo {
        root_run_id: row.try_get("root_run_id")?,
        dot_id: row.try_get("dot_id")?,
        title: row.try_get("title")?,
        archived_at: row.try_get("archived_at")?,
    })
}

/// The topic's runs, oldest first.
async fn thread(conn: &mut SqliteConnection, root: &str) -> Result<Vec<Run>> {
    let rows = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE root_run_id = ? ORDER BY queued_at, id"
    ))
    .bind(root)
    .fetch_all(&mut *conn)
    .await?;
    let runs = rows.iter().map(run_from_row).collect::<Result<Vec<_>>>()?;
    Ok(line_order(runs))
}

/// The topic whose root is `root`, oldest first; `NotFound` when no run has that id, `Invalid`
/// when the run is not a topic's root.
async fn topic(conn: &mut SqliteConnection, root: &str) -> Result<Vec<Run>> {
    let runs = thread(conn, root).await?;
    if runs.iter().any(|r| r.id == root) {
        return Ok(runs);
    }
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM runs WHERE id = ?")
        .bind(root)
        .fetch_optional(&mut *conn)
        .await?;
    Err(match exists {
        Some(_) => Error::Invalid(format!("run {root} is not the first run of a topic")),
        None => Error::NotFound(format!("topic {root}")),
    })
}

/// Refuses to change a topic that works or waits, one a workflow step ran in, or one an app
/// started (it tracks the topic by its root).
async fn check_changeable(conn: &mut SqliteConnection, runs: &[Run]) -> Result<()> {
    if runs.iter().any(|r| !r.status.is_terminal()) {
        return Err(Error::Conflict(TOPIC_BUSY.into()));
    }
    if runs.iter().any(|r| r.workflow_step_run_id.is_some()) {
        return Err(Error::Invalid(
            "a topic a workflow step ran in belongs to the workflow and cannot be changed".into(),
        ));
    }
    let Some(first) = runs.first() else {
        return Ok(());
    };
    let app: Option<String> =
        sqlx::query_scalar("SELECT root_run_id FROM app_topics WHERE root_run_id = ?")
            .bind(&first.root_run_id)
            .fetch_optional(&mut *conn)
            .await?;
    if app.is_some() {
        return Err(Error::Invalid(
            "a topic an app started belongs to the app and cannot be changed".into(),
        ));
    }
    Ok(())
}

/// Marks the topic `root` of `dot_id` to start its next run afresh, making its row if needed.
async fn mark_restart(conn: &mut SqliteConnection, root: &str, dot_id: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO topics (root_run_id, dot_id, restart) VALUES (?, ?, 1) \
         ON CONFLICT(root_run_id) DO UPDATE SET restart = 1",
    )
    .bind(root)
    .bind(dot_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Brings back the topic `root` if it was archived: what was moved or merged into it is wanted.
async fn unarchive(conn: &mut SqliteConnection, root: &str) -> Result<()> {
    sqlx::query(
        "UPDATE topics SET archived_at = NULL, unarchived_at = ? \
         WHERE root_run_id = ? AND archived_at IS NOT NULL",
    )
    .bind(now())
    .bind(root)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Sets `root_run_id = root` on the runs `ids`, and on the proposals they made.
async fn set_root(conn: &mut SqliteConnection, ids: &[String], root: &str) -> Result<()> {
    for table in ["runs", "app_proposals", "instruction_proposals"] {
        let key = if table == "runs" { "id" } else { "run_id" };
        let marks = vec!["?"; ids.len()].join(", ");
        let sql = format!("UPDATE {table} SET root_run_id = ? WHERE {key} IN ({marks})");
        let mut q = sqlx::query(&sql).bind(root);
        for id in ids {
            q = q.bind(id);
        }
        q.execute(&mut *conn).await?;
    }
    Ok(())
}

/// Gives each file of the runs `moving` that takes a name the topic `target` has (in any
/// case) a name of its own, with ` (2)`… as a message's files get (attachments spec section
/// 3.1), in its row and in its run's payload, and marks it not placed: the next run puts it in
/// the topic's folder under that name. Call before the runs take the target's root.
async fn rename_clashing_files(
    conn: &mut SqliteConnection,
    moving: &[String],
    target: &str,
) -> Result<()> {
    let marks = vec!["?"; moving.len()].join(", ");
    let sql = format!(
        "SELECT a.name FROM chat_attachments a JOIN runs r ON r.id = a.run_id \
         WHERE r.root_run_id = ? AND a.name IS NOT NULL AND a.run_id NOT IN ({marks})"
    );
    let mut q = sqlx::query_scalar::<_, String>(&sql).bind(target);
    for id in moving {
        q = q.bind(id);
    }
    let mut taken = q.fetch_all(&mut *conn).await?;
    // A watcher's root names the files it staged in the same folder.
    let root_payload: Option<Option<String>> =
        sqlx::query_scalar("SELECT payload FROM runs WHERE id = ? AND trigger_kind = 'watcher'")
            .bind(target)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(Some(payload)) = root_payload {
        let payload: serde_json::Value = serde_json::from_str(&payload)?;
        for a in payload["attachments"].as_array().into_iter().flatten() {
            if let Some(name) = a["name"].as_str() {
                taken.push(name.to_string());
            }
        }
    }
    let sql = format!(
        "SELECT a.id, a.run_id, a.name FROM chat_attachments a JOIN runs r ON r.id = a.run_id \
         WHERE a.run_id IN ({marks}) AND a.name IS NOT NULL ORDER BY r.queued_at, r.id, a.id"
    );
    let mut q = sqlx::query_as::<_, (String, String, String)>(&sql);
    for id in moving {
        q = q.bind(id);
    }
    let files = q.fetch_all(&mut *conn).await?;
    if files.is_empty() {
        return Ok(());
    }
    let mut seen = HashSet::new();
    taken.retain(|n| seen.insert(n.to_lowercase()));
    let first = taken.len();
    let mut all = taken;
    all.extend(files.iter().map(|(_, _, name)| name.clone()));
    let unique = crate::watchers::files::unique_names(&all);
    for ((id, run_id, name), new) in files.iter().zip(unique.into_iter().skip(first)) {
        if *name == new {
            continue;
        }
        sqlx::query("UPDATE chat_attachments SET name = ?, placed_at = NULL WHERE id = ?")
            .bind(&new)
            .bind(id)
            .execute(&mut *conn)
            .await?;
        let payload: Option<String> = sqlx::query_scalar("SELECT payload FROM runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&mut *conn)
            .await?;
        let Some(payload) = payload else { continue };
        let mut payload: serde_json::Value = serde_json::from_str(&payload)?;
        if let Some(list) = payload
            .get_mut("attachments")
            .and_then(|a| a.as_array_mut())
        {
            for a in list.iter_mut().filter(|a| a["id"] == id.as_str()) {
                a["name"] = serde_json::Value::String(new.clone());
            }
        }
        sqlx::query("UPDATE runs SET payload = ? WHERE id = ?")
            .bind(payload.to_string())
            .bind(run_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// The ids of `run` and the runs that descend from it in `runs` (by `parent_run_id`), and the
/// runs that resent one of them (`retry_of`: Send again and Retry make a sibling of the
/// message, which belongs with it), with theirs.
fn moved_set(run: &str, runs: &[Run]) -> Vec<String> {
    let mut set: HashSet<&str> = HashSet::from([run]);
    loop {
        let before = set.len();
        for r in runs {
            let follows = |of: &Option<String>| of.as_deref().is_some_and(|p| set.contains(p));
            if follows(&r.parent_run_id) || follows(&r.retry_of) {
                set.insert(&r.id);
            }
        }
        if set.len() == before {
            break;
        }
    }
    runs.iter()
        .filter(|r| set.contains(r.id.as_str()))
        .map(|r| r.id.clone())
        .collect()
}

impl Store {
    /// The rows of the dot's topics (only those that have one).
    pub async fn list_topics(&self, dot_id: &str) -> Result<Vec<TopicInfo>> {
        let rows = sqlx::query(&format!(
            "SELECT {TOPIC_COLUMNS} FROM topics WHERE dot_id = ? ORDER BY root_run_id"
        ))
        .bind(dot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(topic_from_row).collect()
    }

    /// The topic's row; `None` when it has none.
    pub async fn topic_info(&self, root: &str) -> Result<Option<TopicInfo>> {
        let row = sqlx::query(&format!(
            "SELECT {TOPIC_COLUMNS} FROM topics WHERE root_run_id = ?"
        ))
        .bind(root)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(topic_from_row).transpose()
    }

    /// The topic `root` of an existing root run, with its row made if it had none.
    async fn ensure_topic(&self, root: &str) -> Result<()> {
        let run = self.get_run(root).await?;
        if run.root_run_id != run.id {
            return Err(Error::Invalid(format!(
                "run {root} is not the first run of a topic"
            )));
        }
        sqlx::query("INSERT OR IGNORE INTO topics (root_run_id, dot_id) VALUES (?, ?)")
            .bind(root)
            .bind(&run.dot_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Names the topic `root` (trimmed; empty or `None` clears the name).
    pub async fn set_topic_title(&self, root: &str, title: Option<&str>) -> Result<TopicInfo> {
        let title = title.map(str::trim).filter(|t| !t.is_empty());
        if title.is_some_and(|t| t.chars().count() > MAX_TOPIC_TITLE) {
            return Err(Error::Invalid(format!(
                "a topic's name is at most {MAX_TOPIC_TITLE} characters"
            )));
        }
        self.ensure_topic(root).await?;
        sqlx::query("UPDATE topics SET title = ? WHERE root_run_id = ?")
            .bind(title)
            .bind(root)
            .execute(&self.pool)
            .await?;
        Ok(self.topic_info(root).await?.expect("the row was made"))
    }

    /// Archives the topic `root`, or unarchives it (which counts as activity for the sweep).
    pub async fn set_topic_archived(
        &self,
        root: &str,
        archived: bool,
        now: DateTime<Utc>,
    ) -> Result<TopicInfo> {
        self.ensure_topic(root).await?;
        let sql = if archived {
            "UPDATE topics SET archived_at = COALESCE(archived_at, ?) WHERE root_run_id = ?"
        } else {
            "UPDATE topics SET archived_at = NULL, unarchived_at = ? WHERE root_run_id = ?"
        };
        sqlx::query(sql)
            .bind(timestamp(now))
            .bind(root)
            .execute(&self.pool)
            .await?;
        Ok(self.topic_info(root).await?.expect("the row was made"))
    }

    /// Moves the run `run_id` and the runs that descend from it to the topic `to`, or to a new
    /// topic it starts (`None`), in one transaction (topic tools spec section 3). Returns the
    /// root of the topic the run is now in. Both topics are marked to restart their session.
    pub async fn move_runs(&self, run_id: &str, to: Option<&str>) -> Result<String> {
        let mut tx = self.pool.begin().await?;
        // A write first, so the checks below and the moves are one step.
        let source: Option<(String, String)> = sqlx::query_as(
            "UPDATE runs SET root_run_id = root_run_id WHERE id = ? RETURNING root_run_id, dot_id",
        )
        .bind(run_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((source_root, dot_id)) = source else {
            return Err(Error::NotFound(format!("run {run_id}")));
        };
        if source_root == run_id {
            return Err(Error::Invalid(MOVE_ROOT.into()));
        }
        let source = thread(&mut tx, &source_root).await?;
        check_changeable(&mut tx, &source).await?;
        let parent = match to {
            Some(root) => {
                if root == source_root {
                    return Err(Error::Invalid(
                        "the message is in that topic already".into(),
                    ));
                }
                let target = topic(&mut tx, root).await?;
                if target[0].dot_id != dot_id {
                    return Err(Error::Invalid(
                        "a message moves only to a topic of the same sol".into(),
                    ));
                }
                check_changeable(&mut tx, &target).await?;
                target.last().map(|r| r.id.clone())
            }
            None => None,
        };
        let new_root = to.unwrap_or(run_id).to_string();
        let ids = moved_set(run_id, &source);
        if let Some(target) = to {
            rename_clashing_files(&mut tx, &ids, target).await?;
        }
        set_root(&mut tx, &ids, &new_root).await?;
        // The unused one-shot grants were made for the work at the end of the line, where an
        // approved call goes on: they follow it when it moves.
        let tail = source.last().map(|r| r.id.as_str());
        if tail.is_some_and(|t| ids.iter().any(|id| id == t)) {
            sqlx::query("UPDATE grants SET root_run_id = ? WHERE root_run_id = ? AND used = 0")
                .bind(&new_root)
                .bind(&source_root)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE runs SET parent_run_id = ? WHERE id = ?")
            .bind(&parent)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
        mark_restart(&mut tx, &source_root, &dot_id).await?;
        mark_restart(&mut tx, &new_root, &dot_id).await?;
        unarchive(&mut tx, &new_root).await?;
        tx.commit().await?;
        Ok(new_root)
    }

    /// Merges the topic `from` into the topic `into` of the same dot (topic tools spec section
    /// 3): its runs take `into`'s root, its root follows `into`'s last run, and its row goes.
    /// `into` is marked to restart its session.
    pub async fn merge_topics(&self, from: &str, into: &str) -> Result<()> {
        if from == into {
            return Err(Error::Invalid(
                "a topic cannot be merged into itself".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        // A write first, so the checks below and the merge are one step.
        sqlx::query("UPDATE runs SET root_run_id = root_run_id WHERE id = ?")
            .bind(from)
            .execute(&mut *tx)
            .await?;
        let source = topic(&mut tx, from).await?;
        let target = topic(&mut tx, into).await?;
        let dot_id = target[0].dot_id.clone();
        if source[0].dot_id != dot_id {
            return Err(Error::Invalid(
                "a topic merges only into a topic of the same sol".into(),
            ));
        }
        check_changeable(&mut tx, &source).await?;
        check_changeable(&mut tx, &target).await?;
        let last = target.last().map(|r| r.id.clone());
        let ids: Vec<String> = source.iter().map(|r| r.id.clone()).collect();
        rename_clashing_files(&mut tx, &ids, into).await?;
        set_root(&mut tx, &ids, into).await?;
        // A message the person scheduled for the merged topic goes to the one it joined.
        sqlx::query(
            "UPDATE scheduled_messages SET root_run_id = ? \
             WHERE root_run_id = ? AND status IN ('scheduled', 'sending', 'failed')",
        )
        .bind(into)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE runs SET parent_run_id = ? WHERE id = ?")
            .bind(&last)
            .bind(from)
            .execute(&mut *tx)
            .await?;
        // The one-shot grants of the topic's approvals stay with its runs.
        sqlx::query("UPDATE grants SET root_run_id = ? WHERE root_run_id = ?")
            .bind(into)
            .bind(from)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM topics WHERE root_run_id = ?")
            .bind(from)
            .execute(&mut *tx)
            .await?;
        mark_restart(&mut tx, into, &dot_id).await?;
        unarchive(&mut tx, into).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether the topic is marked to start its next run afresh; the mark stays until taken.
    pub async fn topic_restart_pending(&self, root: &str) -> Result<bool> {
        let mark: Option<i64> =
            sqlx::query_scalar("SELECT restart FROM topics WHERE root_run_id = ?")
                .bind(root)
                .fetch_optional(&self.pool)
                .await?;
        Ok(mark == Some(1))
    }

    /// Takes the topic's restart mark: true once after a move or merge changed it.
    pub async fn take_topic_restart(&self, root: &str) -> Result<bool> {
        let res =
            sqlx::query("UPDATE topics SET restart = 0 WHERE root_run_id = ? AND restart = 1")
                .bind(root)
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Archives every topic idle for more than `days` days at `now` (topic tools spec section
    /// 3.3) and returns their rows; `days == 0` archives nothing. A dot's current topic (its
    /// newest chat topic), its most recently active topic, a topic with a run that has not
    /// ended, and one with something waiting (a pending follow-up, an open question, a pending
    /// app or instructions proposal) are left alone.
    pub async fn archive_idle_topics(
        &self,
        now: DateTime<Utc>,
        days: u32,
    ) -> Result<Vec<TopicInfo>> {
        if days == 0 {
            return Ok(Vec::new());
        }
        let cutoff = timestamp(now - TimeDelta::days(i64::from(days)));
        let rows = sqlx::query(
            "SELECT g.root, g.dot_id, g.last_at, g.active, t.archived_at, t.unarchived_at, \
                    root.trigger_kind, root.queued_at AS root_queued, \
                    (SELECT COUNT(*) FROM runs x WHERE x.root_run_id = g.root AND ( \
                       EXISTS (SELECT 1 FROM follow_ups f \
                               WHERE f.from_run_id = x.id AND f.status = 'pending') \
                       OR EXISTS (SELECT 1 FROM messages m WHERE m.run_id = x.id \
                                  AND m.kind = 'question' AND m.answered_at IS NULL \
                                  AND m.dismissed_at IS NULL) \
                       OR EXISTS (SELECT 1 FROM app_proposals p \
                                  WHERE p.run_id = x.id AND p.status = 'pending') \
                       OR EXISTS (SELECT 1 FROM instruction_proposals p \
                                  WHERE p.run_id = x.id AND p.status = 'pending'))) AS waiting \
             FROM (SELECT root_run_id AS root, dot_id, \
                          MAX(MAX(queued_at, COALESCE(ended_at, ''))) AS last_at, \
                          SUM(status IN ('queued', 'running', 'awaiting_approval')) AS active \
                   FROM runs GROUP BY root_run_id) g \
             JOIN runs root ON root.id = g.root \
             LEFT JOIN topics t ON t.root_run_id = g.root",
        )
        .fetch_all(&self.pool)
        .await?;
        struct Seen {
            root: String,
            dot_id: String,
            last: String,
            active: bool,
            archived: bool,
            chat: bool,
            root_queued: String,
        }
        let mut seen = Vec::with_capacity(rows.len());
        for row in &rows {
            let last: String = row.try_get("last_at")?;
            let unarchived: Option<String> = row.try_get("unarchived_at")?;
            seen.push(Seen {
                root: row.try_get("root")?,
                dot_id: row.try_get("dot_id")?,
                last: unarchived.filter(|u| *u > last).unwrap_or(last),
                active: row.try_get::<i64, _>("active")? > 0
                    || row.try_get::<i64, _>("waiting")? > 0,
                archived: row.try_get::<Option<String>, _>("archived_at")?.is_some(),
                chat: row.try_get::<String, _>("trigger_kind")? == "chat",
                root_queued: row.try_get("root_queued")?,
            });
        }
        // Each dot's current topic and its most recently active one.
        let mut kept: HashMap<&str, [Option<&Seen>; 2]> = HashMap::new();
        for s in &seen {
            let slot = kept.entry(&s.dot_id).or_default();
            if s.chat
                && slot[0].is_none_or(|c| (&s.root_queued, &s.root) > (&c.root_queued, &c.root))
            {
                slot[0] = Some(s);
            }
            if slot[1].is_none_or(|c| (&s.last, &s.root) > (&c.last, &c.root)) {
                slot[1] = Some(s);
            }
        }
        let keep: HashSet<&str> = kept
            .values()
            .flat_map(|slot| slot.iter().flatten().map(|s| s.root.as_str()))
            .collect();
        let at = timestamp(now);
        let mut archived = Vec::new();
        for s in &seen {
            if s.archived || s.active || keep.contains(s.root.as_str()) || s.last >= cutoff {
                continue;
            }
            sqlx::query(
                "INSERT INTO topics (root_run_id, dot_id, archived_at) VALUES (?, ?, ?) \
                 ON CONFLICT(root_run_id) DO UPDATE SET archived_at = excluded.archived_at",
            )
            .bind(&s.root)
            .bind(&s.dot_id)
            .bind(&at)
            .execute(&self.pool)
            .await?;
            if let Some(info) = self.topic_info(&s.root).await? {
                archived.push(info);
            }
        }
        Ok(archived)
    }
}

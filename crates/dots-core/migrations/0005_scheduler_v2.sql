-- Scheduler v2 (spec 2026-10-14-scheduler-v2-design.md, section 4).

-- `dots.schedule` held a cron expression; it now holds a schedule as JSON. A converted schedule
-- does not catch up, so that it behaves as before the upgrade (new schedules default to true).
UPDATE dots
SET schedule = json_object(
    'rule', json_object('kind', 'cron', 'expr', schedule),
    'catch_up', json('false'),
    'until', NULL
)
WHERE schedule IS NOT NULL;

-- The failed run that a retry repeats.
ALTER TABLE runs ADD COLUMN retry_of TEXT REFERENCES runs(id) ON DELETE SET NULL;

-- One row per dot with a schedule: where the schedule stands. A deleted run clears the
-- columns that point at it: a watched run that is gone counts as done (spec section 10).
CREATE TABLE schedule_state (
    dot_id           TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE,
    rule_key         TEXT NOT NULL,
    status           TEXT NOT NULL,
    last_slot_at     TEXT NOT NULL,
    fired_count      INTEGER NOT NULL DEFAULT 0,
    watch_run_id     TEXT REFERENCES runs(id) ON DELETE SET NULL,
    retry_slot_at    TEXT,
    retry_attempt    INTEGER,
    retry_at         TEXT,
    attention_run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    updated_at       TEXT NOT NULL
);

-- A resume run is found by its parent, and a retry by the run it repeats (a deleted run sets
-- `retry_of` null in the runs that name it).
CREATE INDEX runs_parent ON runs(parent_run_id);
CREATE INDEX runs_retry_of ON runs(retry_of);

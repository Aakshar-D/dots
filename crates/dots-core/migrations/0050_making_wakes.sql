-- The wakes of makers (dots-make-dots design sections 7.2 and 8): a message Dots has for a
-- maker's chat topic, delivered by one new run of the maker in that topic, exactly once. A wake
-- waits while its topic is busy, and across a restart. `root_run_id` is the maker's chat topic
-- and no foreign key, as in `making_events`. `delivered_at` is NULL while the wake waits;
-- `run_id` is the run that delivered it, written with the run in one transaction, and stays
-- NULL for a wake that was dropped because its topic is gone. Deleting the maker discards its
-- wakes.
CREATE TABLE making_wakes (
    id            TEXT PRIMARY KEY,
    maker_dot_id  TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    root_run_id   TEXT NOT NULL,
    message       TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    run_id        TEXT,
    delivered_at  TEXT
);
CREATE INDEX making_wakes_waiting ON making_wakes(created_at) WHERE delivered_at IS NULL;

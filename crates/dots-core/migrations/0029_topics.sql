-- What a topic has beyond its runs (topic tools spec section 4.1). A topic is still its root
-- run and the runs that share its `root_run_id`; a topic with no row is active and untitled.
-- `unarchived_at` counts as activity for the idle sweep; `restart` marks a topic a move or a
-- merge changed: its next run starts a fresh session.
CREATE TABLE topics (
    root_run_id   TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    title         TEXT,
    archived_at   TEXT,
    unarchived_at TEXT,
    restart       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX topics_dot ON topics(dot_id);

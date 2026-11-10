-- Context meter and compact/clear rules (context rules spec). `context_fill` is the tokens the
-- run's last main-thread turn sent and `context_window` the window its engine reported; NULL
-- when none. `dots.context_rules` is the dot's own rules as JSON, NULL for Settings'.
-- `context_threads` holds a topic's answer to "start fresh?" and the person's request for its
-- next turn. `context_events` records each compaction and fresh start.
ALTER TABLE runs ADD COLUMN context_fill INTEGER;
ALTER TABLE runs ADD COLUMN context_window INTEGER;
ALTER TABLE dots ADD COLUMN context_rules TEXT;
CREATE TABLE context_threads (
    root_run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    keep_going  INTEGER NOT NULL DEFAULT 0,
    request     TEXT CHECK (request IN ('compact', 'fresh'))
);
CREATE TABLE context_events (
    id           TEXT PRIMARY KEY,
    run_id       TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    dot_id       TEXT NOT NULL,
    root_run_id  TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('compact', 'clear')),
    trigger      TEXT NOT NULL CHECK (trigger IN ('auto', 'manual')),
    fill_before  INTEGER NOT NULL,
    fill_after   INTEGER,
    from_session TEXT,
    at           TEXT NOT NULL
);
CREATE INDEX context_events_run ON context_events (run_id);
CREATE INDEX context_events_dot ON context_events (dot_id);

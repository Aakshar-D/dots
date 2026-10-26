-- Watchers (watchers spec, docs/superpowers/specs/2026-10-23-watchers-design.md, section 3):
-- what starts a dot when a folder, a Gmail label or a GitHub repository changes.
CREATE TABLE watchers (
    id TEXT PRIMARY KEY,                    -- ULID
    dot_id TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,                     -- folder | email | github
    name TEXT NOT NULL,                     -- the person's label, 1 to 80 characters
    config TEXT NOT NULL,                   -- JSON: the WatcherConfig, tagged by kind
    enabled INTEGER NOT NULL DEFAULT 1,     -- the person's switch
    paused_reason TEXT,                     -- null | flood | error: paused by Dots
    paused_at TEXT,                         -- when Dots paused it
    last_error TEXT,                        -- the latest failure, shown in the tab
    cursor TEXT,                            -- JSON: where an email or GitHub watcher read up to
    last_event_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX watchers_by_dot ON watchers (dot_id, created_at);

-- The runs a watcher started in the last hour, for the cap of 20 (spec section 5).
CREATE TABLE watcher_fires (
    watcher_id TEXT NOT NULL REFERENCES watchers(id) ON DELETE CASCADE,
    at TEXT NOT NULL
);
CREATE INDEX watcher_fires_by_watcher ON watcher_fires (watcher_id, at);

CREATE TABLE schedule_skips (
    id         TEXT PRIMARY KEY,
    dot_id     TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    slot_at    TEXT NOT NULL,          -- the slot's time, RFC 3339 UTC
    reason     TEXT NOT NULL,          -- 'busy' (the only reason for now)
    run_id     TEXT,                   -- the run that made the dot busy, if known
    created_at TEXT NOT NULL
);
CREATE INDEX schedule_skips_dot ON schedule_skips(dot_id, slot_at);

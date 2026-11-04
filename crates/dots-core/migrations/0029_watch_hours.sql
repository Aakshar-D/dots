ALTER TABLE watchers ADD COLUMN hours TEXT;   -- JSON WatchHours, null = any time
CREATE TABLE watcher_held (
    id         TEXT PRIMARY KEY,
    watcher_id TEXT NOT NULL REFERENCES watchers(id) ON DELETE CASCADE,
    payload    TEXT NOT NULL,           -- JSON: the event's payload as fire() would have used it
    files      TEXT,                    -- JSON: files staged for it (email attachments), if any
    held_at    TEXT NOT NULL            -- RFC 3339 UTC
);
CREATE INDEX watcher_held_watcher ON watcher_held(watcher_id, held_at);

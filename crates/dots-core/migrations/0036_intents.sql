-- The person's goals (Intents). A goal belongs to the account (`org_id`, NULL for Personal)
-- and workspace it was added in, like a dot, so the account's roles apply to it. `due_date` is
-- a calendar date (YYYY-MM-DD) with no time or zone; `done_at` is NULL until it is done. A
-- linked dot that is deleted leaves the goal unlinked.
CREATE TABLE intents (
    id           TEXT PRIMARY KEY,
    org_id       TEXT,
    workspace_id TEXT NOT NULL DEFAULT 'default',
    text         TEXT NOT NULL,
    due_date     TEXT,
    urgent       INTEGER NOT NULL DEFAULT 0,
    dot_id       TEXT REFERENCES dots(id) ON DELETE SET NULL,
    created_at   TEXT NOT NULL,
    done_at      TEXT
);
CREATE INDEX intents_scope ON intents(org_id, workspace_id);
CREATE INDEX intents_dot ON intents(dot_id);

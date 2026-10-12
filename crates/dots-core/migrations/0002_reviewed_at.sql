-- Set when the user marks a finished run reviewed or discards its worktree (spec §12 Inbox).
ALTER TABLE runs ADD COLUMN reviewed_at TEXT;

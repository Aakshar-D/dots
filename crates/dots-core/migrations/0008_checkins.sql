-- Check-ins and follow-ups (spec 2026-10-20-checkins-followups-design.md, sections 7 and 8).

-- What dots tell and ask the person. A note of Dots' own (a skipped follow-up) has no run.
CREATE TABLE messages (
    id            TEXT PRIMARY KEY,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT,
    kind          TEXT NOT NULL,
    text          TEXT NOT NULL,
    choices       TEXT,
    created_at    TEXT NOT NULL,
    read_at       TEXT,
    answered_at   TEXT,
    answer_run_id TEXT,
    dismissed_at  TEXT
);
CREATE INDEX messages_dot ON messages(dot_id, created_at);

-- The runs dots scheduled for themselves. The follow-up loop finds the due ones by status.
CREATE TABLE follow_ups (
    id           TEXT PRIMARY KEY,
    dot_id       TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    from_run_id  TEXT NOT NULL,
    due_at       TEXT NOT NULL,
    note         TEXT NOT NULL,
    status       TEXT NOT NULL,
    fired_run_id TEXT,
    attempts     INTEGER NOT NULL DEFAULT 0,
    retry_at     TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX follow_ups_due ON follow_ups(status, due_at);

-- Every preset allows `Notify`, `Ask` and `FollowUp` (section 4). The policy of each dot whose
-- preset is not Custom gets the missing rules when the store opens (`store/upgrade.rs`,
-- `finish_0008`), after 0007's `Context`. The rows wait here, and the table is dropped once
-- they are done.
CREATE TABLE upgrade_0008 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0008 (dot_id) SELECT id FROM dots;

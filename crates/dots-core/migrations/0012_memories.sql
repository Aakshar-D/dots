-- Memory for each dot (spec 2026-10-23-dot-memory-design.md, section 2): what a dot remembers
-- across runs, one fact per row. `source` is `dot` (saved by a run, whose id `run_id` keeps) or
-- `person` (written in the dot's Memory tab). A deleted dot takes its memories with it.
CREATE TABLE memories (
    id         TEXT PRIMARY KEY,
    dot_id     TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    text       TEXT NOT NULL,
    pinned     INTEGER NOT NULL DEFAULT 0,
    source     TEXT NOT NULL,
    run_id     TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX memories_dot ON memories(dot_id, pinned, created_at);

-- Every preset allows `Memory` (section 3). The policy of each dot whose preset is not Custom
-- gets the rule when the store opens (`store/upgrade.rs`, `finish_0012`), after 0008's
-- check-in tools. The rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0012 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0012 (dot_id) SELECT id FROM dots;

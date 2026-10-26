-- Apps (spec 2026-10-23-app-library-design.md). Their rows are Library items of kind `app`;
-- their code and data live in the data folder.

-- Every call of an app tool that writes, from the person, a dot or Dots (spec section 5): the
-- audit trail.
CREATE TABLE app_calls (
    id       TEXT PRIMARY KEY,
    app_id   TEXT NOT NULL,
    tool     TEXT NOT NULL,
    actor    TEXT NOT NULL,
    run_id   TEXT,
    input    TEXT NOT NULL,
    now      TEXT NOT NULL,
    ids      TEXT NOT NULL,
    result   TEXT,
    error    TEXT,
    changes  INTEGER NOT NULL,
    took_ms  INTEGER NOT NULL,
    at       TEXT NOT NULL
);
CREATE INDEX app_calls_app ON app_calls(app_id, at);

-- An app a dot proposed (section 6.2): the review card, waiting for the person or decided. Its
-- files wait in `<data>/apps/proposed/<id>/` until it is decided. `app_id` and `base_version`
-- name the app and version a change was checked against; both are NULL for a new app.
CREATE TABLE app_proposals (
    id            TEXT PRIMARY KEY,
    dot_id        TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT NOT NULL,
    root_run_id   TEXT NOT NULL,
    app_name      TEXT NOT NULL,
    app_id        TEXT,
    base_version  INTEGER,
    note          TEXT NOT NULL,
    summary       TEXT NOT NULL,
    status        TEXT NOT NULL,
    answer        TEXT,
    answer_run_id TEXT,
    created_at    TEXT NOT NULL,
    decided_at    TEXT
);
CREATE INDEX app_proposals_dot ON app_proposals(dot_id, created_at);
CREATE INDEX app_proposals_root ON app_proposals(root_run_id);
-- One proposal waits per run.
CREATE UNIQUE INDEX app_proposals_pending_run ON app_proposals(run_id) WHERE status = 'pending';

-- A topic an app started (section 7): its first run, the app and the dot, the app's tag, and
-- the last of its runs whose end the app was told of (`on_run_end`).
CREATE TABLE app_topics (
    root_run_id TEXT PRIMARY KEY,
    app_id      TEXT NOT NULL,
    dot_id      TEXT NOT NULL,
    tag         TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    told_run_id TEXT
);
CREATE INDEX app_topics_app ON app_topics(app_id, created_at);

-- Every preset allows `App` and `ProposeApp` (section 6). The policy of each dot whose preset
-- is any, Custom too as 0007 did for `Context`, gets the rules when the store opens
-- (`store/upgrade.rs`, `finish_0016`), after
-- 0015's `Workflow`. The rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0016 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0016 (dot_id) SELECT id FROM dots;

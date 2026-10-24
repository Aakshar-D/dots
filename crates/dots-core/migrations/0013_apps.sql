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

-- Every preset allows `App` and `ProposeApp` (section 6). The policy of each dot whose preset
-- is any, Custom too as 0007 did for `Context`, gets the rules when the store opens
-- (`store/upgrade.rs`, `finish_0013`), after
-- 0012's `Memory`. The rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0013 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0013 (dot_id) SELECT id FROM dots;

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

CREATE TABLE dots (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL UNIQUE,
    instructions       TEXT NOT NULL,
    engine             TEXT NOT NULL,
    model              TEXT NOT NULL,
    endpoint_url       TEXT,
    workdir            TEXT NOT NULL,
    workspace_mode     TEXT NOT NULL,
    schedule           TEXT,
    timezone           TEXT NOT NULL DEFAULT 'local',
    webhook_token      TEXT NOT NULL,
    policy             TEXT NOT NULL,
    mcp_servers        TEXT,
    use_user_settings  INTEGER NOT NULL DEFAULT 0,
    max_turns          INTEGER NOT NULL,
    timeout_secs       INTEGER NOT NULL,
    approval_wait_secs INTEGER NOT NULL,
    enabled            INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

CREATE TABLE runs (
    id             TEXT PRIMARY KEY,
    dot_id         TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    root_run_id    TEXT NOT NULL,
    parent_run_id  TEXT REFERENCES runs(id) ON DELETE SET NULL,
    trigger_kind   TEXT NOT NULL,
    payload        TEXT,
    status         TEXT NOT NULL,
    session_id     TEXT,
    workspace_path TEXT,
    branch         TEXT,
    base_commit    TEXT,
    summary        TEXT,
    error          TEXT,
    tokens_in      INTEGER NOT NULL DEFAULT 0,
    tokens_out     INTEGER NOT NULL DEFAULT 0,
    queued_at      TEXT NOT NULL,
    started_at     TEXT,
    ended_at       TEXT
);
CREATE INDEX runs_dot_status ON runs(dot_id, status);
CREATE INDEX runs_status_queued ON runs(status, queued_at);

CREATE TABLE run_events (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    seq    INTEGER NOT NULL,
    ts     TEXT NOT NULL,
    kind   TEXT NOT NULL,
    data   TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);

CREATE TABLE approvals (
    id         TEXT PRIMARY KEY,
    run_id     TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    tool       TEXT NOT NULL,
    input      TEXT NOT NULL,
    input_hash TEXT NOT NULL,
    status     TEXT NOT NULL,
    note       TEXT,
    parked     INTEGER NOT NULL DEFAULT 0,
    resolved   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    decided_at TEXT
);
CREATE INDEX approvals_run ON approvals(run_id, status);

CREATE TABLE grants (
    id          TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL,
    tool        TEXT NOT NULL,
    input_hash  TEXT NOT NULL,
    used        INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE INDEX grants_lookup ON grants(root_run_id, tool, input_hash, used);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

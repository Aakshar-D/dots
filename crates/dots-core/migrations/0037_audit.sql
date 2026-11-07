-- Audit records (env vault spec §10). `audit_chain` keeps each org's last `seq` and the hash
-- of its last line, so the chain goes on after its queue is empty, and after the org is left
-- and joined again. `audit_outbox` holds the lines not yet sent; a 2xx deletes them.
-- `audit_history` is what the account's History lists on this PC: 30 days, at most 1,000 rows
-- per org; `seq` is NULL for a record of an account that sends none. No table holds a value.
CREATE TABLE audit_chain (
    org_id    TEXT PRIMARY KEY,
    seq       INTEGER NOT NULL,
    last_hash TEXT NOT NULL
);
CREATE TABLE audit_outbox (
    org_id TEXT NOT NULL,
    seq    INTEGER NOT NULL,
    line   TEXT NOT NULL,
    PRIMARY KEY (org_id, seq)
) WITHOUT ROWID;
CREATE TABLE audit_history (
    n        INTEGER PRIMARY KEY AUTOINCREMENT,
    id       TEXT NOT NULL,
    org_id   TEXT NOT NULL,
    seq      INTEGER,
    at       TEXT NOT NULL,
    event    TEXT NOT NULL,
    name     TEXT,
    consumer TEXT,
    dot_name TEXT,
    reason   TEXT,
    state    TEXT NOT NULL CHECK (state IN ('local', 'queued', 'sent', 'dropped'))
);
CREATE INDEX audit_history_org ON audit_history (org_id, n);
CREATE INDEX audit_history_seq ON audit_history (org_id, seq);

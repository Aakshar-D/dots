-- Accounts phase 5 (spec 2026-10-28-accounts-members-design.md §10): the API keys made on this
-- PC. `scope` is 'personal' or an org id and `workspace_id` the workspace the key is limited to,
-- so neither is a foreign key; removing an org or workspace revokes or deletes its rows in code.
-- Only the first 12 characters (`prefix`) and the SHA-256 `hash` of a key are kept, never the key.
CREATE TABLE api_keys (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    name TEXT NOT NULL,
    prefix TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at TEXT
);
CREATE INDEX idx_api_keys_scope_workspace ON api_keys (scope, workspace_id);

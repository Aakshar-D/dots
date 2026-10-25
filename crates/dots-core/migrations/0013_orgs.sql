-- Orgs (spec 2026-10-21-env-vault-design.md, sections 5.2 and 6).

-- Each org joined on this PC, with the settings of its join code and never a secret. `source` is
-- the `SourceSettings` JSON. The source's credentials are in Credential Manager, and the org's
-- entries are held in memory and in its offline copy. `last_issued_at` is the `issued_at` of the
-- last policy accepted, kept here so an older policy is refused even after the offline copy
-- expires (section 6.4). `last_fetch_at` is the last good fetch. Leaving the org deletes the row.
CREATE TABLE orgs (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    policy_key TEXT NOT NULL,
    last_issued_at TEXT,
    last_fetch_at TEXT,
    joined_at TEXT NOT NULL
);

-- The org a dot belongs to, set when it is created and never changed. NULL is Personal. There is
-- no foreign key: a removed org's dots keep its id, and joining it again re-attaches them
-- (section 6.4).
ALTER TABLE dots ADD COLUMN org_id TEXT;

-- Each MCP server's tools per scope (section 8): the list accepted and the one last seen with
-- Personal's values, or with an org's. `scope` is `personal` or an org id. The rows made before
-- this are Personal's. A server's catalog in an org is made the first time the server is
-- checked or run in that org. Deleting the server deletes its catalog in every scope.
CREATE TABLE mcp_catalogs_0013 (
    server_id            TEXT NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    scope                TEXT NOT NULL DEFAULT 'personal',
    accepted_fingerprint TEXT,
    accepted_tools       TEXT,
    seen_fingerprint     TEXT,
    seen_tools           TEXT,
    status               TEXT NOT NULL,
    checked_at           TEXT,
    error                TEXT,
    PRIMARY KEY (server_id, scope)
);
INSERT INTO mcp_catalogs_0013 (server_id, scope, accepted_fingerprint, accepted_tools,
    seen_fingerprint, seen_tools, status, checked_at, error)
SELECT server_id, 'personal', accepted_fingerprint, accepted_tools, seen_fingerprint,
    seen_tools, status, checked_at, error FROM mcp_catalogs;
DROP TABLE mcp_catalogs;
ALTER TABLE mcp_catalogs_0013 RENAME TO mcp_catalogs;

-- The origins each http MCP server that sends variables was confirmed for, per scope (section
-- 8): `{"personal": ["https://api.github.com"], "acme-eng": [...]}`, each origin in ASCII as the
-- HTTP client reaches it. NULL is none. At spawn the server starts only when the origin its URL
-- reaches with the run's values is listed for the dot's scope. Deleting the server deletes them.
ALTER TABLE library_items ADD COLUMN confirmed_origins TEXT;

-- Each MCP server made before this keeps working where it sends its variables now: when the
-- runtime starts, `finish_0013` (`store/upgrade.rs`) records each one's origin, computed with
-- Personal's values, as confirmed in Personal. The rows wait here, and the table is dropped
-- once they are done.
CREATE TABLE upgrade_0013 (
    server_id TEXT PRIMARY KEY REFERENCES library_items(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0013 (server_id) SELECT id FROM library_items WHERE kind = 'mcp_server';

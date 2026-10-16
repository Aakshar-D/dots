-- MCP servers, skills and plugins (spec 2026-10-15-mcp-skills-plugins-design.md, sections 2, 3
-- and 10).

-- The Library. An MCP server of a plugin names the plugin in `owner_id` and goes with it.
CREATE TABLE library_items (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL,
    name       TEXT NOT NULL,
    config     TEXT NOT NULL,
    owner_id   TEXT REFERENCES library_items(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (kind, name)
);
CREATE INDEX library_items_owner ON library_items(owner_id);

-- Each MCP server's tools: the list the person accepted and the one last seen.
CREATE TABLE mcp_catalogs (
    server_id            TEXT PRIMARY KEY REFERENCES library_items(id) ON DELETE CASCADE,
    accepted_fingerprint TEXT,
    accepted_tools       TEXT,
    seen_fingerprint     TEXT,
    seen_tools           TEXT,
    status               TEXT NOT NULL,
    checked_at           TEXT,
    error                TEXT
);

-- The server's accepted fingerprint when an MCP call was asked; NULL for other tools.
ALTER TABLE approvals ADD COLUMN mcp_fingerprint TEXT;

-- The Library items each dot uses, by name.
ALTER TABLE dots ADD COLUMN uses TEXT NOT NULL
    DEFAULT '{"mcp_servers":[],"skills":[],"plugins":[]}';

-- Each dot's old per-dot servers and its policy are converted when the store opens
-- (`store/upgrade.rs`): names, shared items and the rules that name a server need code. The
-- rows wait here, and the table is dropped once they are converted.
CREATE TABLE upgrade_0006 (
    dot_id      TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE,
    mcp_servers TEXT
);
INSERT INTO upgrade_0006 (dot_id, mcp_servers) SELECT id, mcp_servers FROM dots;

ALTER TABLE dots DROP COLUMN mcp_servers;

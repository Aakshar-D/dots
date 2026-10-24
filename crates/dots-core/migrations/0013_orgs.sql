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

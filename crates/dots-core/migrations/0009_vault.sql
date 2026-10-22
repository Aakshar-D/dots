-- The env vault (spec 2026-10-21-env-vault-design.md, section 5).

-- Personal variables. A secret's value is in Credential Manager (`vault::secrets`), never here:
-- its `value` is NULL. Names are unique without regard to case, as Windows environment names.
CREATE TABLE env_entries (
    name       TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    kind       TEXT NOT NULL,
    value      TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (kind IN ('plain', 'secret')),
    CHECK (kind = 'plain' OR value IS NULL)
);

-- What each dot's consumers may resolve (section 7): `{"mcp:<server>": [names], "shell": [...]}`.
ALTER TABLE dots ADD COLUMN variables TEXT NOT NULL DEFAULT '{}';

-- Each dot is granted the variables of the servers it uses when the store opens
-- (`store/upgrade.rs`, `finish_0009`), as ticking them would. The rows wait here, and the table
-- is dropped once they are done.
CREATE TABLE upgrade_0009 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0009 (dot_id) SELECT id FROM dots;

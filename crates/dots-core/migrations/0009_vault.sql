-- The env vault (spec 2026-10-21-env-vault-design.md, section 5).

-- Personal variables. A secret's value is in Credential Manager (`vault::secrets`), never here:
-- its `value` is NULL. Names are unique without regard to case, as Windows environment names.
CREATE TABLE env_entries (
    name       TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    kind       TEXT NOT NULL,
    value      TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

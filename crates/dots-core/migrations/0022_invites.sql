-- Accounts phase 3 (spec 2026-10-28-accounts-members-design.md §7 and §6.5): the invites an
-- admin made, the one unpublished member list kept per account, and the request code this PC
-- is waiting on. Both tables go with their org.
CREATE TABLE account_invites (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL,
    workspaces TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    revoked_at TEXT
);
CREATE INDEX account_invites_org ON account_invites (org_id, created_at);

CREATE TABLE roster_drafts (
    org_id TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    base_seq INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    signed TEXT NOT NULL,
    summary TEXT NOT NULL,
    error TEXT NOT NULL,
    created_at TEXT NOT NULL
);

ALTER TABLE orgs ADD COLUMN pending_request TEXT;

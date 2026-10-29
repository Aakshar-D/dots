-- Accounts phase 4 (spec 2026-10-28-accounts-members-design.md §6.3): the workspaces of each
-- account on this PC, and the one workspace each dot and team is in. `scope` is 'personal' or
-- an org id, so it can't be a foreign key; removing an org deletes its rows in code.
-- `use_rules` is the workspace's `use` rules as JSON, NULL when it has none.
CREATE TABLE workspaces (
    scope TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    use_rules TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (scope, id)
);
INSERT INTO workspaces (scope, id, name, use_rules, created_at)
    VALUES ('personal', 'default', 'Default', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
INSERT INTO workspaces (scope, id, name, use_rules, created_at)
    SELECT id, 'default', 'Default', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now') FROM orgs;
ALTER TABLE dots ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE teams ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';

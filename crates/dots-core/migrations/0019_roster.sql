-- Accounts phase 2 (spec 2026-10-28-accounts-members-design.md §6.4): this PC's place on each
-- account's signed member list. 'legacy' until the account's policy asks for a list.
ALTER TABLE orgs ADD COLUMN membership TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE orgs ADD COLUMN my_member_id TEXT;
ALTER TABLE orgs ADD COLUMN my_role TEXT;
ALTER TABLE orgs ADD COLUMN my_workspaces TEXT;
ALTER TABLE orgs ADD COLUMN last_roster_seq INTEGER;

-- Accounts phase 1 (spec 2026-10-28-accounts-members-design.md §8): a team's account. NULL is
-- Personal. A team whose dots are all in one org takes that org; any other team is Personal.
ALTER TABLE teams ADD COLUMN org_id TEXT;
UPDATE teams SET org_id = (
    SELECT MIN(d.org_id) FROM team_members m JOIN dots d ON d.id = m.dot_id WHERE m.team_id = teams.id
) WHERE (
    SELECT COUNT(DISTINCT IFNULL(d.org_id, '')) FROM team_members m JOIN dots d ON d.id = m.dot_id
    WHERE m.team_id = teams.id
) = 1;

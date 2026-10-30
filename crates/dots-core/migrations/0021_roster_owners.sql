-- Accounts phase 2: the Owners of the last member list accepted, as a hash. Only the root key
-- may sign a list that changes them; an admin-signed list must keep them.
ALTER TABLE orgs ADD COLUMN last_owners_hash TEXT;

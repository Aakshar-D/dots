-- Accounts phase 3 review fix: a draft knows the invite it answers, so that dropping the draft
-- without publishing it makes the invite open again (accounts spec §7 and §6.5).
ALTER TABLE roster_drafts ADD COLUMN invite_id TEXT;

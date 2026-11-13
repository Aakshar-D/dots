-- Dots that make dots (dots-make-dots design sections 3, 5 and 9). `making_on` is "Can make
-- dots", off for every existing dot; `making_per_run` and `making_total` are how many dots one
-- run of it may make and how many made dots it may keep before Dots asks the person. A dot a
-- maker made has `made_in_run`, the maker's run that made it, and `made_purpose`, the maker's one
-- line on what it is for; both are NULL for a dot a person made. `made_by_dot_id` is the maker
-- and becomes NULL when the maker is deleted; `made_by_name` is the maker's name, kept current
-- while the maker exists and as it last was after.
ALTER TABLE dots ADD COLUMN making_on INTEGER NOT NULL DEFAULT 0;
ALTER TABLE dots ADD COLUMN making_per_run INTEGER NOT NULL DEFAULT 3;
ALTER TABLE dots ADD COLUMN making_total INTEGER NOT NULL DEFAULT 10;
ALTER TABLE dots ADD COLUMN made_by_dot_id TEXT REFERENCES dots(id) ON DELETE SET NULL;
ALTER TABLE dots ADD COLUMN made_by_name TEXT;
ALTER TABLE dots ADD COLUMN made_in_run TEXT;
ALTER TABLE dots ADD COLUMN made_purpose TEXT;
CREATE INDEX dots_made_by ON dots(made_by_dot_id);

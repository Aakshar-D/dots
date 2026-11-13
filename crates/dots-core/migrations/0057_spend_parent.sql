-- The chain a made dot's spend follows up to its makers' caps (dots-make-dots design section 6,
-- Spend survives a delete): `spend_parent_dot_id` is the dot whose caps its spend counts toward
-- next. It is the maker when the dot is made; when a dot between them is deleted, the
-- transaction that deletes it moves its children to its own spend parent (NULL if it has none),
-- so a maker cannot free the dots below from its cap by deleting the dot between them.
-- `made_by_dot_id`, who made the dot and may change or delete it, does not move. NULL for a dot a
-- person made. Set NULL by the delete's code before the row goes; `ON DELETE SET NULL` only if a
-- row goes another way.
ALTER TABLE dots ADD COLUMN spend_parent_dot_id TEXT REFERENCES dots(id) ON DELETE SET NULL;
UPDATE dots SET spend_parent_dot_id = made_by_dot_id WHERE made_by_dot_id IS NOT NULL;
CREATE INDEX dots_spend_parent ON dots(spend_parent_dot_id);

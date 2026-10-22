-- What needs me (spec 2026-10-22-needs-you-design.md, section 3): when the person last looked
-- at the dot's conversation, RFC 3339 UTC. Null until they first do: then only what needs them
-- counts, so an upgrade shows no wall of old results.
ALTER TABLE dots ADD COLUMN seen_at TEXT;

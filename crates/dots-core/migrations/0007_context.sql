-- The shared context library (spec 2026-10-19-context-library-design.md, sections 2 and 8).

-- A dot's first prompt starts with the global instructions of Settings unless it is switched off.
ALTER TABLE dots ADD COLUMN use_global_instructions INTEGER NOT NULL DEFAULT 1;

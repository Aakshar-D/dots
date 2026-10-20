-- The shared context library (spec 2026-10-19-context-library-design.md, sections 2 and 8).

-- A dot's first prompt starts with the global instructions of Settings unless it is switched off.
ALTER TABLE dots ADD COLUMN use_global_instructions INTEGER NOT NULL DEFAULT 1;

-- Every preset allows `Context` (section 3). Each dot's policy gets the rule when the store opens
-- (`store/upgrade.rs`, `finish_0007`), after the code part of 0006 has added `Skill`, so the
-- rules end as the presets' do. The rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0007 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0007 (dot_id) SELECT id FROM dots;

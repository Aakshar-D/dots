-- Kinds of memory (spec 2026-11-19-memory-design.md, sections 2 and 12): every memory is
-- `core`, `episodic`, `semantic` or `procedural`, and may name the project's files it is
-- about (`files`, a JSON array of paths relative to the project's folder).
ALTER TABLE memories ADD COLUMN kind TEXT NOT NULL DEFAULT 'semantic';
ALTER TABLE memories ADD COLUMN files TEXT NOT NULL DEFAULT '[]';

-- What was stored before is `semantic`. A pinned memory was in every prompt in full, so it
-- becomes `core`, which still is, while the dot's pinned ones fit what a prompt gives core
-- memories: each at most 160 characters, and newest first while their lines together stay
-- within 600 bytes (a line is the text and 41 bytes around it). The others keep their pin.
UPDATE memories SET kind = 'core' WHERE id IN (
    SELECT id FROM (
        SELECT id, SUM(length(CAST(text AS BLOB)) + 41) OVER (
            PARTITION BY dot_id ORDER BY created_at DESC, id DESC
        ) AS total
        FROM memories WHERE pinned = 1 AND length(text) <= 160
    ) WHERE total <= 600
);

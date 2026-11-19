-- The capture step (spec 2026-11-19-memory-design.md, section 8): after a run ends, Sage may
-- save what it learned. `memory_capture` says whether the run was taken for it: null (not
-- taken), `started` (claimed, in one statement, before anything is written) or `done`. A
-- run is claimed at most once, so a restart or a second end never captures it again. Every
-- run stored before this is null and, having ended long ago, is never taken.
ALTER TABLE runs ADD COLUMN memory_capture TEXT;

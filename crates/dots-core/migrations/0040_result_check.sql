-- The result check of a run's final answer (result check spec): JSON of
-- `result_check::ResultCheck` (`{"state":"checking"|"done"|"skipped", ...}`), NULL when no check
-- applies to the run. What the checker used is added to the run's usage columns and recorded as
-- a `result_check` run event.
ALTER TABLE runs ADD COLUMN result_check TEXT;
-- A dot can turn the check off for itself; existing dots keep it on.
ALTER TABLE dots ADD COLUMN check_results INTEGER NOT NULL DEFAULT 1;

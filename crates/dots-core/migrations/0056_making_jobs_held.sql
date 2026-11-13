-- A job whose run waits for a spending cap or Claude's usage limit tells its maker once that it
-- has not started (dots-make-dots design section 8): `held_told_at` is when that wake was
-- stored, in the same transaction; NULL while it has not been told.
ALTER TABLE making_jobs ADD COLUMN held_told_at TEXT;

-- Every pass that follows the jobs reads the open ones; ended rows are kept.
CREATE INDEX making_jobs_open ON making_jobs(created_at) WHERE status != 'ended';
-- The newest job from a maker's topic to a made dot (`latest_making_job`) and the jobs of a
-- maker's topic (`making_jobs_of_topic`).
CREATE INDEX making_jobs_topic_dot ON making_jobs(maker_root_run_id, dot_id);
-- Deleting a maker deletes its wakes (`ON DELETE CASCADE`).
CREATE INDEX making_wakes_maker ON making_wakes(maker_dot_id);

-- The jobs makers give to the dots they made (dots-make-dots design section 8): one row per
-- job. `maker_run_id` is the maker's run that gave it and `maker_root_run_id` the maker's chat
-- topic, where the wake goes when the job ends; `dot_id` is the made dot, `dot_root_run_id` the
-- made dot's chat topic the job runs in, and `first_run_id` the run the job started. None of the
-- run ids is a foreign key, as in `making_events`. `last_run_id` is the run the job is at now: it
-- moves forward when a run of the same turn continues the one before (a resume after an
-- approval, a message sent again after a restart) and when the person's answer to a question
-- starts a run. `status` is `working`, `waiting_answer` (its last run ended with a question
-- waiting for the person) or `ended`; `ended_how` says how it ended (`succeeded`, `failed`,
-- `cancelled`, `deleted`). `wake_id` is the wake that tells the maker, written in the
-- transaction that ends the job, so a job has one wake and no more. Deleting the maker or the
-- made dot discards the job.
CREATE TABLE making_jobs (
    id                 TEXT PRIMARY KEY,
    maker_dot_id       TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    maker_run_id       TEXT NOT NULL,
    maker_root_run_id  TEXT NOT NULL,
    dot_id             TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    dot_root_run_id    TEXT,
    first_run_id       TEXT,
    last_run_id        TEXT,
    message            TEXT NOT NULL,
    status             TEXT NOT NULL,
    ended_how          TEXT,
    created_at         TEXT NOT NULL,
    ended_at           TEXT,
    wake_id            TEXT
);
CREATE INDEX making_jobs_last_run ON making_jobs(last_run_id);
CREATE INDEX making_jobs_maker_topic ON making_jobs(maker_dot_id, maker_root_run_id);
CREATE INDEX making_jobs_dot ON making_jobs(dot_id, status);

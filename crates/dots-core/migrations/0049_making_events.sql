-- The makers' log (dots-make-dots design sections 10 and 11): what a maker's run did, one row
-- per card of the maker's chat topic. `kind` is `made` for a dot it made; `detail` is JSON per
-- kind (for `made`: {"purpose": ..}). `run_id` is the maker's run and `root_run_id` its chat
-- topic; neither is a foreign key, as in `instruction_proposals`. `dot_id` is the made dot and
-- becomes NULL when it is deleted; `dot_name` keeps its name. The `made` rows of a run are what
-- the run's limit counts, so deleting a made dot does not give the run a place back.
CREATE TABLE making_events (
    id            TEXT PRIMARY KEY,
    maker_dot_id  TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT NOT NULL,
    root_run_id   TEXT NOT NULL,
    kind          TEXT NOT NULL,
    dot_id        TEXT REFERENCES dots(id) ON DELETE SET NULL,
    dot_name      TEXT NOT NULL,
    detail        TEXT NOT NULL DEFAULT '{}',
    created_at    TEXT NOT NULL
);
CREATE INDEX making_events_topic ON making_events(root_run_id, created_at);
CREATE INDEX making_events_run ON making_events(run_id, kind);

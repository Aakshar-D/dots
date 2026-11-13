-- Make requests (dots-make-dots design sections 7.1, 7.2 and 10): a dot a maker asked for that
-- is beyond its reach or over a limit, kept until the person answers. Nothing of it exists
-- before the answer. `run_id` is the maker's run that asked and `root_run_id` its chat topic;
-- neither is a foreign key, as in `making_events`. `kind` is `make`; `dot_id` is for a request
-- to change a made dot and NULL for a make. `spec` is the dot as asked and `without` the same
-- with the extras removed, both JSON; `extras` is a JSON list and `over` the limit that would be
-- passed, JSON or NULL. `create_folder` is the folder of `spec` to make first, when it lies
-- below the maker's and does not exist. `status` is `waiting`, `allowed`, `made_without`,
-- `refused`, `gone` (something it names has gone since, `gone_reason` says what, and only a
-- refusal is left) or `discarded`. `made_dot_id` is the dot an answer made. Deleting the maker
-- discards its requests.
CREATE TABLE make_requests (
    id            TEXT PRIMARY KEY,
    maker_dot_id  TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    run_id        TEXT NOT NULL,
    root_run_id   TEXT NOT NULL,
    kind          TEXT NOT NULL,
    dot_id        TEXT REFERENCES dots(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    purpose       TEXT NOT NULL,
    spec          TEXT NOT NULL,
    without       TEXT NOT NULL,
    extras        TEXT NOT NULL,
    over          TEXT,
    create_folder TEXT,
    first_job     TEXT,
    status        TEXT NOT NULL,
    gone_reason   TEXT,
    created_at    TEXT NOT NULL,
    decided_at    TEXT,
    made_dot_id   TEXT REFERENCES dots(id) ON DELETE SET NULL
);
CREATE INDEX make_requests_maker ON make_requests(maker_dot_id, status);

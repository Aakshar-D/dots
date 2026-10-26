-- Teams and workflows (spec 2026-10-23-teams-workflows-design.md, section 4). A team is a
-- named group of dots, each with a role; its projects are local git repos; its workflows are
-- ordered lists of steps (`definition`, JSON, `workflow::Definition`). A deleted dot leaves
-- every team it was in; a deleted team takes its members, projects and workflows with it.
CREATE TABLE teams (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL COLLATE NOCASE UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE team_members (
    team_id  TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    dot_id   TEXT NOT NULL REFERENCES dots(id) ON DELETE CASCADE,
    role     TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL,
    PRIMARY KEY (team_id, dot_id)
);
CREATE INDEX team_members_dot ON team_members(dot_id);

CREATE TABLE team_projects (
    id             TEXT PRIMARY KEY,
    team_id        TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name           TEXT NOT NULL COLLATE NOCASE,
    repo_path      TEXT NOT NULL,
    default_branch TEXT,
    remote         TEXT NOT NULL DEFAULT 'origin',
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    UNIQUE (team_id, name)
);

CREATE TABLE workflows (
    id         TEXT PRIMARY KEY,
    team_id    TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name       TEXT NOT NULL COLLATE NOCASE,
    definition TEXT NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (team_id, name)
);

-- One execution of a workflow (section 4). `definition` is a copy of the workflow's at start, so
-- editing the workflow never changes a run in flight; `project` a copy of the project's spec
-- (`ProjectSpec`), so a project deleted meanwhile leaves the run its repo. `budget` starts as
-- the definition's and grows with "Add budget". `active_ms` is the time spent not waiting on the
-- person, counted up to `clock_at` (section 9).
CREATE TABLE workflow_runs (
    id            TEXT PRIMARY KEY,
    workflow_id   TEXT REFERENCES workflows(id) ON DELETE SET NULL,
    team_id       TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    definition    TEXT NOT NULL,
    project_id    TEXT,
    project       TEXT NOT NULL,
    task          TEXT NOT NULL,
    trigger_kind  TEXT NOT NULL,
    origin_run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    test          INTEGER NOT NULL DEFAULT 0,
    status        TEXT NOT NULL,
    waiting_on    TEXT,
    current_step  TEXT,
    budget        TEXT NOT NULL,
    send_backs    INTEGER NOT NULL DEFAULT 0,
    active_ms     INTEGER NOT NULL DEFAULT 0,
    clock_at      TEXT,
    error         TEXT,
    started_at    TEXT NOT NULL,
    ended_at      TEXT
);
CREATE INDEX workflow_runs_team ON workflow_runs(team_id, started_at);
CREATE INDEX workflow_runs_status ON workflow_runs(status);

-- One step's execution inside a workflow run; `attempt` grows each time the step runs again.
CREATE TABLE workflow_step_runs (
    id              TEXT PRIMARY KEY,
    workflow_run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_id         TEXT NOT NULL,
    attempt         INTEGER NOT NULL,
    status          TEXT NOT NULL,
    handoff         TEXT,
    decision        TEXT,
    error           TEXT,
    started_at      TEXT NOT NULL,
    ended_at        TEXT
);
CREATE INDEX workflow_step_runs_run ON workflow_step_runs(workflow_run_id, started_at);

-- The dot runs of a step, resume children included (they inherit it from their parent).
-- `tokens_cache_read` is the part of `tokens_in` read from the prompt cache (section 9).
ALTER TABLE runs ADD COLUMN workflow_step_run_id TEXT
    REFERENCES workflow_step_runs(id) ON DELETE SET NULL;
ALTER TABLE runs ADD COLUMN tokens_cache_read INTEGER NOT NULL DEFAULT 0;
CREATE INDEX runs_workflow_step ON runs(workflow_step_run_id);

-- Every preset has a `Workflow` rule (section 7). The policy of each dot whose preset is not
-- Custom gets the rule when the store opens (`store/upgrade.rs`, `finish_0014`), after 0012's
-- `Memory`. The rows wait here, and the table is dropped once they are done.
CREATE TABLE upgrade_0014 (
    dot_id TEXT PRIMARY KEY REFERENCES dots(id) ON DELETE CASCADE
);
INSERT INTO upgrade_0014 (dot_id) SELECT id FROM dots;

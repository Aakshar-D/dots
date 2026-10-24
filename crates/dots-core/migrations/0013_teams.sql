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

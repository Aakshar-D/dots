-- Projects' data (app library spec section 8). Ids are ctx.id() values; times are ctx.now
-- (RFC 3339); a due date is YYYY-MM-DD.
CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  archived_at TEXT
);

CREATE TABLE sections (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects (id),
  name TEXT NOT NULL,
  position INTEGER NOT NULL
);

-- A task with no project is a to-do.
CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  project_id TEXT REFERENCES projects (id),
  section_id TEXT REFERENCES sections (id),
  title TEXT NOT NULL,
  spec TEXT NOT NULL DEFAULT '',
  assignee_kind TEXT CHECK (assignee_kind IN ('person', 'dot')),
  assignee_dot_id TEXT,
  due_on TEXT,
  status TEXT NOT NULL CHECK (status IN ('to_do', 'in_progress', 'in_review', 'done')),
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);

CREATE TABLE subtasks (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks (id),
  title TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL
);

-- Every change to a task or a subtask, every comment and every run of a dot on a task, oldest
-- first by rowid. A change's field is title, spec, status, assignee, due_on, section or
-- subtask:<id>, with the values before and after as JSON; undone_by names the change that undid
-- it. A run row's field is started, not_started or ended.
CREATE TABLE activity (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks (id),
  at TEXT NOT NULL,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('person', 'dot')),
  actor_dot_id TEXT,
  actor_name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('change', 'comment', 'run')),
  field TEXT,
  before_json TEXT,
  after_json TEXT,
  text TEXT,
  run_id TEXT,
  undone_by TEXT
);

CREATE INDEX tasks_by_project ON tasks (project_id, section_id, position);
CREATE INDEX subtasks_by_task ON subtasks (task_id, position);
CREATE INDEX activity_by_task ON activity (task_id);

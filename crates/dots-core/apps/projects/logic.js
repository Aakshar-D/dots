// Projects (app library spec section 8): projects, sections, tasks and to-dos, shared by the
// person and their dots. Every change to a task or a subtask writes an Activity row with the
// values before and after, in the same call. A dot may not mark a task Done.

const ONLY_PERSON_DONE = "Only the person marks a task Done; set In review";
/// The Activity rows get_task gives when it is not told how many.
const RECENT_ACTIVITY = 20;
/// The tasks list_tasks gives when it is not told how many.
const LIST_LIMIT = 100;
/// The most bytes of JSON list_tasks gives a dot: a dot's result is bounded at 30 KB.
const DOT_LIST_BYTES = 28000;
/// The most characters of a spec an Activity row shows, before and after.
const ACTIVITY_SPEC = 200;

const fail = (message) => {
  throw new Error(message);
};
const one = (ctx, sql, params) => ctx.db.query(sql, params)[0] ?? null;
const exec = (ctx, sql, params) => ctx.db.exec(sql, params);
const json = (value) => (value === undefined ? null : JSON.stringify(value));
const parse = (text) => (text === null ? null : JSON.parse(text));

/// The bytes of `text` in UTF-8.
function utf8Bytes(text) {
  let n = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    // Each half of a surrogate pair counts 2: the pair is 4 bytes.
    else if (c >= 0xd800 && c <= 0xdfff) n += 2;
    else n += 3;
  }
  return n;
}

/// A spec as an Activity row shows it: its first characters, followed by "…" when it is longer.
function shortSpec(text) {
  if (typeof text !== "string" || text.length <= ACTIVITY_SPEC) return text;
  // A cut between the halves of a surrogate pair would leave half a character.
  const high = text.charCodeAt(ACTIVITY_SPEC - 1);
  const end = high >= 0xd800 && high <= 0xdbff ? ACTIVITY_SPEC - 1 : ACTIVITY_SPEC;
  return `${text.slice(0, end)}…`;
}

function trimmed(text, what) {
  const t = text.trim();
  if (t === "") fail(`The ${what} cannot be empty`);
  return t;
}

/// Who calls, as an Activity row names them.
function who(ctx) {
  const a = ctx.actor;
  if (a.kind === "dot") return { kind: "dot", dot_id: a.id, name: a.name };
  return { kind: "person", dot_id: null, name: "You" };
}

const isDot = (ctx) => ctx.actor.kind === "dot";

function projectRow(ctx, id) {
  return one(ctx, "SELECT * FROM projects WHERE id = ?", [id]) ?? fail(`There is no project ${id}`);
}

function sectionRow(ctx, id) {
  return one(ctx, "SELECT * FROM sections WHERE id = ?", [id]) ?? fail(`There is no section ${id}`);
}

function taskRow(ctx, id) {
  return one(ctx, "SELECT * FROM tasks WHERE id = ?", [id]) ?? fail(`There is no task ${id}`);
}

function subtaskRow(ctx, id) {
  return one(ctx, "SELECT * FROM subtasks WHERE id = ?", [id]) ?? fail(`There is no subtask ${id}`);
}

/// The next position at the end of a task list: the project's section, or the to-dos.
function nextTaskPosition(ctx, projectId, sectionId) {
  return one(
    ctx,
    "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM tasks WHERE project_id IS ? AND section_id IS ?",
    [projectId, sectionId],
  ).p;
}

/// The dot `ref` names, by id or by name, among those that use Projects.
function dotNamed(ctx, ref) {
  const dots = ctx.dots.list();
  return dots.find((d) => d.id === ref) ?? dots.find((d) => d.name === ref) ?? null;
}

/// The assignee an input names: null, the person, or a dot that uses Projects. "me" is the
/// caller.
function assigneeOf(ctx, ref) {
  if (ref === null) return null;
  if (ref === "person") return { kind: "person" };
  if (ref === "me") return isDot(ctx) ? { kind: "dot", id: ctx.actor.id } : { kind: "person" };
  const dot = dotNamed(ctx, ref);
  if (dot === null) fail(`No dot ${ref} uses Projects; list_dots names those that do`);
  return { kind: "dot", id: dot.id };
}

const DAYS = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// A due date: null or YYYY-MM-DD.
function dueOf(value) {
  if (value === null) return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (m !== null) {
    const [year, month, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
    const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
    const last = month === 2 && leap ? 29 : DAYS[month - 1];
    if (month >= 1 && month <= 12 && day >= 1 && day <= last) return value;
  }
  return fail(`The due date ${value} is not a date: write YYYY-MM-DD`);
}

/// Where a task stands: its project and section, or null for a to-do.
function placeOf(task) {
  if (task.project_id === null) return null;
  return { project_id: task.project_id, section_id: task.section_id };
}

/// The value of a task's field, as a change row holds it (without the names shown with it).
function valueOf(ctx, task, field) {
  switch (field) {
    case "title":
    case "spec":
    case "status":
    case "due_on":
      return task[field];
    case "assignee":
      if (task.assignee_kind === null) return null;
      if (task.assignee_kind === "person") return { kind: "person" };
      return { kind: "dot", id: task.assignee_dot_id };
    case "section":
      return placeOf(task);
    default: {
      const id = field.slice("subtask:".length);
      const s = one(ctx, "SELECT title, done FROM subtasks WHERE id = ? AND task_id = ?", [
        id,
        task.id,
      ]);
      return s === null ? null : { title: s.title, done: s.done === 1 };
    }
  }
}

/// `value` without the names a change row shows with it, so two values compare by what they are.
function bare(value) {
  if (value === null || typeof value !== "object") return value;
  const out = {};
  for (const key of Object.keys(value).sort()) {
    if (key !== "name" && key !== "project" && key !== "section") out[key] = value[key];
  }
  return out;
}

const same = (a, b) => JSON.stringify(bare(a)) === JSON.stringify(bare(b));

/// `value` with the names the Activity shows: a dot's, a project's and a section's.
function named(ctx, field, value) {
  if (value === null) return null;
  if (field === "assignee" && value.kind === "dot") {
    const dot = dotNamed(ctx, value.id);
    const name = dot?.name ?? (isDot(ctx) && ctx.actor.id === value.id ? ctx.actor.name : null);
    return { ...value, name };
  }
  if (field === "section") {
    const project = one(ctx, "SELECT name FROM projects WHERE id = ?", [value.project_id]);
    const section =
      value.section_id === null
        ? null
        : one(ctx, "SELECT name FROM sections WHERE id = ?", [value.section_id]);
    return { ...value, project: project?.name ?? null, section: section?.name ?? null };
  }
  return value;
}

/// Writes `value` into a task's field.
function apply(ctx, task, field, value) {
  const now = ctx.now;
  switch (field) {
    case "title":
    case "spec":
    case "due_on":
      exec(ctx, `UPDATE tasks SET ${field} = ?, updated_at = ? WHERE id = ?`, [value, now, task.id]);
      return;
    case "status":
      exec(ctx, "UPDATE tasks SET status = ?, completed_at = ?, updated_at = ? WHERE id = ?", [
        value,
        value === "done" ? now : null,
        now,
        task.id,
      ]);
      return;
    case "assignee":
      exec(ctx, "UPDATE tasks SET assignee_kind = ?, assignee_dot_id = ?, updated_at = ? WHERE id = ?", [
        value?.kind ?? null,
        value?.kind === "dot" ? value.id : null,
        now,
        task.id,
      ]);
      return;
    case "section": {
      const project = value?.project_id ?? null;
      const section = value?.section_id ?? null;
      exec(
        ctx,
        "UPDATE tasks SET project_id = ?, section_id = ?, position = ?, updated_at = ? WHERE id = ?",
        [project, section, nextTaskPosition(ctx, project, section), now, task.id],
      );
      return;
    }
    default: {
      const id = field.slice("subtask:".length);
      if (value === null) {
        exec(ctx, "DELETE FROM subtasks WHERE id = ?", [id]);
      } else if (one(ctx, "SELECT id FROM subtasks WHERE id = ?", [id]) !== null) {
        exec(ctx, "UPDATE subtasks SET title = ?, done = ? WHERE id = ?", [
          value.title,
          value.done ? 1 : 0,
          id,
        ]);
      } else {
        const position = one(
          ctx,
          "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM subtasks WHERE task_id = ?",
          [task.id],
        ).p;
        exec(ctx, "INSERT INTO subtasks (id, task_id, title, done, position) VALUES (?, ?, ?, ?, ?)", [
          id,
          task.id,
          value.title,
          value.done ? 1 : 0,
          position,
        ]);
      }
      exec(ctx, "UPDATE tasks SET updated_at = ? WHERE id = ?", [now, task.id]);
    }
  }
}

/// Writes an Activity row of the task `taskId` by `actor`; returns its id.
function record(ctx, taskId, actor, row) {
  const id = ctx.id();
  exec(
    ctx,
    "INSERT INTO activity (id, task_id, at, actor_kind, actor_dot_id, actor_name, kind, field, " +
      "before_json, after_json, text, run_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    [
      id,
      taskId,
      ctx.now,
      actor.kind,
      actor.dot_id,
      actor.name,
      row.kind,
      row.field ?? null,
      json(row.before),
      json(row.after),
      row.text ?? null,
      row.run_id ?? null,
    ],
  );
  return id;
}

/// Sets a task's field to `after` as `actor`, with its Activity row. Returns the row's id, or
/// null when the field holds `after` already.
function change(ctx, taskId, field, after, actor) {
  const task = taskRow(ctx, taskId);
  const before = valueOf(ctx, task, field);
  if (same(before, after)) return null;
  const shownBefore = named(ctx, field, before);
  apply(ctx, task, field, after);
  return record(ctx, taskId, actor, {
    kind: "change",
    field,
    before: shownBefore,
    after: named(ctx, field, after),
  });
}

/// What a dot is told when Projects starts it on a task (spec section 8, "Assigning to a dot").
function startMessage(task) {
  return (
    `You are assigned the Projects task "${task.title}" (id ${task.id}). Read it first with the ` +
    "Projects tool get_task: its spec and subtasks say what to do. As you work, post progress " +
    "with comment and check off subtasks with set_subtask. When you have finished, set its " +
    "status to in_review with update_task; only the person marks a task Done."
  );
}

/// Starts the dot a task was just given to, when Projects may start dots: the task goes In
/// progress and its Activity says the dot started, with the run. A dot that takes a task itself
/// is at work already, and a Done task waits. A start the rules refuse is written in Activity,
/// and the task waits for the dot to find it.
function startAssigned(ctx, taskId, actor) {
  const task = taskRow(ctx, taskId);
  if (task.assignee_kind !== "dot" || task.status === "done") return;
  if (isDot(ctx) && ctx.actor.id === task.assignee_dot_id) return;
  if (typeof ctx.dots.start !== "function") return;
  const dot = dotNamed(ctx, task.assignee_dot_id);
  const runner = { kind: "dot", dot_id: task.assignee_dot_id, name: dot?.name ?? "removed dot" };
  let run;
  try {
    run = ctx.dots.start(task.assignee_dot_id, startMessage(task), task.id);
  } catch (e) {
    record(ctx, taskId, runner, { kind: "run", field: "not_started", text: String(e.message ?? e) });
    return;
  }
  // The dot still works on the task from an earlier start: that run is the one.
  const known = one(
    ctx,
    "SELECT id FROM activity WHERE task_id = ? AND kind = 'run' AND field = 'started' AND run_id = ?",
    [taskId, run],
  );
  if (known !== null) return;
  change(ctx, taskId, "status", "in_progress", actor);
  record(ctx, taskId, runner, { kind: "run", field: "started", run_id: run });
}

const FIELD_NAMES = {
  title: "The title",
  spec: "The spec",
  status: "The status",
  assignee: "The assignee",
  due_on: "The due date",
  section: "The section",
};

function assigneeOut(task, dots) {
  if (task.assignee_kind === null) return null;
  if (task.assignee_kind === "person") return { kind: "person" };
  const dot = dots.find((d) => d.id === task.assignee_dot_id);
  return { kind: "dot", id: task.assignee_dot_id, name: dot?.name ?? null };
}

function taskOut(task, dots) {
  return {
    id: task.id,
    title: task.title,
    project_id: task.project_id,
    section_id: task.section_id,
    status: task.status,
    assignee: assigneeOut(task, dots),
    due_on: task.due_on,
    position: task.position,
    created_at: task.created_at,
    updated_at: task.updated_at,
    completed_at: task.completed_at,
  };
}

/// An Activity row as a tool gives it. A spec change shows the start of the spec before and after,
/// so that a task's Activity stays within a dot's result bound; undo reads the row itself.
function activityOut(r) {
  const spec = r.kind === "change" && r.field === "spec";
  const shown = (text) => (spec ? shortSpec(parse(text)) : parse(text));
  return {
    id: r.id,
    at: r.at,
    actor: { kind: r.actor_kind, dot_id: r.actor_dot_id, name: r.actor_name },
    kind: r.kind,
    field: r.field,
    before: shown(r.before_json),
    after: shown(r.after_json),
    text: r.text,
    run_id: r.run_id,
    undone_by: r.undone_by,
  };
}

export function list_projects(_, ctx) {
  const sections = ctx.db.query(
    "SELECT id, project_id, name, position FROM sections ORDER BY position, rowid",
    [],
  );
  return ctx.db
    .query(
      "SELECT id, name, position FROM projects WHERE archived_at IS NULL ORDER BY position, rowid",
      [],
    )
    .map((p) => ({
      ...p,
      sections: sections
        .filter((s) => s.project_id === p.id)
        .map(({ id, name, position }) => ({ id, name, position })),
    }));
}

export function create_project({ name }, ctx) {
  const id = ctx.id();
  const position = one(ctx, "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM projects", []).p;
  const shown = trimmed(name, "project's name");
  exec(ctx, "INSERT INTO projects (id, name, position) VALUES (?, ?, ?)", [id, shown, position]);
  return { id, name: shown, position, sections: [] };
}

export function create_section({ project_id, name }, ctx) {
  projectRow(ctx, project_id);
  const id = ctx.id();
  const position = one(
    ctx,
    "SELECT COALESCE(MAX(position), 0) + 1 AS p FROM sections WHERE project_id = ?",
    [project_id],
  ).p;
  const shown = trimmed(name, "section's name");
  exec(ctx, "INSERT INTO sections (id, project_id, name, position) VALUES (?, ?, ?, ?)", [
    id,
    project_id,
    shown,
    position,
  ]);
  return { id, name: shown, position };
}

export function rename_section({ id, name }, ctx) {
  sectionRow(ctx, id);
  const shown = trimmed(name, "section's name");
  exec(ctx, "UPDATE sections SET name = ? WHERE id = ?", [shown, id]);
  return { id, name: shown };
}

export function list_tasks(input, ctx) {
  const where = ["(t.project_id IS NULL OR p.archived_at IS NULL)"];
  const params = [];
  if (input.project_id !== undefined) {
    where.push("t.project_id = ?");
    params.push(input.project_id);
  }
  if (input.assignee !== undefined) {
    if (input.assignee === "person" || (input.assignee === "me" && !isDot(ctx))) {
      where.push("t.assignee_kind = 'person'");
    } else {
      // A dot that no longer uses Projects is still found by its id.
      const dot = input.assignee === "me" ? { id: ctx.actor.id } : dotNamed(ctx, input.assignee);
      where.push("t.assignee_kind = 'dot' AND t.assignee_dot_id = ?");
      params.push(dot?.id ?? input.assignee);
    }
  }
  if (input.status !== undefined) {
    where.push("t.status = ?");
    params.push(input.status);
  }
  if (input.due_before !== undefined) {
    where.push("t.due_on IS NOT NULL AND t.due_on < ?");
    params.push(dueOf(input.due_before));
  }
  if (input.view === "my_tasks") {
    where.push("(t.assignee_kind = 'person' OR (t.project_id IS NULL AND t.assignee_kind IS NULL))");
  } else if (input.view === "assigned_to_dots") {
    where.push("t.assignee_kind = 'dot'");
  }
  const limit = input.limit ?? LIST_LIMIT;
  const rows = ctx.db.query(
    "SELECT t.*, " +
      "(SELECT COUNT(*) FROM subtasks WHERE task_id = t.id) AS subtasks_total, " +
      "(SELECT COUNT(*) FROM subtasks WHERE task_id = t.id AND done = 1) AS subtasks_done " +
      "FROM tasks t LEFT JOIN projects p ON p.id = t.project_id " +
      "LEFT JOIN sections s ON s.id = t.section_id " +
      `WHERE ${where.join(" AND ")} ` +
      "ORDER BY t.project_id IS NOT NULL, p.position, s.position IS NOT NULL, s.position, " +
      "t.position, t.rowid LIMIT ?",
    [...params, limit + 1],
  );
  const dots = ctx.dots.list();
  const tasks = rows.slice(0, limit).map((t) => ({
    ...taskOut(t, dots),
    subtasks: { done: t.subtasks_done, total: t.subtasks_total },
  }));
  if (!isDot(ctx)) return { tasks, more: rows.length > limit };
  // A dot's result is bounded: its list stops before its JSON would pass DOT_LIST_BYTES, and
  // says there are more. The person's screen is not bounded this way.
  let bytes = utf8Bytes(JSON.stringify({ tasks: [], more: false }));
  let fits = 0;
  for (const t of tasks) {
    bytes += utf8Bytes(JSON.stringify(t)) + (fits > 0 ? 1 : 0);
    if (bytes > DOT_LIST_BYTES) break;
    fits++;
  }
  return { tasks: tasks.slice(0, fits), more: fits < tasks.length || rows.length > limit };
}

export function get_task({ id, activity }, ctx) {
  const task = taskRow(ctx, id);
  const project =
    task.project_id === null
      ? null
      : one(ctx, "SELECT id, name FROM projects WHERE id = ?", [task.project_id]);
  const section =
    task.section_id === null
      ? null
      : one(ctx, "SELECT id, name FROM sections WHERE id = ?", [task.section_id]);
  const subtasks = ctx.db
    .query("SELECT id, title, done FROM subtasks WHERE task_id = ? ORDER BY position, rowid", [id])
    .map((s) => ({ id: s.id, title: s.title, done: s.done === 1 }));
  const rows = ctx.db.query(
    "SELECT * FROM (SELECT rowid AS seq, * FROM activity WHERE task_id = ? ORDER BY rowid DESC " +
      "LIMIT ?) ORDER BY seq",
    [id, activity ?? RECENT_ACTIVITY],
  );
  const total = one(ctx, "SELECT COUNT(*) AS n FROM activity WHERE task_id = ?", [id]).n;
  return {
    ...taskOut(task, ctx.dots.list()),
    spec: task.spec,
    project,
    section,
    subtasks,
    activity: rows.map(activityOut),
    activity_total: total,
  };
}

export function create_task(input, ctx) {
  const actor = who(ctx);
  const title = trimmed(input.title, "task's title");
  let project = input.project_id ?? null;
  const section = input.section_id ?? null;
  if (section !== null) {
    const s = sectionRow(ctx, section);
    if (project !== null && project !== s.project_id) {
      fail(`The section ${section} is not in the project ${project}`);
    }
    project = s.project_id;
  } else if (project !== null) {
    projectRow(ctx, project);
  }
  const status = input.status ?? "to_do";
  if (status === "done" && isDot(ctx)) fail(ONLY_PERSON_DONE);
  const assignee = input.assignee === undefined ? null : assigneeOf(ctx, input.assignee);
  const due = input.due_on === undefined ? null : dueOf(input.due_on);
  const id = ctx.id();
  exec(
    ctx,
    "INSERT INTO tasks (id, project_id, section_id, title, spec, assignee_kind, assignee_dot_id, " +
      "due_on, status, position, created_at, updated_at, completed_at) " +
      "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    [
      id,
      project,
      section,
      title,
      input.spec ?? "",
      assignee?.kind ?? null,
      assignee?.kind === "dot" ? assignee.id : null,
      due,
      status,
      nextTaskPosition(ctx, project, section),
      ctx.now,
      ctx.now,
      status === "done" ? ctx.now : null,
    ],
  );
  record(ctx, id, actor, { kind: "change", field: "created", before: null, after: { title } });
  startAssigned(ctx, id, actor);
  return taskOut(taskRow(ctx, id), ctx.dots.list());
}

export function update_task(input, ctx) {
  const actor = who(ctx);
  const task = taskRow(ctx, input.id);
  const wanted = [];
  if (input.title !== undefined) wanted.push(["title", trimmed(input.title, "task's title")]);
  if (input.spec !== undefined) wanted.push(["spec", input.spec]);
  if (input.status !== undefined) {
    if (input.status === "done" && isDot(ctx)) fail(ONLY_PERSON_DONE);
    wanted.push(["status", input.status]);
  }
  if (input.assignee !== undefined) wanted.push(["assignee", assigneeOf(ctx, input.assignee)]);
  if (input.due_on !== undefined) wanted.push(["due_on", dueOf(input.due_on)]);
  if (input.section_id !== undefined) {
    const place =
      input.section_id === null
        ? { project_id: task.project_id, section_id: null }
        : { project_id: sectionRow(ctx, input.section_id).project_id, section_id: input.section_id };
    wanted.push(["section", place.project_id === null ? null : place]);
  }
  const changes = [];
  for (const [field, value] of wanted) {
    const row = change(ctx, task.id, field, value, actor);
    if (row !== null) changes.push(row);
    if (row !== null && field === "assignee") startAssigned(ctx, task.id, actor);
  }
  return { task: taskOut(taskRow(ctx, task.id), ctx.dots.list()), changes };
}

export function add_subtask({ task_id, title }, ctx) {
  taskRow(ctx, task_id);
  const id = ctx.id();
  const shown = trimmed(title, "subtask's title");
  change(ctx, task_id, `subtask:${id}`, { title: shown, done: false }, who(ctx));
  return { id, title: shown, done: false };
}

export function set_subtask({ id, done, title }, ctx) {
  const s = subtaskRow(ctx, id);
  const after = {
    title: title === undefined ? s.title : trimmed(title, "subtask's title"),
    done: done ?? s.done === 1,
  };
  change(ctx, s.task_id, `subtask:${id}`, after, who(ctx));
  return { id, ...after };
}

export function comment({ task_id, text }, ctx) {
  taskRow(ctx, task_id);
  const id = record(ctx, task_id, who(ctx), { kind: "comment", text: trimmed(text, "comment") });
  return { id };
}

export function list_dots(_, ctx) {
  return ctx.dots.list();
}

export function move_task({ id, section_id, before_id }, ctx) {
  let task = taskRow(ctx, id);
  if (section_id !== undefined) {
    const place =
      section_id === null
        ? placeOf({ project_id: task.project_id, section_id: null })
        : { project_id: sectionRow(ctx, section_id).project_id, section_id };
    change(ctx, id, "section", place, who(ctx));
    task = taskRow(ctx, id);
  }
  // The order within a list is not a change of the task: Activity does not show it.
  const ids = ctx.db
    .query(
      "SELECT id FROM tasks WHERE project_id IS ? AND section_id IS ? AND id != ? " +
        "ORDER BY position, rowid",
      [task.project_id, task.section_id, id],
    )
    .map((r) => r.id);
  const at = before_id === undefined || before_id === null ? ids.length : ids.indexOf(before_id);
  if (at < 0) fail(`The task ${before_id} is not in the list the task moves to`);
  ids.splice(at, 0, id);
  ids.forEach((each, i) =>
    exec(ctx, "UPDATE tasks SET position = ? WHERE id = ? AND position != ?", [i + 1, each, i + 1]),
  );
  return taskOut(taskRow(ctx, id), ctx.dots.list());
}

export function undo({ activity_id }, ctx) {
  const row =
    one(ctx, "SELECT rowid AS seq, * FROM activity WHERE id = ?", [activity_id]) ??
    fail(`There is no Activity row ${activity_id}`);
  if (row.kind !== "change" || row.field === "created") {
    fail("Only a change is undone: not a comment, a run or a task's creation");
  }
  if (row.undone_by !== null) fail("This change was undone already");
  if (isDot(ctx) && row.actor_kind !== "dot") fail("Only the person undoes the person's changes");
  const task = taskRow(ctx, row.task_id);
  const before = parse(row.before_json);
  if (!same(valueOf(ctx, task, row.field), parse(row.after_json))) {
    const what = FIELD_NAMES[row.field] ?? "The subtask";
    const later = one(
      ctx,
      "SELECT actor_name, at FROM activity WHERE task_id = ? AND field = ? AND kind = 'change' " +
        "AND rowid > ? ORDER BY rowid DESC LIMIT 1",
      [row.task_id, row.field, row.seq],
    );
    fail(
      later === null
        ? `${what} no longer holds what this change set`
        : `${what} was changed since by ${later.actor_name} at ${later.at}; undo that change first`,
    );
  }
  if (row.field === "status" && before === "done" && isDot(ctx)) fail(ONLY_PERSON_DONE);
  // An undo puts the value back and starts no dot, even when it gives the task back to one.
  const by = change(ctx, row.task_id, row.field, before, who(ctx));
  exec(ctx, "UPDATE activity SET undone_by = ? WHERE id = ?", [by, row.id]);
  return { undone: row.id, by };
}

/// A run Projects started has ended (spec section 7): its summary or error goes to the task's
/// Activity, and the task goes In review unless it is Done or now someone else's.
export function run_ended(e, ctx) {
  const task = one(ctx, "SELECT * FROM tasks WHERE id = ?", [e.tag]);
  if (task === null) return null;
  const started = one(
    ctx,
    "SELECT actor_name FROM activity WHERE task_id = ? AND field = 'started' AND run_id = ?",
    [task.id, e.root_run_id],
  );
  const name = dotNamed(ctx, e.dot_id)?.name ?? started?.actor_name ?? "removed dot";
  const runner = { kind: "dot", dot_id: e.dot_id, name };
  const text = e.status === "succeeded" ? (e.summary ?? "") : (e.error ?? e.summary ?? "");
  record(ctx, task.id, runner, {
    kind: "run",
    field: "ended",
    after: e.status,
    text,
    run_id: e.run_id,
  });
  if (task.assignee_kind === "dot" && task.assignee_dot_id === e.dot_id && task.status !== "done") {
    change(ctx, task.id, "status", "in_review", runner);
  }
  return null;
}

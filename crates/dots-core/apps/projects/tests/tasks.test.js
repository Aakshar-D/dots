// Projects' tasks: projects and sections, to-dos, statuses and the Done rule, assignees, due
// dates, subtasks, comments, and the Activity row every change writes.

const builder = { kind: "dot", id: "d1", name: "builder", run_id: "r1" };
const dots = [
  { id: "d1", name: "builder" },
  { id: "d2", name: "writer" },
];
const as = (actor) => ({ actor, dots });

function project() {
  const p = call("create_project", { name: "Website" });
  const s = call("create_section", { project_id: p.id, name: "To do" });
  return { p, s };
}

test("a project lists its sections, and its tasks in order", () => {
  const { p, s } = project();
  const later = call("create_section", { project_id: p.id, name: "Later" });
  call("create_task", { title: "Second", section_id: later.id });
  call("create_task", { title: "First", section_id: s.id });
  call("create_task", { title: "Also first", section_id: s.id });
  assert.deepEqual(
    call("list_projects", {}).map((x) => [x.name, x.sections.map((y) => y.name)]),
    [["Website", ["To do", "Later"]]],
  );
  const listed = call("list_tasks", { project_id: p.id });
  assert.deepEqual(
    listed.tasks.map((t) => t.title),
    ["First", "Also first", "Second"],
  );
  assert.equal(listed.more, false);
  assert.equal(listed.tasks[0].project_id, p.id);
});

test("a section is renamed, and a blank name is refused", () => {
  const { s } = project();
  assert.equal(call("rename_section", { id: s.id, name: " Doing " }).name, "Doing");
  assert.throws(() => call("rename_section", { id: s.id, name: "  " }), "cannot be empty");
  assert.throws(() => call("rename_section", { id: "nope", name: "X" }), "There is no section nope");
});

test("a to-do has no project and shows in my tasks with the person's tasks", () => {
  const { s } = project();
  const todo = call("create_task", { title: "Call the landlord" });
  const mine = call("create_task", { title: "Pricing copy", section_id: s.id, assignee: "person" });
  call("create_task", { title: "Signup form", section_id: s.id, assignee: "d1" }, { dots });
  assert.equal(todo.project_id, null);
  assert.deepEqual(
    call("list_tasks", { view: "my_tasks" }).tasks.map((t) => t.id),
    [todo.id, mine.id],
  );
  assert.deepEqual(
    call("list_tasks", { view: "assigned_to_dots" }, { dots }).tasks.map((t) => t.title),
    ["Signup form"],
  );
});

test("each change writes an Activity row with its actor and the values before and after", () => {
  const { s } = project();
  const t = call("create_task", { title: "Signup form", section_id: s.id });
  const out = call("update_task", { id: t.id, status: "in_progress", title: "Signup" }, as(builder));
  assert.equal(out.changes.length, 2);
  assert.equal(out.task.status, "in_progress");
  const rows = call("get_task", { id: t.id }).activity;
  assert.deepEqual(
    rows.map((r) => [r.kind, r.field, r.before, r.after, r.actor.kind, r.actor.name]),
    [
      ["change", "created", null, { title: "Signup form" }, "person", "You"],
      ["change", "title", "Signup form", "Signup", "dot", "builder"],
      ["change", "status", "to_do", "in_progress", "dot", "builder"],
    ],
  );
  assert.equal(rows[1].actor.dot_id, "d1");
  assert.equal(rows[1].at, "2026-01-01T09:00:00+00:00");
});

test("a field set to the value it holds writes no row", () => {
  const t = call("create_task", { title: "Same" });
  assert.equal(call("update_task", { id: t.id, title: "Same", status: "to_do" }).changes.length, 0);
  assert.equal(call("get_task", { id: t.id }).activity.length, 1);
});

test("a dot cannot mark a task Done, and nothing changes", () => {
  const t = call("create_task", { title: "Ship it" });
  assert.throws(
    () => call("update_task", { id: t.id, status: "done", title: "Shipped" }, as(builder)),
    "Only the person marks a task Done; set In review",
  );
  assert.throws(
    () => call("create_task", { title: "Done already", status: "done" }, as(builder)),
    "Only the person marks a task Done",
  );
  const now = call("get_task", { id: t.id });
  assert.equal(now.status, "to_do");
  assert.equal(now.title, "Ship it");
  call("update_task", { id: t.id, status: "in_review" }, as(builder));
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
});

test("the person marks Done, which sets completed_at; leaving Done clears it", () => {
  const t = call("create_task", { title: "Ship it" });
  call("update_task", { id: t.id, status: "done" }, { now: "2026-03-02T10:00:00+01:00" });
  assert.equal(call("get_task", { id: t.id }).completed_at, "2026-03-02T10:00:00+01:00");
  call("update_task", { id: t.id, status: "in_progress" });
  assert.equal(call("get_task", { id: t.id }).completed_at, null);
});

test("assignee: me is the calling dot, a dot is named by id or name, others are refused", () => {
  const t = call("create_task", { title: "Signup form" });
  call("update_task", { id: t.id, assignee: "me" }, as(builder));
  assert.deepEqual(call("get_task", { id: t.id }, { dots }).assignee, {
    kind: "dot",
    id: "d1",
    name: "builder",
  });
  call("update_task", { id: t.id, assignee: "writer" }, { dots });
  assert.equal(call("get_task", { id: t.id }, { dots }).assignee.id, "d2");
  assert.throws(
    () => call("update_task", { id: t.id, assignee: "ghost" }, { dots }),
    "No dot ghost uses Projects",
  );
  call("update_task", { id: t.id, assignee: "me" });
  assert.deepEqual(call("get_task", { id: t.id }).assignee, { kind: "person" });
  const rows = call("get_task", { id: t.id }).activity.filter((r) => r.field === "assignee");
  assert.deepEqual(rows[0].after, { kind: "dot", id: "d1", name: "builder" });
  assert.deepEqual(rows[1].before, { kind: "dot", id: "d1", name: "builder" });
});

test("the assignee of a dot that no longer uses Projects shows without a name", () => {
  const t = call("create_task", { title: "Signup form", assignee: "d1" }, { dots });
  assert.deepEqual(call("get_task", { id: t.id }, { dots: [] }).assignee, {
    kind: "dot",
    id: "d1",
    name: null,
  });
  assert.equal(call("list_tasks", { assignee: "d1" }, { dots: [] }).tasks.length, 1);
});

test("a due date is a real date or null", () => {
  const t = call("create_task", { title: "Rent", due_on: "2028-02-29" });
  assert.equal(t.due_on, "2028-02-29");
  for (const bad of ["2027-02-29", "2026-13-01", "tomorrow", "2026-1-2"]) {
    assert.throws(() => call("update_task", { id: t.id, due_on: bad }), "is not a date");
  }
  call("update_task", { id: t.id, due_on: null });
  assert.equal(call("get_task", { id: t.id }).due_on, null);
});

test("list_tasks filters by assignee, status and due date, and says when there are more", () => {
  const a = call("create_task", { title: "A", assignee: "person", due_on: "2026-10-01" });
  const b = call("create_task", { title: "B", assignee: "d1", due_on: "2026-11-01" }, { dots });
  call("create_task", { title: "C", status: "in_review" });
  assert.deepEqual(call("list_tasks", { assignee: "person" }).tasks.map((t) => t.id), [a.id]);
  assert.deepEqual(call("list_tasks", { assignee: "me" }, as(builder)).tasks.map((t) => t.id), [b.id]);
  assert.deepEqual(call("list_tasks", { assignee: "builder" }, { dots }).tasks.map((t) => t.id), [b.id]);
  assert.deepEqual(call("list_tasks", { status: "in_review" }).tasks.map((t) => t.title), ["C"]);
  assert.deepEqual(call("list_tasks", { due_before: "2026-10-15" }).tasks.map((t) => t.id), [a.id]);
  const limited = call("list_tasks", { limit: 2 });
  assert.equal(limited.tasks.length, 2);
  assert.equal(limited.more, true);
});

test("a task moved to a section of another project changes project", () => {
  const { s } = project();
  const other = call("create_project", { name: "Home" });
  const boxes = call("create_section", { project_id: other.id, name: "Boxes" });
  const t = call("create_task", { title: "Pack", section_id: s.id });
  call("update_task", { id: t.id, section_id: boxes.id });
  const moved = call("get_task", { id: t.id });
  assert.equal(moved.project.name, "Home");
  assert.equal(moved.section.name, "Boxes");
  const row = moved.activity[moved.activity.length - 1];
  assert.equal(row.field, "section");
  assert.equal(row.before.project, "Website");
  assert.equal(row.after.section, "Boxes");
  call("update_task", { id: t.id, section_id: null });
  assert.equal(call("get_task", { id: t.id }).project.name, "Home");
  assert.throws(() => call("create_task", { title: "X", project_id: other.id, section_id: s.id }), "is not in the project");
});

test("subtasks are added, checked and renamed, each with a row", () => {
  const t = call("create_task", { title: "Signup form" });
  const markup = call("add_subtask", { task_id: t.id, title: "Form markup" });
  call("add_subtask", { task_id: t.id, title: "Validation" });
  call("set_subtask", { id: markup.id, done: true }, as(builder));
  call("set_subtask", { id: markup.id, title: "Markup" });
  const got = call("get_task", { id: t.id });
  assert.deepEqual(got.subtasks.map((s) => [s.title, s.done]), [["Markup", true], ["Validation", false]]);
  assert.deepEqual(
    got.activity.slice(1).map((r) => [r.field, r.before, r.after]),
    [
      [`subtask:${markup.id}`, null, { title: "Form markup", done: false }],
      [`subtask:${got.subtasks[1].id}`, null, { title: "Validation", done: false }],
      [`subtask:${markup.id}`, { title: "Form markup", done: false }, { title: "Form markup", done: true }],
      [`subtask:${markup.id}`, { title: "Form markup", done: true }, { title: "Markup", done: true }],
    ],
  );
  assert.equal(call("list_tasks", {}).tasks[0].subtasks.done, 1);
});

test("a comment is an Activity row; get_task gives the last 20 rows, oldest first", () => {
  const t = call("create_task", { title: "Talk" });
  for (let i = 1; i <= 25; i++) call("comment", { task_id: t.id, text: `note ${i}` }, as(builder));
  const got = call("get_task", { id: t.id });
  assert.equal(got.activity.length, 20);
  assert.equal(got.activity_total, 26);
  assert.equal(got.activity[0].text, "note 6");
  assert.equal(got.activity[19].text, "note 25");
  assert.equal(got.activity[19].kind, "comment");
  assert.equal(got.activity[19].actor.name, "builder");
  assert.equal(call("get_task", { id: t.id, activity: 500 }).activity.length, 26);
  assert.throws(() => call("comment", { task_id: t.id, text: " " }), "cannot be empty");
});

test("list_dots gives the dots that use Projects", () => {
  assert.deepEqual(call("list_dots", {}, { dots }), dots);
});

test("a dot's list of many tasks stays within the result bound and says there are more", () => {
  const { s } = project();
  for (let i = 0; i < 150; i++) {
    call("create_task", { title: `Task ${i} `.padEnd(200, "x"), section_id: s.id });
  }
  const listed = call("list_tasks", {}, as(builder));
  const size = JSON.stringify(listed).length;
  assert.ok(size <= 28000, `the dot's list is ${size} characters`);
  assert.ok(listed.tasks.length > 0);
  assert.equal(listed.more, true);
  // The person's calls (the screen) give every row up to the limit.
  const person = call("list_tasks", {});
  assert.equal(person.tasks.length, 100);
  assert.equal(person.more, true);
  const all = call("list_tasks", { limit: 1000 });
  assert.equal(all.tasks.length, 150);
  assert.equal(all.more, false);
});

test("a task's Activity after many spec edits stays within the result bound", () => {
  const first = "Lorem ipsum dolor sit amet. ".repeat(143).slice(0, 4000);
  const t = call("create_task", { title: "Signup form", spec: first });
  const specs = [first];
  for (let i = 1; i <= 10; i++) {
    specs.push(`${first} edit ${i}`);
    call("update_task", { id: t.id, spec: specs[i] });
  }
  const got = call("get_task", { id: t.id }, as(builder));
  const size = JSON.stringify(got).length;
  assert.ok(size <= 30720, `the dot's task is ${size} characters`);
  assert.equal(got.spec, specs[10]);
  const rows = got.activity.filter((r) => r.field === "spec");
  assert.equal(rows.length, 10);
  for (const r of rows) {
    assert.equal(typeof r.before, "string");
    assert.equal(typeof r.after, "string");
    assert.ok(r.before.length <= 201 && r.after.length <= 201, "a spec row is shortened");
  }
  assert.equal(rows[0].before, `${first.slice(0, 200)}…`);
  // Undo reads the database, not this output: the whole previous spec comes back.
  call("undo", { activity_id: rows[9].id });
  assert.equal(call("get_task", { id: t.id }).spec, specs[9]);
});

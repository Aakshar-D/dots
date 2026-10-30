// Projects' undo (app library spec section 8, "Undo"): a change is put back only while its field
// still holds the change's value; the undo is itself a change; the person undoes anyone's
// change, a dot only a dot's. And moving a task between sections and within one.

const builder = { kind: "dot", id: "d1", name: "builder", run_id: "r1" };
const writer = { kind: "dot", id: "d2", name: "writer", run_id: "r2" };
const dots = [
  { id: "d1", name: "builder" },
  { id: "d2", name: "writer" },
];
const as = (actor) => ({ actor, dots });

const last = (task) => {
  const rows = call("get_task", { id: task.id, activity: 500 }).activity;
  return rows[rows.length - 1];
};

test("the person undoes a dot's change, and the undo is a change row of its own", () => {
  const t = call("create_task", { title: "Signup form" });
  call("update_task", { id: t.id, title: "Signup" }, as(builder));
  const change = last(t);
  const out = call("undo", { activity_id: change.id });
  assert.equal(call("get_task", { id: t.id }).title, "Signup form");
  const undo = last(t);
  assert.equal(out.by, undo.id);
  assert.deepEqual(
    [undo.field, undo.before, undo.after, undo.actor.kind],
    ["title", "Signup", "Signup form", "person"],
  );
  const rows = call("get_task", { id: t.id }).activity;
  assert.equal(rows.find((r) => r.id === change.id).undone_by, undo.id);
  assert.throws(() => call("undo", { activity_id: change.id }), "undone already");
});

test("undo refuses once the field changed since, naming the later change", () => {
  const t = call("create_task", { title: "Signup form" });
  call("update_task", { id: t.id, status: "in_progress" }, as(builder));
  const first = last(t);
  call(
    "update_task",
    { id: t.id, status: "in_review" },
    { actor: writer, dots, now: "2026-01-02T10:00:00+00:00" },
  );
  assert.throws(
    () => call("undo", { activity_id: first.id }),
    "The status was changed since by writer at 2026-01-02T10:00:00+00:00; undo that change first",
  );
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
  // Once the later change is undone, the field holds the first change's value again.
  call("undo", { activity_id: last(t).id });
  call("undo", { activity_id: first.id });
  assert.equal(call("get_task", { id: t.id }).status, "to_do");
});

test("the person undoes their own change; a dot undoes a dot's but not the person's", () => {
  const t = call("create_task", { title: "Rent" });
  call("update_task", { id: t.id, due_on: "2026-11-01" });
  const mine = last(t);
  assert.throws(
    () => call("undo", { activity_id: mine.id }, as(builder)),
    "Only the person undoes the person's changes",
  );
  call("update_task", { id: t.id, title: "Pay rent" }, as(writer));
  call("undo", { activity_id: last(t).id }, as(builder));
  assert.equal(call("get_task", { id: t.id }).title, "Rent");
  call("undo", { activity_id: mine.id });
  assert.equal(call("get_task", { id: t.id }).due_on, null);
});

test("comments, runs and a task's creation have no undo", () => {
  const t = call("create_task", { title: "Talk" });
  const created = last(t);
  call("comment", { task_id: t.id, text: "hello" });
  for (const row of [created, last(t)]) {
    assert.throws(() => call("undo", { activity_id: row.id }), "Only a change is undone");
  }
  assert.throws(() => call("undo", { activity_id: "nope" }), "There is no Activity row nope");
});

test("a dot's undo cannot bring back Done", () => {
  const t = call("create_task", { title: "Ship" });
  call("update_task", { id: t.id, status: "done" });
  call("update_task", { id: t.id, status: "in_progress" }, as(builder));
  assert.throws(
    () => call("undo", { activity_id: last(t).id }, as(builder)),
    "Only the person marks a task Done",
  );
  call("undo", { activity_id: last(t).id });
  assert.equal(call("get_task", { id: t.id }).status, "done");
});

test("undoing a subtask's addition removes it, and undoing that brings it back", () => {
  const t = call("create_task", { title: "Signup form" });
  const s = call("add_subtask", { task_id: t.id, title: "Tests" }, as(builder));
  call("undo", { activity_id: last(t).id });
  assert.equal(call("get_task", { id: t.id }).subtasks.length, 0);
  call("undo", { activity_id: last(t).id });
  assert.deepEqual(call("get_task", { id: t.id }).subtasks, [
    { id: s.id, title: "Tests", done: false },
  ]);
});

test("undoing an assignment puts the assignee back and starts no dot", () => {
  const t = call("create_task", { title: "Signup form", assignee: "person" });
  call("update_task", { id: t.id, assignee: "builder" }, { dots });
  call("update_task", { id: t.id, assignee: "person" }, { dots });
  call("undo", { activity_id: last(t).id }, { dots, can_start: true });
  assert.equal(call("get_task", { id: t.id }, { dots }).assignee.name, "builder");
  assert.deepEqual(started(), []);
});

test("a task dragged to another section moves there, before the task it was dropped on", () => {
  const p = call("create_project", { name: "Website" });
  const todo = call("create_section", { project_id: p.id, name: "To do" });
  const done = call("create_section", { project_id: p.id, name: "Done" });
  const a = call("create_task", { title: "A", section_id: todo.id });
  const b = call("create_task", { title: "B", section_id: done.id });
  const c = call("create_task", { title: "C", section_id: done.id });
  call("move_task", { id: a.id, section_id: done.id, before_id: c.id });
  const titles = () => call("list_tasks", { project_id: p.id }).tasks.map((t) => t.title);
  assert.deepEqual(titles(), ["B", "A", "C"]);
  assert.equal(last(a).field, "section");
  assert.equal(last(a).after.section, "Done");
  // Within a section the order changes and Activity does not.
  const rows = call("get_task", { id: c.id }).activity.length;
  call("move_task", { id: c.id, before_id: b.id });
  assert.deepEqual(titles(), ["C", "B", "A"]);
  assert.equal(call("get_task", { id: c.id }).activity.length, rows);
  assert.throws(() => call("move_task", { id: c.id, before_id: "nope" }), "is not in the list");
});

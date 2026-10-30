// Assigning a task to a dot (app library spec section 8): with "Can start dots" on, Projects
// starts the dot on the task, the task goes In progress and Activity says the dot started; when
// the run ends, Activity gets its summary or error and the task goes In review unless it is Done.

const dots = [
  { id: "d1", name: "builder" },
  { id: "d2", name: "writer" },
];
const builder = { kind: "dot", id: "d1", name: "builder", run_id: "r1" };
const on = { dots, can_start: true };
const ended = (e, list = dots) => call("run_ended", e, { actor: { kind: "dots" }, dots: list });
const end = (task, run, status, more) => ({
  run_id: run,
  root_run_id: run,
  tag: task.id,
  dot_id: "d1",
  status,
  summary: null,
  error: null,
  ...more,
});

const rows = (task) => call("get_task", { id: task.id, activity: 500 }, { dots }).activity;

test("assigning a task to a dot starts it on the task, and the task goes In progress", () => {
  const t = call("create_task", { title: "Signup form", spec: "Email and password" });
  call("update_task", { id: t.id, assignee: "builder" }, on);
  const [start] = started();
  assert.equal(start.dot_id, "d1");
  assert.equal(start.tag, t.id);
  assert.ok(start.message.includes(`"Signup form" (id ${t.id})`), start.message);
  assert.ok(start.message.includes("get_task"), start.message);
  assert.ok(start.message.includes("in_review"), start.message);
  assert.equal(call("get_task", { id: t.id }).status, "in_progress");
  const run = rows(t).find((r) => r.kind === "run");
  assert.deepEqual(
    [run.field, run.run_id, run.actor.kind, run.actor.dot_id, run.actor.name],
    ["started", start.run_id, "dot", "d1", "builder"],
  );
});

test("a task created for a dot starts it too", () => {
  const t = call("create_task", { title: "Docs", assignee: "writer" }, on);
  assert.deepEqual(
    started().map((s) => [s.dot_id, s.tag]),
    [["d2", t.id]],
  );
});

test("with the switch off the task waits, and the dot finds it with list_tasks", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, { dots });
  assert.deepEqual(started(), []);
  assert.equal(call("get_task", { id: t.id }).status, "to_do");
  const mine = call("list_tasks", { assignee: "me" }, { actor: builder, dots });
  assert.deepEqual(
    mine.tasks.map((x) => x.id),
    [t.id],
  );
});

test("a dot that takes a task itself, or a Done task, starts nothing", () => {
  const t = call("create_task", { title: "Mine" });
  call("update_task", { id: t.id, assignee: "me" }, { actor: builder, ...on });
  const done = call("create_task", { title: "Old", status: "done" });
  call("update_task", { id: done.id, assignee: "builder" }, on);
  assert.deepEqual(started(), []);
});

test("assigning the same dot again while it works writes no second start", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  call("update_task", { id: t.id, assignee: "person" }, on);
  call("update_task", { id: t.id, assignee: "builder" }, on);
  assert.equal(started().length, 1);
  assert.equal(rows(t).filter((r) => r.field === "started").length, 1);
});

test("a start the rules refuse is written in Activity, and the task waits", () => {
  for (let i = 0; i < 20; i++) call("create_task", { title: `Task ${i}`, assignee: "builder" }, on);
  const t = call("create_task", { title: "One too many", assignee: "builder" }, on);
  assert.equal(started().length, 20);
  assert.equal(call("get_task", { id: t.id }).status, "to_do");
  const row = rows(t).find((r) => r.kind === "run");
  assert.equal(row.field, "not_started");
  assert.ok(row.text.includes("as many dots as the hour allows"), row.text);
});

test("a run's end posts its summary and moves the task In review", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  const [start] = started();
  ended(end(t, start.run_id, "succeeded", { summary: "Form built; tests pass." }));
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
  const [row, review] = rows(t).slice(-2);
  assert.deepEqual(
    [row.kind, row.field, row.after, row.text, row.run_id, row.actor.name],
    ["run", "ended", "succeeded", "Form built; tests pass.", start.run_id, "builder"],
  );
  assert.deepEqual(
    [review.field, review.before, review.after, review.actor.name],
    ["status", "in_progress", "in_review", "builder"],
  );
});

test("a failed run posts its error, and still leaves the task In review", () => {
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  const [start] = started();
  ended({ ...end(t, start.run_id, "failed", { error: "The model stopped" }), run_id: "r9" });
  const row = rows(t).find((r) => r.field === "ended");
  assert.deepEqual([row.after, row.text, row.run_id], ["failed", "The model stopped", "r9"]);
  assert.equal(call("get_task", { id: t.id }).status, "in_review");
});

test("a run's end leaves a Done task Done, and a task given to someone else as it is", () => {
  const done = call("create_task", { title: "A", assignee: "builder" }, on);
  call("update_task", { id: done.id, status: "done" });
  const moved = call("create_task", { title: "B", assignee: "builder" }, on);
  call("update_task", { id: moved.id, assignee: "person" });
  const [first, second] = started();
  ended(end(done, first.run_id, "succeeded", { summary: "Done." }));
  ended(end(moved, second.run_id, "succeeded", { summary: "Done." }));
  assert.equal(call("get_task", { id: done.id }).status, "done");
  assert.equal(call("get_task", { id: moved.id }).status, "in_progress");
  assert.equal(rows(moved).filter((r) => r.field === "ended").length, 1);
});

test("a run whose task is gone is ignored; one whose dot was removed keeps its name", () => {
  const gone = { id: "gone" };
  assert.equal(ended(end(gone, "r1", "succeeded", { summary: "x" })), null);
  const t = call("create_task", { title: "Signup form", assignee: "builder" }, on);
  const [start] = started();
  ended(end(t, start.run_id, "succeeded", { summary: "Done." }), []);
  assert.equal(rows(t).find((r) => r.field === "ended").actor.name, "builder");
});

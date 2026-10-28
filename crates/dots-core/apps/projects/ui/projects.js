// Projects' screen (app library spec section 8, mockup A, list view only): the left column (My
// tasks, Assigned to dots, the projects, + New project) and the list, grouped by section, with
// the columns Task, Assignee, Due and Status. Tasks and sections are added inline, sections
// renamed, and tasks dragged between sections. A task opens in the side pane: its fields, its
// spec in Markdown, its subtasks and its Activity, where a change has Undo and a dot's run opens
// in its conversation. The page talks to Dots only through `dots`, the client Dots puts in it:
// every change is a call of a Projects tool, made as the person, and `dots.onChange` brings the
// screen up to date after any change, a dot's included.

import { markdown } from "./markdown.js";

export const STATUS_LABELS = {
  to_do: "To do",
  in_progress: "In progress",
  in_review: "In review",
  done: "Done",
};
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// The most tasks one list shows.
const LIST_LIMIT = 1000;
/// The most Activity rows the side pane shows.
const PANE_ACTIVITY = 200;
/// The fields whose text and focus a redraw keeps, and the panes whose scroll it keeps.
const FIELDS = "input[aria-label], textarea[aria-label]";
const SCROLLERS = [".list", ".side", ".pane"];
/// True while `draw` replaces the page: a blur caused by the removal of an input does nothing.
let drawing = false;

/// The fields of `root`, each with its key: its accessible name and its place among the fields of
/// that name (two sections can share a name).
function keyed(root) {
  const seen = new Map();
  return [...root.querySelectorAll(FIELDS)].map((el) => {
    const label = el.getAttribute("aria-label");
    const n = seen.get(label) ?? 0;
    seen.set(label, n + 1);
    return [`${label}#${n}`, el];
  });
}

/// An element: `attrs` are attributes, with `on<event>` for listeners and `text` for its text;
/// a null or false attribute or child is left out.
export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "text") el.textContent = value;
    else if (key === "value") {
      // The default too, so that a redraw can tell a draft from the value it was drawn with.
      el.defaultValue = value;
      el.value = value;
    }
    else el.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : String(child));
  }
  return el;
}

/// A due date as the list shows it: "Oct 28".
export function shortDate(day) {
  if (!day) return "";
  const [, month, date] = day.split("-").map(Number);
  return `${MONTHS[month - 1]} ${date}`;
}

/// Who a task is assigned to, as a pill.
export function assigneePill(assignee) {
  if (assignee === null) return "";
  if (assignee.kind === "person") return h("span", { class: "pill" }, "You");
  return h("span", { class: "pill dot" }, `● ${assignee.name ?? "removed dot"}`);
}

/// Who did an Activity row's change: "You" or "● <dot>".
const actorName = (actor) => (actor.kind === "person" ? "You" : `● ${actor.name}`);

/// An Activity row's time: "09:40".
const clock = (at) => at.slice(11, 16);

/// What an Activity row says, after the actor's name.
export function describe(row) {
  const { field, before, after } = row;
  if (row.kind === "comment") return `: ${row.text}`;
  if (row.kind === "run") {
    if (field === "started") return " started";
    if (field === "not_started") return ` could not start: ${row.text}`;
    const said = row.text ? `: ${row.text}` : "";
    if (after === "succeeded") return ` finished${said}`;
    return `'s run ${after === "cancelled" ? "was cancelled" : "failed"}${said}`;
  }
  switch (field) {
    case "created":
      return " created the task";
    case "title":
      return ` renamed it to "${after}"`;
    case "spec":
      return " changed the spec";
    case "status":
      return ` set the status to ${STATUS_LABELS[after]}`;
    case "assignee":
      if (after === null) return " unassigned it";
      return ` assigned it to ${after.kind === "person" ? "you" : (after.name ?? "a removed dot")}`;
    case "due_on":
      return after === null ? " removed the due date" : ` set the due date to ${shortDate(after)}`;
    case "section":
      if (after === null) return " made it a to-do";
      return ` moved it to ${after.project ?? "a project"}${after.section ? ` › ${after.section}` : ""}`;
    default:
      if (before === null) return ` added the subtask "${after.title}"`;
      if (after === null) return ` removed the subtask "${before.title}"`;
      if (before.title !== after.title) {
        return ` renamed the subtask "${before.title}" to "${after.title}"`;
      }
      return ` ${after.done ? "checked" : "unchecked"} "${after.title}"`;
  }
}

/// An input that does `action(text)` on Enter and forgets its text on Escape. It forgets the
/// text before the action runs, so that a second Enter sends nothing.
function entry(attrs, action) {
  return h("input", {
    ...attrs,
    onkeydown: (e) => {
      if (e.key === "Enter" && e.target.value.trim() !== "") {
        e.preventDefault();
        const text = e.target.value.trim();
        e.target.value = "";
        action(text);
      } else if (e.key === "Escape") {
        e.target.value = "";
        e.target.blur();
      }
    },
  });
}

/// Shows Projects in `root`, talking to Dots through `dots`. Returns `ready`, which settles once
/// the first load has drawn, and `reload`.
export function mount(root, dots) {
  const state = {
    view: { kind: "my_tasks" },
    projects: [],
    dots: [],
    tasks: [],
    more: false,
    error: null,
    renaming: null,
    dragging: null,
    // The input to focus again after the next draw (an add row keeps the cursor).
    focus: null,
    // The task open in the side pane: its id, what get_task gave, whether its spec is being
    // edited, and a refusal of a change made there.
    open: null,
    task: null,
    editingSpec: false,
    paneError: null,
  };
  let loads = 0;

  async function load() {
    const mine = ++loads;
    try {
      const [projects, users] = await Promise.all([
        dots.call("list_projects", {}),
        dots.call("list_dots", {}),
      ]);
      if (state.view.kind === "project" && !projects.some((p) => p.id === state.view.id)) {
        state.view = { kind: "my_tasks" };
      }
      const listed = await dots.call("list_tasks", filter(state.view));
      const task = state.open === null ? null : await openTask(state.open);
      if (mine !== loads) return;
      Object.assign(state, { projects, dots: users, tasks: listed.tasks, more: listed.more, task });
      // A task deleted meanwhile closes its pane.
      if (task === null) state.open = null;
    } catch (e) {
      if (mine !== loads) return;
      state.error = String(e?.message ?? e);
    }
    draw();
  }

  /// The task `id` for the side pane, or null when it is gone.
  async function openTask(id) {
    try {
      return await dots.call("get_task", { id, activity: PANE_ACTIVITY });
    } catch (e) {
      if (String(e?.message ?? e).startsWith("There is no task")) return null;
      throw e;
    }
  }

  function select(id) {
    state.open = id;
    state.editingSpec = false;
    state.paneError = null;
    return load();
  }

  /// A change made in the side pane, then a load; a refusal shows in the pane. `done` runs when
  /// the change was accepted, before the load draws.
  async function paneAct(change, done) {
    state.paneError = null;
    try {
      await change();
      done?.();
    } catch (e) {
      state.paneError = String(e?.message ?? e);
    }
    await load();
  }

  /// Runs a change, then loads again; a refusal shows above the list.
  async function act(change) {
    state.error = null;
    try {
      await change();
    } catch (e) {
      state.error = String(e?.message ?? e);
    }
    await load();
  }

  function filter(view) {
    if (view.kind === "project") return { project_id: view.id, limit: LIST_LIMIT };
    return { view: view.kind, limit: LIST_LIMIT };
  }

  function show(view) {
    state.view = view;
    state.renaming = null;
    return load();
  }

  /// What the person has typed (a field whose text differs from the one it was drawn with, by its
  /// accessible name), the field with the focus and its selection, and the scroll offsets.
  function snapshot() {
    const fields = new Map();
    let focus = null;
    for (const [key, el] of keyed(root)) {
      if (el.value !== el.defaultValue) fields.set(key, el.value);
      if (el === document.activeElement) {
        focus = { key, start: el.selectionStart, end: el.selectionEnd };
      }
    }
    const scroll = SCROLLERS.map((sel) => [sel, root.querySelector(sel)?.scrollTop ?? 0]);
    return { fields, focus, scroll };
  }

  /// Puts back what `snapshot` noted, in the page just drawn.
  function restore({ fields, focus, scroll }) {
    const now = keyed(root);
    for (const [key, el] of now) {
      if (fields.has(key)) el.value = fields.get(key);
    }
    // Another view or task opens at the top.
    if (drawnPlace === placeNow()) {
      for (const [sel, top] of scroll) {
        const el = root.querySelector(sel);
        if (el) el.scrollTop = top;
      }
    }
    if (focus === null) return;
    const el = now.find(([key]) => key === focus.key)?.[1];
    if (!el) return;
    el.focus();
    try {
      if (focus.start !== null) el.setSelectionRange(focus.start, focus.end);
    } catch {
      // An input of a kind with no selection (a date).
    }
  }

  /// The view and task the page shows, as text.
  const placeNow = () => JSON.stringify([state.view, state.open]);
  /// Where the page was drawn last: scroll offsets belong to that place only.
  let drawnPlace = null;

  function draw() {
    const before = snapshot();
    drawing = true;
    try {
      root.replaceChildren(
        h("div", { class: "projects" }, side(), list(), state.task && pane(state.task)),
      );
    } finally {
      drawing = false;
    }
    restore(before);
    drawnPlace = placeNow();
    if (state.focus !== null) {
      root.querySelector(`[data-focus="${state.focus}"]`)?.focus();
      state.focus = null;
    }
  }

  function side() {
    const item = (label, view, on) =>
      h("button", { class: on ? "nav on" : "nav", "aria-current": on ? "page" : null, onclick: () => show(view) }, label);
    const v = state.view;
    return h(
      "nav",
      { class: "side", "aria-label": "Projects" },
      item("My tasks", { kind: "my_tasks" }, v.kind === "my_tasks"),
      item("Assigned to dots", { kind: "assigned_to_dots" }, v.kind === "assigned_to_dots"),
      h("h2", { text: "Projects" }),
      state.projects.map((p) =>
        item(`● ${p.name}`, { kind: "project", id: p.id }, v.kind === "project" && v.id === p.id),
      ),
      entry({ class: "add", placeholder: "+ New project", "aria-label": "New project", "data-focus": "project" }, (name) =>
        act(async () => {
          const p = await dots.call("create_project", { name });
          state.view = { kind: "project", id: p.id };
        }),
      ),
    );
  }

  /// The list's groups: a project's sections (and its tasks in no section), or, in My tasks and
  /// Assigned to dots, one group per project with the to-dos first.
  function groups() {
    const v = state.view;
    if (v.kind === "project") {
      const project = state.projects.find((p) => p.id === v.id);
      // A project that is not listed (made, and then the list failed to load; or deleted).
      if (!project) return [];
      const loose = state.tasks.filter((t) => t.section_id === null);
      const out = project.sections.map((s) => ({
        key: s.id,
        section: s,
        tasks: state.tasks.filter((t) => t.section_id === s.id),
        add: { project_id: v.id, section_id: s.id },
      }));
      if (loose.length > 0 || out.length === 0) {
        out.unshift({ key: "none", section: null, tasks: loose, add: { project_id: v.id } });
      }
      return out;
    }
    const todos = state.tasks.filter((t) => t.project_id === null);
    const out = [];
    if (v.kind === "my_tasks" || todos.length > 0) {
      out.push({ key: "todos", title: "To-dos", tasks: todos, add: v.kind === "my_tasks" ? {} : null });
    }
    for (const p of state.projects) {
      const tasks = state.tasks.filter((t) => t.project_id === p.id);
      if (tasks.length > 0) out.push({ key: p.id, title: p.name, tasks, add: null });
    }
    return out;
  }

  function title() {
    const v = state.view;
    if (v.kind === "my_tasks") return "My tasks";
    if (v.kind === "assigned_to_dots") return "Assigned to dots";
    return state.projects.find((p) => p.id === v.id)?.name ?? "";
  }

  function list() {
    const inProject = state.view.kind === "project";
    return h(
      "main",
      { class: "list" },
      h("h1", { text: title() }),
      state.error && h("p", { class: "error", role: "alert", text: state.error }),
      h(
        "div",
        { class: "row head", role: "row" },
        h("span"),
        h("span", { text: "Task" }),
        h("span", { text: "Assignee" }),
        h("span", { text: "Due" }),
        h("span", { text: "Status" }),
      ),
      groups().map(group),
      state.more && h("p", { class: "muted", text: `Showing the first ${LIST_LIMIT} tasks.` }),
      inProject &&
        entry({ class: "add", placeholder: "+ Add section", "aria-label": "New section", "data-focus": "section" }, (name) =>
          act(() => {
            state.focus = "section";
            return dots.call("create_section", { project_id: state.view.id, name });
          }),
        ),
    );
  }

  function heading(g) {
    if (!g.section) {
      return g.title ? h("h3", { text: g.title }) : null;
    }
    if (state.renaming === g.section.id) {
      return h(
        "h3",
        {},
        entry(
          { class: "rename", value: g.section.name, "aria-label": `Rename ${g.section.name}`, "data-focus": "rename", onblur: () => {
              // Removing the input blurs it too: only a person leaving it redraws.
              if (drawing || state.renaming === null) return;
              state.renaming = null;
              draw();
            },
          },
          (name) => {
            state.renaming = null;
            return act(() => dots.call("rename_section", { id: g.section.id, name }));
          },
        ),
      );
    }
    return h(
      "h3",
      {},
      h("button", {
        class: "section",
        title: "Rename the section",
        text: g.section.name,
        onclick: () => {
          state.renaming = g.section.id;
          state.focus = "rename";
          draw();
        },
      }),
    );
  }

  function group(g) {
    const inProject = state.view.kind === "project";
    const sectionId = g.section ? g.section.id : null;
    return h(
      "section",
      {
        class: "group",
        "aria-label": g.section ? g.section.name : (g.title ?? "No section"),
        ondragover: inProject ? (e) => e.preventDefault() : null,
        ondrop: inProject ? (e) => drop(e, sectionId, null) : null,
      },
      heading(g),
      g.tasks.map((t) => row(t, sectionId)),
      g.tasks.length === 0 && !g.add && h("p", { class: "muted", text: "No tasks here yet." }),
      g.add &&
        entry(
          {
            class: "add",
            placeholder: "+ Add task",
            "aria-label": `Add a task to ${g.section ? g.section.name : (g.title ?? title())}`,
            "data-focus": `add-${g.key}`,
          },
          (text) =>
            act(() => {
              state.focus = `add-${g.key}`;
              return dots.call("create_task", { title: text, ...g.add });
            }),
        ),
    );
  }

  /// A task dropped on the section `sectionId`, before the task `beforeId` (or at the end).
  function drop(e, sectionId, beforeId) {
    e.preventDefault();
    e.stopPropagation();
    const id = state.dragging;
    state.dragging = null;
    if (id === null || id === beforeId) return;
    act(() => dots.call("move_task", { id, section_id: sectionId, before_id: beforeId }));
  }

  function row(task, sectionId) {
    const done = task.status === "done";
    const inProject = state.view.kind === "project";
    return h(
      "div",
      {
        class: state.open === task.id ? "row open" : "row",
        role: "row",
        "aria-label": task.title,
        "data-task": task.id,
        draggable: inProject ? "true" : null,
        ondragend: inProject ? () => (state.dragging = null) : null,
        ondragstart: inProject
          ? (e) => {
              state.dragging = task.id;
              e.dataTransfer?.setData("text/plain", task.title);
            }
          : null,
        ondragover: inProject ? (e) => e.preventDefault() : null,
        ondrop: inProject ? (e) => drop(e, sectionId, task.id) : null,
      },
      h("button", {
        class: done ? "check done" : "check",
        "aria-label": done ? `Mark ${task.title} not done` : `Mark ${task.title} done`,
        text: done ? "✓" : "○",
        onclick: () => act(() => dots.call("update_task", { id: task.id, status: done ? "to_do" : "done" })),
      }),
      h("button", { class: "title", text: task.title, onclick: () => select(task.id) }),
      h("span", { class: "cell" }, assigneePill(task.assignee)),
      h("span", { class: "cell", text: shortDate(task.due_on) }),
      h("span", { class: "cell" }, h("span", { class: `pill ${task.status}`, text: STATUS_LABELS[task.status] })),
    );
  }

  /// The side pane of the open task `t`.
  function pane(t) {
    const update = (fields) => paneAct(() => dots.call("update_task", { id: t.id, ...fields }));
    const listed = t.assignee?.kind === "dot" && state.dots.some((d) => d.id === t.assignee.id);
    const assignee = t.assignee === null ? "" : t.assignee.kind === "person" ? "person" : t.assignee.id;
    const pick = (value) => (value === assignee ? true : null);
    return h(
      "aside",
      { class: "pane", "aria-label": "Task" },
      h("button", {
        class: "close",
        "aria-label": "Close the task",
        text: "×",
        onclick: () => {
          // A load in flight must not open the task again.
          loads++;
          state.open = null;
          state.task = null;
          draw();
          load();
        },
      }),
      h("input", {
        class: "pane-title",
        "aria-label": "Title",
        value: t.title,
        onchange: (e) => e.target.value.trim() !== "" && update({ title: e.target.value.trim() }),
      }),
      state.paneError && h("p", { class: "error", role: "alert", text: state.paneError }),
      h(
        "div",
        { class: "fields" },
        h("label", { for: "pane-assignee", text: "Assignee" }),
        h(
          "select",
          { id: "pane-assignee", onchange: (e) => update({ assignee: e.target.value || null }) },
          h("option", { value: "", selected: pick(""), text: "No one" }),
          h("option", { value: "person", selected: pick("person"), text: "You" }),
          state.dots.map((d) => h("option", { value: d.id, selected: pick(d.id), text: `● ${d.name}` })),
          t.assignee?.kind === "dot" &&
            !listed &&
            h("option", {
              value: t.assignee.id,
              selected: true,
              text: `● ${t.assignee.name ?? "removed dot"}`,
            }),
        ),
        h("label", { for: "pane-due", text: "Due" }),
        h("input", {
          id: "pane-due",
          type: "date",
          value: t.due_on ?? "",
          onchange: (e) => update({ due_on: e.target.value || null }),
        }),
        h("label", { for: "pane-status", text: "Status" }),
        h(
          "select",
          { id: "pane-status", onchange: (e) => update({ status: e.target.value }) },
          Object.entries(STATUS_LABELS).map(([value, text]) =>
            h("option", { value, selected: value === t.status ? true : null, text }),
          ),
        ),
      ),
      spec(t),
      h("h4", { text: "Subtasks" }),
      h(
        "ul",
        { class: "subtasks" },
        t.subtasks.map((s) =>
          h(
            "li",
            {},
            h(
              "label",
              {},
              h("input", {
                type: "checkbox",
                checked: s.done ? true : null,
                onchange: (e) =>
                  paneAct(() => dots.call("set_subtask", { id: s.id, done: e.target.checked })),
              }),
              ` ${s.title}`,
            ),
          ),
        ),
      ),
      entry(
        { class: "add", placeholder: "+ Add subtask", "aria-label": "New subtask", "data-focus": "subtask" },
        (title) =>
          paneAct(() => {
            state.focus = "subtask";
            return dots.call("add_subtask", { task_id: t.id, title });
          }),
      ),
      activity(t),
    );
  }

  /// The task's spec: shown as Markdown, or edited as text.
  function spec(t) {
    if (state.editingSpec) {
      const box = h("textarea", { class: "spec-edit", "aria-label": "Spec", rows: 10, value: t.spec });
      return h(
        "div",
        { class: "spec" },
        h("h4", { text: "Spec" }),
        box,
        h(
          "div",
          { class: "buttons" },
          h("button", {
            class: "primary",
            text: "Save",
            // The edit stays, with the person's text, until the update is accepted.
            onclick: () =>
              paneAct(
                () => dots.call("update_task", { id: t.id, spec: box.value }),
                () => (state.editingSpec = false),
              ),
          }),
          h("button", {
            text: "Cancel",
            onclick: () => {
              state.editingSpec = false;
              draw();
            },
          }),
        ),
      );
    }
    const shown = h("div", { class: "markdown" });
    if (t.spec.trim() === "") shown.append(h("p", { class: "muted", text: "No spec yet." }));
    // `markdown` escapes every character of the spec before it adds markup of its own.
    else shown.innerHTML = markdown(t.spec);
    return h(
      "div",
      { class: "spec" },
      h(
        "h4",
        {},
        "Spec ",
        h("button", {
          class: "link-button",
          "aria-label": "Edit the spec",
          text: "Edit",
          onclick: () => {
            state.editingSpec = true;
            draw();
          },
        }),
      ),
      shown,
    );
  }

  /// The task's Activity, newest last, with Undo on each change still in force, a dot's runs
  /// opening in its conversation, and the comment box.
  function activity(t) {
    const box = h("textarea", { "aria-label": "Comment", rows: 2, placeholder: "Comment…" });
    return h(
      "div",
      { class: "activity" },
      h("h4", { text: "Activity" }),
      t.activity_total > t.activity.length &&
        h("p", { class: "muted", text: `The last ${t.activity.length} of ${t.activity_total}.` }),
      h(
        "ol",
        { "aria-label": "Activity" },
        t.activity.map((r) =>
          h(
            "li",
            { class: r.undone_by ? "undone" : null },
            h("span", { class: "who", text: actorName(r.actor) }),
            describe(r),
            h("span", { class: "muted", text: ` · ${clock(r.at)}` }),
            r.undone_by && h("span", { class: "muted", text: " (undone)" }),
            r.kind === "change" &&
              r.field !== "created" &&
              !r.undone_by &&
              h("button", {
                class: "link-button",
                text: "Undo",
                onclick: () => paneAct(() => dots.call("undo", { activity_id: r.id })),
              }),
            r.kind === "run" &&
              r.run_id &&
              h("button", {
                class: "link-button",
                text: "Open the run",
                onclick: () =>
                  Promise.resolve()
                    .then(() => dots.openRun(r.run_id))
                    .catch((e) => {
                      state.paneError = String(e?.message ?? e);
                      draw();
                    }),
              }),
          ),
        ),
      ),
      box,
      h("button", {
        class: "primary",
        text: "Comment",
        onclick: () => {
          const text = box.value.trim();
          if (text === "") return;
          // Forgotten before the call, so that a second click sends nothing.
          box.value = "";
          return paneAct(() => dots.call("comment", { task_id: t.id, text }));
        },
      }),
    );
  }

  dots.onChange(() => {
    load();
  });
  return { ready: load(), reload: load };
}

// Projects' screen (app library spec section 8, mockup A, list view only): the left column (My
// tasks, Assigned to dots, the projects, + New project) and the list, grouped by section, with
// the columns Task, Assignee, Due and Status. Tasks and sections are added inline, sections
// renamed, and tasks dragged between sections. The page talks to Dots only through `dots`, the
// client Dots puts in it: every change is a call of a Projects tool, made as the person, and
// `dots.onChange` brings the screen up to date after any change, a dot's included.

export const STATUS_LABELS = {
  to_do: "To do",
  in_progress: "In progress",
  in_review: "In review",
  done: "Done",
};
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// The most tasks one list shows.
const LIST_LIMIT = 1000;

/// An element: `attrs` are attributes, with `on<event>` for listeners and `text` for its text;
/// a null or false attribute or child is left out.
export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "text") el.textContent = value;
    else if (key === "value") el.value = value;
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

/// An input that does `action(text)` on Enter and forgets its text on Escape.
function entry(attrs, action) {
  return h("input", {
    ...attrs,
    onkeydown: (e) => {
      if (e.key === "Enter" && e.target.value.trim() !== "") {
        e.preventDefault();
        action(e.target.value.trim());
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
      if (mine !== loads) return;
      Object.assign(state, { projects, dots: users, tasks: listed.tasks, more: listed.more });
    } catch (e) {
      if (mine !== loads) return;
      state.error = String(e?.message ?? e);
    }
    draw();
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

  function draw() {
    root.replaceChildren(h("div", { class: "projects" }, side(), list()));
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
          { class: "rename", value: g.section.name, "aria-label": `Rename ${g.section.name}`, "data-focus": "rename", onblur: () => { state.renaming = null; draw(); } },
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
        class: "row",
        role: "row",
        "aria-label": task.title,
        "data-task": task.id,
        draggable: inProject ? "true" : null,
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
      h("span", { class: "title", text: task.title }),
      h("span", { class: "cell" }, assigneePill(task.assignee)),
      h("span", { class: "cell", text: shortDate(task.due_on) }),
      h("span", { class: "cell" }, h("span", { class: `pill ${task.status}`, text: STATUS_LABELS[task.status] })),
    );
  }

  dots.onChange(() => {
    load();
  });
  return { ready: load(), reload: load };
}

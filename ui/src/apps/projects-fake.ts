// A fake `dots` for Projects' screen in jsdom, and the sample's data: what the screen's tests
// call its tools against.
import userEvent from "@testing-library/user-event";
import { afterEach } from "vitest";

import { mount } from "../../../crates/dots-core/apps/projects/ui/projects.js";

export type Input = Record<string, unknown>;
export type Handler = (input: Input) => unknown;

/// A `dots` client whose calls `handlers` answer, with every call kept.
export function fakeDots(handlers: Record<string, Handler>) {
  const changes = new Set<() => void>();
  const calls: [string, Input][] = [];
  const opened: string[] = [];
  const dots = {
    call: async (tool: string, input: Input) => {
      calls.push([tool, input]);
      const handler = handlers[tool];
      if (!handler) throw new Error(`the fake has no ${tool}`);
      return handler(input);
    },
    onChange: (cb: () => void) => {
      changes.add(cb);
      return () => changes.delete(cb);
    },
    onTheme: () => () => {},
    openRun: async (runId: string) => {
      opened.push(runId);
    },
  };
  return {
    dots,
    opened,
    callsTo: (tool: string) => calls.filter(([t]) => t === tool).map(([, input]) => input),
    change: () => changes.forEach((cb) => cb()),
  };
}

export const task = (over: Input) => ({
  id: "t1",
  title: "Write pricing page copy",
  project_id: "p1",
  section_id: "s1",
  status: "to_do",
  assignee: { kind: "person" },
  due_on: "2026-10-28",
  position: 1,
  created_at: "2026-10-20T09:00:00+00:00",
  updated_at: "2026-10-20T09:00:00+00:00",
  completed_at: null,
  subtasks: { done: 0, total: 0 },
  ...over,
});

export const website = {
  id: "p1",
  name: "Website relaunch",
  position: 1,
  sections: [
    { id: "s1", name: "To do", position: 1 },
    { id: "s2", name: "Done", position: 2 },
  ],
};
export const copy = task({});
export const signup = task({
  id: "t2",
  title: "Build signup form",
  assignee: { kind: "dot", id: "d1", name: "builder" },
  due_on: "2026-10-25",
  status: "in_progress",
  position: 2,
});
export const hosting = task({ id: "t3", title: "Pick hosting", section_id: "s2", status: "done" });
export const landlord = task({
  id: "t4",
  title: "Call the landlord",
  project_id: null,
  section_id: null,
  assignee: null,
  due_on: null,
});

/// Projects with the sample's data, as the handlers' defaults.
export function sample(more: Record<string, Handler> = {}) {
  const projects = [website];
  return fakeDots({
    list_projects: () => projects,
    list_dots: () => [{ id: "d1", name: "builder" }],
    list_tasks: (input) => {
      if (input.project_id === "p1") return { tasks: [copy, signup, hosting], more: false };
      if (input.view === "my_tasks") return { tasks: [landlord, copy, hosting], more: false };
      if (input.view === "assigned_to_dots") return { tasks: [signup], more: false };
      return { tasks: [], more: false };
    },
    create_project: (input) => {
      const p = { id: "p2", name: String(input.name), position: 2, sections: [] };
      projects.push(p);
      return p;
    },
    create_task: () => task({ id: "t9" }),
    create_section: () => ({ id: "s9", name: "Launch", position: 3 }),
    rename_section: (input) => input,
    move_task: () => copy,
    update_task: () => ({ task: copy, changes: [] }),
    ...more,
  });
}

let root: HTMLElement | null = null;

export async function show(fake: ReturnType<typeof fakeDots>) {
  root = document.createElement("div");
  document.body.append(root);
  await mount(root, fake.dots).ready;
  return userEvent.setup();
}

afterEach(() => {
  root?.remove();
  root = null;
});

// Projects' side pane (app library spec section 8): a task's fields, its spec in Markdown, its
// subtasks and its Activity, with Undo, a dot's runs and the comment box.
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { type Input, sample, show, signup } from "./projects-fake";

const row = (over: Input) => ({
  id: "a1",
  at: "2026-10-21T09:12:00+00:00",
  actor: { kind: "person", dot_id: null, name: "You" },
  kind: "change",
  field: null,
  before: null,
  after: null,
  text: null,
  run_id: null,
  undone_by: null,
  ...over,
});
const builder = { kind: "dot", dot_id: "d1", name: "builder" };

const detail = (over: Input = {}) => ({
  ...signup,
  spec: "## Form\nEmail and **password**.\n\n<img src=x onerror=alert(1)>",
  project: { id: "p1", name: "Website relaunch" },
  section: { id: "s1", name: "To do" },
  subtasks: [
    { id: "u1", title: "Form markup", done: true },
    { id: "u2", title: "Validation", done: false },
  ],
  activity: [
    row({ field: "created", after: { title: "Build signup form" } }),
    row({ id: "a2", field: "assignee", after: { kind: "dot", id: "d1", name: "builder" } }),
    row({ id: "a3", actor: builder, kind: "run", field: "started", run_id: "r1" }),
    row({
      id: "a4",
      actor: builder,
      field: "subtask:u1",
      before: { title: "Form markup", done: false },
      after: { title: "Form markup", done: true },
      at: "2026-10-21T09:40:00+00:00",
    }),
    row({ id: "a5", actor: builder, kind: "comment", text: "Markup done." }),
    row({ id: "a6", actor: builder, field: "title", before: "Signup", after: "Build signup form", undone_by: "a7" }),
    row({ id: "a8", actor: builder, kind: "run", field: "ended", after: "succeeded", text: "Form built.", run_id: "r1" }),
  ],
  activity_total: 7,
  ...over,
});

async function opened(more: Parameters<typeof sample>[0] = {}) {
  const fake = sample({ get_task: () => detail(), ...more });
  const user = await show(fake);
  await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
  await user.click(screen.getByRole("button", { name: "Build signup form" }));
  const pane = await screen.findByRole("complementary", { name: "Task" });
  return { fake, user, pane };
}

const item = (pane: HTMLElement, text: RegExp) =>
  within(within(pane).getByRole("list", { name: "Activity" }))
    .getAllByRole("listitem")
    .find((li) => text.test(li.textContent ?? ""))!;

describe("Projects' side pane", () => {
  it("shows the task's fields, and its spec as Markdown with its HTML as text", async () => {
    const { fake, pane } = await opened();
    expect(fake.callsTo("get_task")).toEqual([{ id: "t2", activity: 200 }]);
    expect(within(pane).getByRole("textbox", { name: "Title" })).toHaveValue("Build signup form");
    expect(within(pane).getByRole("combobox", { name: "Assignee" })).toHaveValue("d1");
    expect(within(pane).getByLabelText("Due")).toHaveValue("2026-10-25");
    expect(within(pane).getByRole("combobox", { name: "Status" })).toHaveValue("in_progress");
    expect(within(pane).getByRole("heading", { name: "Form" })).toBeVisible();
    expect(pane.querySelector("strong")).toHaveTextContent("password");
    expect(pane.querySelector(".markdown img")).toBeNull();
    expect(pane).toHaveTextContent("<img src=x onerror=alert(1)>");
    expect(screen.getByRole("row", { name: "Build signup form" })).toHaveClass("open");
  });

  it("changes the title, the assignee, the due date and the status", async () => {
    const { fake, user, pane } = await opened();
    const title = within(pane).getByRole("textbox", { name: "Title" });
    await user.clear(title);
    await user.type(title, "Signup{Tab}");
    await user.selectOptions(within(pane).getByRole("combobox", { name: "Assignee" }), "person");
    await user.selectOptions(within(pane).getByRole("combobox", { name: "Status" }), "in_review");
    await waitFor(() =>
      expect(fake.callsTo("update_task")).toEqual([
        { id: "t2", title: "Signup" },
        { id: "t2", assignee: "person" },
        { id: "t2", status: "in_review" },
      ]),
    );
    await user.clear(within(screen.getByRole("complementary", { name: "Task" })).getByLabelText("Due"));
    await waitFor(() =>
      expect(fake.callsTo("update_task").at(-1)).toEqual({ id: "t2", due_on: null }),
    );
  });

  it("edits the spec as text and saves it", async () => {
    const { fake, user, pane } = await opened();
    await user.click(within(pane).getByRole("button", { name: "Edit the spec" }));
    // Each change draws the pane again: find it anew.
    const box = screen.getByRole("textbox", { name: "Spec" });
    await user.clear(box);
    await user.type(box, "Just email");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(fake.callsTo("update_task")).toEqual([{ id: "t2", spec: "Just email" }]),
    );
  });

  it("checks a subtask and adds one", async () => {
    const { fake, user, pane } = await opened({ set_subtask: () => ({}), add_subtask: () => ({}) });
    expect(within(pane).getByRole("checkbox", { name: "Form markup" })).toBeChecked();
    await user.click(within(pane).getByRole("checkbox", { name: "Validation" }));
    await waitFor(() =>
      expect(fake.callsTo("set_subtask")).toEqual([{ id: "u2", done: true }]),
    );
    await user.type(
      within(screen.getByRole("complementary", { name: "Task" })).getByRole("textbox", {
        name: "New subtask",
      }),
      "Tests{Enter}",
    );
    await waitFor(() =>
      expect(fake.callsTo("add_subtask")).toEqual([{ task_id: "t2", title: "Tests" }]),
    );
  });

  it("shows the Activity newest last, with Undo on each change still in force", async () => {
    let refuse = false;
    const { fake, user, pane } = await opened({
      undo: () => {
        if (refuse) throw new Error("The subtask was changed since by You; undo that change first");
        return { undone: "a4", by: "a9" };
      },
    });
    const texts = within(within(pane).getByRole("list", { name: "Activity" }))
      .getAllByRole("listitem")
      .map((li) => li.textContent);
    expect(texts).toEqual([
      "You created the task · 09:12",
      "You assigned it to builder · 09:12Undo",
      "● builder started · 09:12Open the run",
      '● builder checked "Form markup" · 09:40Undo',
      "● builder: Markup done. · 09:12",
      '● builder renamed it to "Build signup form" · 09:12 (undone)',
      "● builder finished: Form built. · 09:12Open the run",
    ]);
    await user.click(within(item(pane, /checked/)).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.callsTo("undo")).toEqual([{ activity_id: "a4" }]));
    refuse = true;
    const again = screen.getByRole("complementary", { name: "Task" });
    await user.click(within(item(again, /checked/)).getByRole("button", { name: "Undo" }));
    expect(await within(screen.getByRole("complementary", { name: "Task" })).findByRole("alert"))
      .toHaveTextContent("The subtask was changed since by You; undo that change first");
  });

  it("opens a dot's run in its conversation, and says so when it cannot", async () => {
    const { fake, user, pane } = await opened();
    await user.click(within(item(pane, /started/)).getByRole("button", { name: "Open the run" }));
    expect(fake.opened).toEqual(["r1"]);
    fake.dots.openRun = async () => {
      throw new Error("the app projects started no run r1");
    };
    await user.click(within(item(pane, /finished/)).getByRole("button", { name: "Open the run" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("started no run r1");
  });

  it("posts a comment", async () => {
    const { fake, user, pane } = await opened({ comment: () => ({ id: "a9" }) });
    await user.type(within(pane).getByRole("textbox", { name: "Comment" }), "Looks good");
    await user.click(within(pane).getByRole("button", { name: "Comment" }));
    await waitFor(() =>
      expect(fake.callsTo("comment")).toEqual([{ task_id: "t2", text: "Looks good" }]),
    );
  });

  it("closes, and closes by itself when the task is gone", async () => {
    let gone = false;
    const { fake, user, pane } = await opened({
      get_task: () => {
        if (gone) throw new Error("There is no task t2");
        return detail();
      },
    });
    await user.click(within(pane).getByRole("button", { name: "Close the task" }));
    expect(screen.queryByRole("complementary", { name: "Task" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Build signup form" }));
    await screen.findByRole("complementary", { name: "Task" });
    gone = true;
    fake.change();
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Task" })).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows a removed dot as the assignee it was", async () => {
    const removed = { kind: "dot", id: "dx", name: null };
    await opened({ get_task: () => detail({ assignee: removed }) });
    const select = screen.getByRole("combobox", { name: "Assignee" });
    expect(select).toHaveValue("dx");
    expect(within(select).getByRole("option", { name: "● removed dot" })).toBeVisible();
  });

  it("keeps the spec being edited, the comment and the subtask draft across a redraw", async () => {
    const { fake, user, pane } = await opened();
    await user.type(within(pane).getByRole("textbox", { name: "Comment" }), "Half a thou");
    await user.type(within(pane).getByRole("textbox", { name: "New subtask" }), "Dep");
    fake.change();
    await waitFor(() => expect(fake.callsTo("get_task")).toHaveLength(2));
    const now = screen.getByRole("complementary", { name: "Task" });
    expect(within(now).getByRole("textbox", { name: "Comment" })).toHaveValue("Half a thou");
    expect(within(now).getByRole("textbox", { name: "New subtask" })).toHaveValue("Dep");
    expect(within(now).getByRole("textbox", { name: "New subtask" })).toHaveFocus();
  });

  it("a refused spec save keeps what the person wrote", async () => {
    const { user, pane } = await opened({
      update_task: () => {
        throw new Error("The spec is too long");
      },
    });
    await user.click(within(pane).getByRole("button", { name: "Edit the spec" }));
    const box = screen.getByRole("textbox", { name: "Spec" });
    await user.clear(box);
    await user.type(box, "My new spec");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The spec is too long");
    expect(screen.getByRole("textbox", { name: "Spec" })).toHaveValue("My new spec");
  });

  it("closing the pane while a change arrives keeps it closed", async () => {
    let release: () => void = () => {};
    let hold = false;
    const { fake, user, pane } = await opened({
      get_task: async () => {
        if (hold) await new Promise<void>((resolve) => (release = resolve));
        return detail();
      },
    });
    hold = true;
    fake.change();
    await waitFor(() => expect(fake.callsTo("get_task")).toHaveLength(2));
    await user.click(within(pane).getByRole("button", { name: "Close the task" }));
    release();
    await waitFor(() => expect(fake.callsTo("list_tasks").length).toBeGreaterThan(2));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByRole("complementary", { name: "Task" })).toBeNull();
  });

  it("two quick clicks on Comment add one comment", async () => {
    const { fake, pane } = await opened({ comment: () => ({ id: "a9" }) });
    const box = within(pane).getByRole("textbox", { name: "Comment" });
    fireEvent.change(box, { target: { value: "Once" } });
    const button = within(pane).getByRole("button", { name: "Comment" });
    fireEvent.click(button);
    fireEvent.click(button);
    await waitFor(() => expect(fake.callsTo("get_task")).toHaveLength(2));
    expect(fake.callsTo("comment")).toEqual([{ task_id: "t2", text: "Once" }]);
  });

  it("opening and closing a task keeps the list where it was", async () => {
    const fake = sample({ get_task: () => detail() });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    const list = () => document.querySelector(".list") as HTMLElement;
    list().scrollTop = 40;
    await user.click(screen.getByRole("button", { name: "Build signup form" }));
    const pane = await screen.findByRole("complementary", { name: "Task" });
    expect(list().scrollTop).toBe(40);
    await user.click(within(pane).getByRole("button", { name: "Close the task" }));
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Task" })).toBeNull());
    expect(list().scrollTop).toBe(40);
  });
});

// Projects' screen (app library spec section 8, mockup A), run in jsdom against a fake `dots`:
// the left column, the list grouped by section, inline adds, sections and dragging.
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { copy, landlord, sample, show, signup, task, website } from "./projects-fake";

const region = (name: string) => screen.getByRole("region", { name });
const row = (name: string) => screen.getByRole("row", { name });

describe("Projects' screen", () => {
  it("opens on My tasks, with the projects and New project in the left column", async () => {
    const fake = sample();
    await show(fake);
    const nav = screen.getByRole("navigation", { name: "Projects" });
    expect(within(nav).getByRole("button", { name: "My tasks" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(within(nav).getByRole("button", { name: "Assigned to dots" })).toBeVisible();
    expect(within(nav).getByRole("button", { name: "● Website relaunch" })).toBeVisible();
    expect(within(nav).getByRole("textbox", { name: "New project" })).toBeVisible();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("My tasks");
    // The to-dos first, then each project's tasks.
    expect(within(region("To-dos")).getByRole("row", { name: "Call the landlord" })).toBeVisible();
    expect(within(region("Website relaunch")).getAllByRole("row")).toHaveLength(2);
    expect(fake.callsTo("list_tasks")).toEqual([{ view: "my_tasks", limit: 1000 }]);
  });

  it("shows a project's tasks by section with Task, Assignee, Due and Status", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Website relaunch");
    const head = screen.getAllByRole("row")[0]!;
    expect(head).toHaveTextContent("TaskAssigneeDueStatus");
    const todo = region("To do");
    expect(within(todo).getAllByRole("row").map((r) => r.getAttribute("aria-label"))).toEqual([
      "Write pricing page copy",
      "Build signup form",
    ]);
    expect(row("Build signup form")).toHaveTextContent("● builder");
    expect(row("Build signup form")).toHaveTextContent("Oct 25");
    expect(row("Build signup form")).toHaveTextContent("In progress");
    expect(row("Write pricing page copy")).toHaveTextContent("You");
    expect(within(region("Done")).getByRole("button", { name: "Mark Pick hosting not done" }))
      .toHaveTextContent("✓");
  });

  it("shows the tasks assigned to dots, and a removed dot as such", async () => {
    const gone = task({ id: "t5", title: "Old", assignee: { kind: "dot", id: "dx", name: null } });
    const fake = sample({ list_tasks: () => ({ tasks: [signup, gone], more: false }) });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "Assigned to dots" }));
    expect(fake.callsTo("list_tasks").at(-1)).toEqual({ view: "assigned_to_dots", limit: 1000 });
    expect(row("Old")).toHaveTextContent("● removed dot");
  });

  it("adds a task inline to its section and keeps the cursor there", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    await user.type(screen.getByRole("textbox", { name: "Add a task to To do" }), "Tests{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("create_task")).toEqual([
        { title: "Tests", project_id: "p1", section_id: "s1" },
      ]),
    );
    await waitFor(() =>
      expect(document.activeElement).toHaveAccessibleName("Add a task to To do"),
    );
    // A to-do from My tasks has no project.
    await user.click(screen.getByRole("button", { name: "My tasks" }));
    await user.type(screen.getByRole("textbox", { name: "Add a task to To-dos" }), "Rent{Enter}");
    await waitFor(() => expect(fake.callsTo("create_task").at(-1)).toEqual({ title: "Rent" }));
  });

  it("adds a section, renames one on Enter and leaves it as it was on Escape", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    await user.type(screen.getByRole("textbox", { name: "New section" }), "Launch{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("create_section")).toEqual([{ project_id: "p1", name: "Launch" }]),
    );
    await user.click(screen.getByRole("button", { name: "To do" }));
    const rename = screen.getByRole("textbox", { name: "Rename To do" });
    await user.clear(rename);
    await user.type(rename, "Backlog{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("rename_section")).toEqual([{ id: "s1", name: "Backlog" }]),
    );
    await user.click(screen.getByRole("button", { name: "Done" }));
    await user.type(screen.getByRole("textbox", { name: "Rename Done" }), "x{Escape}");
    expect(fake.callsTo("rename_section")).toHaveLength(1);
  });

  it("creates a project and opens it", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.type(screen.getByRole("textbox", { name: "New project" }), "Home{Enter}");
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Home"),
    );
    expect(fake.callsTo("create_project")).toEqual([{ name: "Home" }]);
    expect(screen.getByRole("button", { name: "● Home" })).toHaveAttribute("aria-current", "page");
  });

  it("moves a task dropped on another section's task before it, or to a section's end", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    fireEvent.dragStart(row("Write pricing page copy"));
    fireEvent.drop(row("Pick hosting"));
    await waitFor(() =>
      expect(fake.callsTo("move_task")).toEqual([{ id: "t1", section_id: "s2", before_id: "t3" }]),
    );
    fireEvent.dragStart(row("Pick hosting"));
    fireEvent.drop(region("To do"));
    await waitFor(() =>
      expect(fake.callsTo("move_task").at(-1)).toEqual({
        id: "t3",
        section_id: "s1",
        before_id: null,
      }),
    );
  });

  it("marks a task Done with its circle, and shows a refusal above the list", async () => {
    let refuse = false;
    const fake = sample({
      update_task: () => {
        if (refuse) throw new Error("There is no task t1");
        return { task: copy, changes: [] };
      },
    });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "Mark Write pricing page copy done" }));
    await waitFor(() =>
      expect(fake.callsTo("update_task")).toEqual([{ id: "t1", status: "done" }]),
    );
    refuse = true;
    await user.click(screen.getByRole("button", { name: "Mark Pick hosting not done" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("There is no task t1");
  });

  it("follows the app's data when it changes", async () => {
    const tasks = [landlord];
    const fake = sample({ list_tasks: () => ({ tasks, more: false }) });
    await show(fake);
    tasks.push(task({ id: "t8", title: "Pack boxes", project_id: null, section_id: null }));
    fake.change();
    expect(await screen.findByRole("row", { name: "Pack boxes" })).toBeVisible();
  });

  it("a change from a dot keeps what the person is typing and where they are", async () => {
    const tasks = [landlord];
    const fake = sample({ list_tasks: () => ({ tasks, more: false }) });
    const user = await show(fake);
    const box = screen.getByRole("textbox", { name: "Add a task to To-dos" });
    await user.type(box, "Pack the");
    tasks.push(task({ id: "t8", title: "Pack boxes", project_id: null, section_id: null }));
    fake.change();
    expect(await screen.findByRole("row", { name: "Pack boxes" })).toBeVisible();
    const again = screen.getByRole("textbox", { name: "Add a task to To-dos" });
    expect(again).toHaveValue("Pack the");
    expect(again).toHaveFocus();
  });

  it("a title is shown as text, never as markup", async () => {
    const evil = task({
      id: "t7",
      title: '<img src=x onerror="window.__hit=1">',
      project_id: null,
      section_id: null,
    });
    const fake = sample({ list_tasks: () => ({ tasks: [evil], more: false }) });
    await show(fake);
    expect(row('<img src=x onerror="window.__hit=1">')).toHaveTextContent(
      '<img src=x onerror="window.__hit=1">',
    );
    expect(document.querySelector("img")).toBeNull();
    expect((window as unknown as { __hit?: number }).__hit).toBeUndefined();
  });

  it("a list that fails to load says so and the page stays usable", async () => {
    let fail = true;
    const fake = sample({
      list_tasks: () => {
        if (fail) throw new Error("the list is down");
        return { tasks: [landlord], more: false };
      },
    });
    await show(fake);
    expect(screen.getByRole("alert")).toHaveTextContent("the list is down");
    fail = false;
    fake.change();
    expect(await screen.findByRole("row", { name: "Call the landlord" })).toBeVisible();
  });

  it("creating a project whose list then fails to load does not break the page", async () => {
    let fail = false;
    const fake = sample({
      list_projects: () => {
        if (fail) throw new Error("no projects now");
        return [website];
      },
      create_project: () => {
        fail = true;
        return { id: "p2", name: "Home", position: 2, sections: [] };
      },
    });
    const user = await show(fake);
    await user.type(screen.getByRole("textbox", { name: "New project" }), "Home{Enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent("no projects now");
    fail = false;
    await user.click(screen.getByRole("button", { name: "My tasks" }));
    expect(await screen.findByRole("row", { name: "Call the landlord" })).toBeVisible();
  });

  it("two quick Enters add one task", async () => {
    const fake = sample();
    await show(fake);
    const box = screen.getByRole("textbox", { name: "Add a task to To-dos" });
    fireEvent.change(box, { target: { value: "Rent" } });
    fireEvent.keyDown(box, { key: "Enter" });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(fake.callsTo("list_tasks")).toHaveLength(2));
    expect(fake.callsTo("create_task")).toEqual([{ title: "Rent" }]);
  });

  it("a drop after a drag has ended moves nothing", async () => {
    const fake = sample();
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    fireEvent.dragStart(row("Write pricing page copy"));
    fireEvent.dragEnd(row("Write pricing page copy"));
    fireEvent.drop(row("Pick hosting"));
    expect(fake.callsTo("move_task")).toEqual([]);
  });

  it("two sections with the same name keep their own drafts and focus", async () => {
    const twins = {
      ...website,
      sections: [
        { id: "s1", name: "To do", position: 1 },
        { id: "s2", name: "To do", position: 2 },
      ],
    };
    const fake = sample({
      list_projects: () => [twins],
      list_tasks: () => ({ tasks: [], more: false }),
    });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    const boxes = () => screen.getAllByRole("textbox", { name: "Add a task to To do" });
    await user.type(boxes()[1]!, "Second");
    fake.change();
    await waitFor(() => expect(fake.callsTo("list_projects")).toHaveLength(3));
    expect(boxes()[0]).toHaveValue("");
    expect(boxes()[1]).toHaveValue("Second");
    expect(boxes()[1]).toHaveFocus();
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(fake.callsTo("create_task")).toEqual([
        { title: "Second", project_id: "p1", section_id: "s2" },
      ]),
    );
  });

  it("opening another project starts at the top of its list", async () => {
    const other = { id: "p2", name: "Home", position: 2, sections: [] };
    const fake = sample({ list_projects: () => [website, other] });
    const user = await show(fake);
    await user.click(screen.getByRole("button", { name: "● Website relaunch" }));
    const list = () => document.querySelector(".list") as HTMLElement;
    list().scrollTop = 50;
    fake.change();
    await waitFor(() => expect(fake.callsTo("list_projects")).toHaveLength(3));
    expect(list().scrollTop).toBe(50);
    await user.click(screen.getByRole("button", { name: "● Home" }));
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Home"));
    expect(list().scrollTop).toBe(0);
  });
});

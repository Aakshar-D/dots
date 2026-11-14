import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Intent, IntentFields } from "@/api/types";
import { addDays, localDate } from "@/lib/intents";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const today = localDate(new Date());

function goal(id: string, over: Partial<Intent> = {}): Intent {
  return {
    id,
    org_id: null,
    workspace_id: "default",
    text: `Goal ${id}`,
    due_date: null,
    urgent: false,
    dot_id: null,
    created_at: "2026-11-01T09:00:00.000Z",
    done_at: null,
    ...over,
  };
}

/// A backend holding the dot and `intents` of Personal's Default workspace, changed in place
/// as the core would; `refusal` is why this PC may not change them.
function holding(intents: Intent[], refusal: string | null = null) {
  const held = { intents: [...intents] };
  home([]);
  backend.on("intents_list", () => ({ intents: held.intents, refusal }));
  backend.on("intent_create", (args) => {
    const fields = args.fields as IntentFields;
    const made = goal(`new-${held.intents.length}`, { ...fields, text: fields.text.trim() });
    held.intents.push(made);
    return made;
  });
  backend.on("intent_update", (args) => {
    const fields = args.fields as IntentFields;
    held.intents = held.intents.map((i) => (i.id === args.id ? { ...i, ...fields } : i));
    return held.intents.find((i) => i.id === args.id);
  });
  backend.on("intent_set_done", (args) => {
    held.intents = held.intents.map((i) =>
      i.id === args.id ? { ...i, done_at: args.done ? "2026-11-05T10:00:00.000Z" : null } : i,
    );
    return held.intents.find((i) => i.id === args.id);
  });
  backend.on("intent_delete", (args) => {
    held.intents = held.intents.filter((i) => i.id !== args.id);
    return null;
  });
  return held;
}

async function open() {
  const view = renderApp("/intents");
  expect(await screen.findByRole("heading", { name: "Intents" })).toBeVisible();
  return view;
}

const headings = () => screen.queryAllByRole("heading", { level: 2 }).map((h) => h.textContent);

describe("the Intents screen", () => {
  it("is in the main navigation", async () => {
    holding([]);
    const { user } = renderApp("/home");
    await user.click(await screen.findByRole("link", { name: "Intents" }));
    expect(await screen.findByRole("heading", { name: "Intents" })).toBeVisible();
    expect(screen.getByText("Goals, grouped by how soon they matter.")).toBeVisible();
  });

  it("groups goals by how soon they matter, empty groups left out, done folded away", async () => {
    holding([
      goal("none"),
      goal("later", { due_date: addDays(today, 8) }),
      goal("week", { due_date: addDays(today, 7) }),
      goal("today", { due_date: today }),
      goal("overdue", { due_date: addDays(today, -1) }),
      goal("urgent", { urgent: true, due_date: addDays(today, 30) }),
      goal("done", { done_at: "2026-11-04T09:00:00.000Z" }),
    ]);
    const { user } = await open();
    await screen.findByRole("region", { name: "Now" });
    expect(headings()).toEqual(["Now", "Overdue", "Today", "This week", "Later", "No date"]);
    const items = (name: string) =>
      within(screen.getByRole("region", { name }))
        .getAllByRole("listitem")
        .map((li) => li.getAttribute("aria-label"));
    expect(items("Now")).toEqual(["Goal urgent"]);
    expect(items("Overdue")).toEqual(["Goal overdue"]);
    expect(items("This week")).toEqual(["Goal week"]);
    expect(items("Later")).toEqual(["Goal later"]);
    // Done is folded until opened.
    const toggle = screen.getByRole("button", { name: "Done (1)" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("listitem", { name: "Goal done" })).toBeNull();
    await user.click(toggle);
    expect(screen.getByRole("listitem", { name: "Goal done" })).toBeVisible();
  });

  it("says there are no goals yet, under the add box", async () => {
    holding([]);
    await open();
    expect(
      await screen.findByText("No goals yet. Add one above, with a date if it has one."),
    ).toBeVisible();
    expect(screen.getByRole("form", { name: "Add a goal" })).toBeVisible();
    expect(headings()).toEqual([]);
  });

  it("adds a goal with a due date, urgent and a sol", async () => {
    const held = holding([]);
    const { user } = await open();
    const form = await screen.findByRole("form", { name: "Add a goal" });
    const add = within(form).getByRole("button", { name: "Add" });
    expect(add).toBeDisabled();
    await user.type(within(form).getByRole("textbox", { name: "Goal" }), "  Plan the offsite ");
    await user.type(within(form).getByLabelText("Due date"), addDays(today, 3));
    await user.click(within(form).getByRole("switch", { name: "Urgent" }));
    await user.selectOptions(within(form).getByRole("combobox", { name: "Sol" }), dot.name);
    await user.click(add);
    await waitFor(() =>
      expect(backend.callsTo("intent_create")).toEqual([
        {
          scope: "personal",
          workspace: "default",
          fields: {
            text: "  Plan the offsite ",
            due_date: addDays(today, 3),
            urgent: true,
            dot_id: dot.id,
          },
        },
      ]),
    );
    expect(held.intents).toHaveLength(1);
    // The box is cleared, and the goal shows under Now.
    expect(await screen.findByRole("listitem", { name: "Plan the offsite" })).toBeVisible();
    expect(within(form).getByRole("textbox", { name: "Goal" })).toHaveValue("");
    expect(within(screen.getByRole("region", { name: "Now" })).getByText(dot.name, { exact: false }))
      .toBeVisible();
  });

  it("shows the core's refusal under the add box", async () => {
    holding([]);
    backend.on("intent_create", () => {
      throw { kind: "invalid", message: "A due date must be a real calendar date, like 2026-11-05." };
    });
    const { user } = await open();
    const form = await screen.findByRole("form", { name: "Add a goal" });
    await user.type(within(form).getByRole("textbox", { name: "Goal" }), "Book it{Enter}");
    expect(await within(form).findByRole("alert")).toHaveTextContent(
      "A due date must be a real calendar date, like 2026-11-05.",
    );
    // The text stays for another try.
    expect(within(form).getByRole("textbox", { name: "Goal" })).toHaveValue("Book it");
  });

  it("completes a goal with its checkbox, and undoes it from Done", async () => {
    holding([goal("a", { text: "Renew the passport" })]);
    const { user } = await open();
    await user.click(await screen.findByRole("checkbox", { name: 'Mark "Renew the passport" done' }));
    await waitFor(() =>
      expect(backend.callsTo("intent_set_done")).toEqual([{ id: "a", done: true }]),
    );
    await user.click(await screen.findByRole("button", { name: "Done (1)" }));
    await user.click(screen.getByRole("checkbox", { name: 'Mark "Renew the passport" not done' }));
    await waitFor(() =>
      expect(backend.callsTo("intent_set_done")).toEqual([
        { id: "a", done: true },
        { id: "a", done: false },
      ]),
    );
    expect(await screen.findByRole("region", { name: "No date" })).toBeVisible();
  });

  it("edits a goal in a small dialog", async () => {
    holding([goal("a", { text: "Book the venue" })]);
    const { user } = await open();
    const item = await screen.findByRole("listitem", { name: "Book the venue" });
    await user.click(within(item).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit goal" });
    const text = within(dialog).getByRole("textbox", { name: "Goal" });
    await user.clear(text);
    await user.type(text, "Book the bigger venue");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("intent_update")).toEqual([
        {
          id: "a",
          fields: { text: "Book the bigger venue", due_date: null, urgent: false, dot_id: null },
        },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByRole("listitem", { name: "Book the bigger venue" })).toBeVisible();
  });

  it("deletes a goal once asked", async () => {
    holding([goal("a", { text: "Old idea" })]);
    const { user } = await open();
    const item = await screen.findByRole("listitem", { name: "Old idea" });
    await user.click(within(item).getByRole("button", { name: "Delete" }));
    await user.click(await screen.findByRole("button", { name: "Delete" }));
    await waitFor(() => expect(backend.callsTo("intent_delete")).toEqual([{ id: "a" }]));
    expect(await screen.findByText(/No goals yet/)).toBeVisible();
  });

  it("starts a new topic with the goal's sol, the goal as an unsent draft", async () => {
    holding([goal("a", { text: "Draft the newsletter", dot_id: dot.id })]);
    const { user, router } = await open();
    const item = await screen.findByRole("listitem", { name: "Draft the newsletter" });
    // A goal with no dot has no Start; this one does.
    await user.click(within(item).getByRole("button", { name: "Start" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}`));
    const box = await screen.findByLabelText("Message");
    expect(box).toHaveValue("Draft the newsletter");
    expect(screen.getByRole("group", { name: "Reply target" })).toHaveTextContent("New topic");
    expect(backend.callsTo("chat")).toEqual([]);
    // The draft is not carried by the history entry any more.
    await waitFor(() => expect(router.state.location.state).toBeNull());
  });

  it("has no Start for a goal without a sol", async () => {
    holding([goal("a", { text: "Think it over" })]);
    await open();
    const item = await screen.findByRole("listitem", { name: "Think it over" });
    expect(within(item).queryByRole("button", { name: "Start" })).toBeNull();
  });

  it("shows the list read-only to whoever can't change goals", async () => {
    const refusal = "Acme Engineering: a Viewer can't change goals. Ask an Admin or Owner.";
    holding(
      [
        goal("a", { text: "Close the quarter", dot_id: dot.id }),
        goal("b", { text: "Closed one", done_at: "2026-11-04T09:00:00.000Z" }),
      ],
      refusal,
    );
    const { user } = await open();
    expect(await screen.findByText(refusal)).toBeVisible();
    expect(screen.queryByRole("form", { name: "Add a goal" })).toBeNull();
    const item = screen.getByRole("listitem", { name: "Close the quarter" });
    expect(within(item).getByRole("checkbox")).toBeDisabled();
    for (const name of ["Start", "Edit", "Delete"]) {
      expect(within(item).queryByRole("button", { name })).toBeNull();
    }
    // Done can still be opened to look.
    await user.click(screen.getByRole("button", { name: "Done (1)" }));
    expect(
      within(screen.getByRole("listitem", { name: "Closed one" })).getByRole("checkbox"),
    ).toBeDisabled();
  });

  it("lists the goals of the workspace the person works in", async () => {
    holding([]);
    backend.on("scope_get", () => ({ ...sample.scope, account: "acme-eng", workspace: "platform" }));
    await open();
    await waitFor(() =>
      expect(backend.callsTo("intents_list")).toContainEqual({
        scope: "acme-eng",
        workspace: "platform",
      }),
    );
  });
});

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const row = (name: string): DotOverview => ({
  dot: { ...sample.dot, id: `dot-${name}`, name, schedule: null, enabled: true },
  active_run: null,
  last_run: null,
  schedule_state: null,
  needs_you: 0,
  unread: 0,
  made_dots: 0,
});

/// The sols the backend has; `delete_dot` takes one out unless its name is in `stuck`.
function sols(names: string[], stuck: string[] = []) {
  let rows = names.map(row);
  backend.on("dot_overview", () => rows);
  backend.on("delete_dot", ({ id }) => {
    const found = rows.find((r) => r.dot.id === id);
    if (found && stuck.includes(found.dot.name)) {
      throw { kind: "invalid", message: "it is in a workflow" };
    }
    rows = rows.filter((r) => r.dot.id !== id);
    return null;
  });
}

const column = () => screen.getByRole("navigation", { name: "Sols" });
const names = () =>
  within(column())
    .getAllByRole("listitem")
    .map((li) => li.getAttribute("aria-label"));

async function open() {
  const view = renderApp("/home");
  await within(await screen.findByRole("navigation", { name: "Sols" })).findByRole("listitem", {
    name: "one",
  });
  return view;
}

describe("deleting several sols at once", () => {
  it("deletes the sols picked, after asking, and leaves the others", async () => {
    sols(["one", "two", "three"]);
    const { user } = await open();
    // Nothing is picked, and nothing can be deleted, until Select.
    expect(within(column()).queryByRole("checkbox")).toBeNull();
    await user.click(within(column()).getByRole("button", { name: "Select" }));
    const remove = () => within(column()).getByRole("button", { name: /^Delete/ });
    expect(remove()).toBeDisabled();

    await user.click(within(column()).getByRole("checkbox", { name: "Select one" }));
    await user.click(within(column()).getByRole("checkbox", { name: "Select three" }));
    expect(within(column()).getByText("2 selected")).toBeVisible();
    expect(remove()).toHaveTextContent("Delete 2 sols");
    // Picking opens no sol.
    expect(backend.callsTo("delete_dot")).toEqual([]);

    await user.click(remove());
    const dialog = await screen.findByRole("alertdialog", { name: "Delete 2 sols?" });
    expect(within(dialog).getByText("one, three")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(names()).toEqual(["two"]));
    expect(backend.callsTo("delete_dot")).toEqual([{ id: "dot-one" }, { id: "dot-three" }]);
    expect(await screen.findByText("2 sols deleted")).toBeVisible();
    // The list is the usual one again.
    await waitFor(() => expect(within(column()).queryByRole("checkbox")).toBeNull());
    expect(within(column()).getByRole("button", { name: "Select" })).toBeVisible();
  });

  it("deletes nothing when the person cancels, and Done leaves the list as it was", async () => {
    sols(["one", "two"]);
    const { user } = await open();
    await user.click(within(column()).getByRole("button", { name: "Select" }));
    await user.click(within(column()).getByRole("button", { name: "All" }));
    expect(within(column()).getByText("2 selected")).toBeVisible();
    await user.click(within(column()).getByRole("button", { name: "Delete 2 sols" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("delete_dot")).toEqual([]);

    await user.click(within(column()).getByRole("button", { name: "None" }));
    expect(within(column()).getByText("0 selected")).toBeVisible();
    await user.click(within(column()).getByRole("button", { name: "Done" }));
    expect(within(column()).queryByRole("checkbox")).toBeNull();
    expect(names()).toEqual(["one", "two"]);
  });

  it("deletes the others when one cannot be deleted, and keeps that one picked", async () => {
    sols(["one", "two", "three"], ["two"]);
    const { user } = await open();
    await user.click(within(column()).getByRole("button", { name: "Select" }));
    await user.click(within(column()).getByRole("button", { name: "All" }));
    await user.click(within(column()).getByRole("button", { name: "Delete 3 sols" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete 3 sols?" });
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(names()).toEqual(["two"]));
    // Two toasts, one over the other: the one under is on the page but not in view.
    expect(await screen.findByText("2 sols deleted")).toBeInTheDocument();
    expect(await screen.findByText("two was not deleted: it is in a workflow")).toBeVisible();
    expect(within(column()).getByRole("checkbox", { name: "Select two" })).toBeChecked();
    expect(within(column()).getByRole("button", { name: "Delete 1 sol" })).toBeEnabled();
  });
});

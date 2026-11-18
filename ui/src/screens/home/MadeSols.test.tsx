import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { underMakers } from "@/screens/home/DotsColumn";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

type Over = { by?: string | null; byName?: string; on?: boolean; made?: number; total?: number };

/// A sol named `name`; with `by`, one the sol `by` made (null: its maker is deleted).
const row = (name: string, over: Over = {}): DotOverview => ({
  dot: {
    ...sample.dot,
    id: `dot-${name}`,
    name,
    schedule: null,
    enabled: true,
    making: { on: over.on ?? false, per_run: 3, total: over.total ?? 10 },
    made:
      over.by === undefined
        ? null
        : {
            by_dot_id: over.by === null ? null : `dot-${over.by}`,
            by_name: over.byName ?? over.by ?? "Researcher",
            run_id: "01J0RUN",
            purpose: "Reads the feeds",
          },
  },
  active_run: null,
  last_run: null,
  schedule_state: null,
  needs_you: 0,
  unread: 0,
  made_dots: over.made ?? 0,
});

const column = () => screen.getByRole("navigation", { name: "Sols" });
const items = () => within(column()).getAllByRole("listitem");
const item = (name: string) => within(column()).getByRole("listitem", { name });

async function open(rows: DotOverview[]) {
  let held = rows;
  backend.on("dot_overview", () => held);
  backend.on("delete_dot", ({ id }) => {
    held = held.filter((r) => r.dot.id !== id);
    return null;
  });
  const view = renderApp("/home");
  await within(await screen.findByRole("navigation", { name: "Sols" })).findByRole("listitem", {
    name: rows[0]!.dot.name,
  });
  return view;
}

describe("made sols in the sols list", () => {
  it("puts each made sol under its maker, stepped in, with who made it", async () => {
    await open([
      row("Alpha"),
      row("Blog Watcher", { by: "Researcher" }),
      row("Researcher", { on: true, made: 2 }),
      row("Feeds", { by: "Researcher" }),
      row("Zeta"),
    ]);
    expect(items().map((li) => li.getAttribute("aria-label"))).toEqual([
      "Alpha",
      "Researcher",
      "Blog Watcher",
      "Feeds",
      "Zeta",
    ]);
    expect(items().map((li) => li.getAttribute("data-depth"))).toEqual(["0", "0", "1", "1", "0"]);
    expect(item("Blog Watcher")).toHaveClass("ml-4");
    expect(item("Researcher")).not.toHaveClass("ml-4");
    expect(item("Blog Watcher")).toHaveTextContent("made by Researcher");
    expect(item("Blog Watcher")).not.toHaveTextContent("(deleted)");
    expect(item("Alpha")).not.toHaveTextContent("made by");
  });

  it("says how many sols a maker made of how many it may keep", async () => {
    await open([
      row("Researcher", { on: true, made: 2, total: 10 }),
      row("Plain"),
      // The switch is off again: it is no maker, whatever it made before.
      row("Retired", { made: 1 }),
    ]);
    expect(item("Researcher")).toHaveTextContent("can make sols · 2 of 10");
    expect(item("Plain")).not.toHaveTextContent("can make sols");
    expect(item("Retired")).not.toHaveTextContent("can make sols");
  });

  it("keeps a made sol whose maker was deleted at the top, with its tag", async () => {
    await open([row("Orphan", { by: null, byName: "Researcher" }), row("Other")]);
    expect(item("Orphan")).toHaveAttribute("data-depth", "0");
    expect(item("Orphan")).not.toHaveClass("ml-4");
    expect(item("Orphan")).toHaveTextContent("made by Researcher (deleted)");
  });

  it("steps a sol made by a made sol in once more, and no further than that", () => {
    const placed = underMakers([
      row("D", { by: "C" }),
      row("C", { by: "B" }),
      row("A", { on: true }),
      row("B", { by: "A" }),
      // Its maker is in another workspace, so not in this list.
      row("Far", { by: "Elsewhere" }),
    ]);
    expect(placed.map(({ row: r, depth }) => [r.dot.name, depth])).toEqual([
      ["A", 0],
      ["B", 1],
      ["C", 2],
      ["D", 3],
      ["Far", 0],
    ]);
    // Makers that name each other still show, each once.
    const ring = underMakers([row("X", { by: "Y" }), row("Y", { by: "X" })]);
    expect(ring.map(({ row: r }) => r.dot.name).sort()).toEqual(["X", "Y"]);
  });

  it("selects and deletes made sols like any other, and says what happens to a maker's sols", async () => {
    const { user } = await open([
      row("Researcher", { on: true, made: 1 }),
      row("Blog Watcher", { by: "Researcher" }),
      row("Other"),
    ]);
    await user.click(within(column()).getByRole("button", { name: "Select" }));
    // The made sol keeps its place under its maker while picking.
    expect(items().map((li) => li.getAttribute("aria-label"))).toEqual([
      "Other",
      "Researcher",
      "Blog Watcher",
    ]);
    expect(item("Blog Watcher")).toHaveClass("ml-4");

    // A made sol alone: nothing about makers.
    await user.click(within(column()).getByRole("checkbox", { name: "Select Blog Watcher" }));
    await user.click(within(column()).getByRole("button", { name: "Delete 1 sol" }));
    let dialog = await screen.findByRole("alertdialog", { name: "Delete 1 sol?" });
    expect(dialog).not.toHaveTextContent("made sols");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    // With its maker: the maker's sols are only deleted when picked themselves.
    await user.click(within(column()).getByRole("checkbox", { name: "Select Researcher" }));
    await user.click(within(column()).getByRole("button", { name: "Delete 2 sols" }));
    dialog = await screen.findByRole("alertdialog", { name: "Delete 2 sols?" });
    expect(dialog).toHaveTextContent(
      "Researcher made sols. Those are not deleted with their maker: only the sols you selected are.",
    );
    expect(dialog).toHaveTextContent("a maker's requests that wait for your answer are dropped");
    expect(within(dialog).getByText("Researcher, Blog Watcher")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    // The maker goes first, then the sol it made.
    await waitFor(() =>
      expect(items().map((li) => li.getAttribute("aria-label"))).toEqual(["Other"]),
    );
    expect(backend.callsTo("delete_dot")).toEqual([
      { id: "dot-Researcher" },
      { id: "dot-Blog Watcher" },
    ]);
  });
});

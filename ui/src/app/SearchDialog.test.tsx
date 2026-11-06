import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { SearchHit } from "@/api/types";
import { HIT_END, HIT_START } from "@/lib/search";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const chatHit: SearchHit = {
  kind: "chat",
  dot_id: "dot-1",
  dot_name: "planner",
  target: { type: "topic", dot_id: "dot-1", root_run_id: "root-1", run_id: "run-2" },
  at: "2026-11-01T09:00:00.000Z",
  snippet: `Plan the ${HIT_START}Lisbon${HIT_END} trip <img src=x onerror=alert(1)>`,
};
const memoryHit: SearchHit = {
  kind: "memory",
  dot_id: "dot-1",
  dot_name: "planner",
  target: { type: "memory", dot_id: "dot-1", memory_id: "mem-1" },
  at: "2026-11-01T09:00:00.000Z",
  snippet: `Likes ${HIT_START}Lisbon${HIT_END} in spring`,
};
const runHit: SearchHit = {
  kind: "reply",
  dot_id: "dot-1",
  dot_name: "planner",
  target: { type: "run", run_id: "run-9" },
  at: "2026-11-01T09:00:00.000Z",
  snippet: `Booked a ${HIT_START}Lisbon${HIT_END} hotel`,
};

/// Opens the search box with Ctrl+K and gives its input.
async function openSearch() {
  await screen.findByRole("navigation", { name: "Main" });
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
  const dialog = await screen.findByRole("dialog", { name: "Search" });
  return { dialog, box: within(dialog).getByRole("combobox", { name: "Search everything" }) };
}

describe("the search box", () => {
  it("opens with Ctrl+K and with the rail's button, and closes with Escape", async () => {
    const { user } = renderApp("/home");
    const { box } = await openSearch();
    expect(box).toHaveFocus();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: "Search (Ctrl+K)" }));
    expect(await screen.findByRole("dialog", { name: "Search" })).toBeVisible();
  });

  it("searches the workspace in view after a pause and groups the hits by kind", async () => {
    backend.on("search", () => [memoryHit, chatHit]);
    const { user } = renderApp("/home");
    const { dialog, box } = await openSearch();
    await user.type(box, "lisbon");

    const list = await within(dialog).findByRole("listbox", { name: "Results" });
    const groups = within(list).getAllByRole("group");
    const heading = (g: HTMLElement) =>
      document.getElementById(g.getAttribute("aria-labelledby")!)?.textContent;
    expect(groups.map(heading)).toEqual(["You said", "Memory"]);
    // One search for the settled text, not one per key.
    expect(backend.callsTo("search")).toEqual([
      {
        scope: "personal",
        workspace: "default",
        query: { text: "lisbon", kinds: [], dot_id: null, limit: null },
      },
    ]);
    // The matched word is marked; the rest of the snippet is text, never markup.
    const first = within(list).getAllByRole("option")[0]!;
    expect(within(first).getByText("Lisbon").tagName).toBe("MARK");
    expect(first).toHaveTextContent("<img src=x onerror=alert(1)>");
    expect(first.querySelector("img")).toBeNull();
  });

  it("moves through the hits with the arrow keys and opens one with Enter", async () => {
    backend.on("search", () => [chatHit, runHit, memoryHit]);
    const { user, router } = renderApp("/home");
    const { dialog, box } = await openSearch();
    await user.type(box, "lisbon");
    const options = await within(dialog).findAllByRole("option");
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    expect(box).toHaveAttribute("aria-activedescendant", options[0]!.id);

    await user.keyboard("{ArrowDown}");
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    expect(box).toHaveAttribute("aria-activedescendant", options[1]!.id);
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(options[2]).toHaveAttribute("aria-selected", "true");

    await user.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-1"));
    expect(router.state.location.search).toBe("?tab=memory");
    expect(screen.queryByRole("dialog", { name: "Search" })).not.toBeInTheDocument();
  });

  it("opens a chat hit in its topic and a run's hit on the run's page", async () => {
    const run = { ...sample.run, dot_id: sample.dot.id };
    const inTopic: SearchHit = {
      ...chatHit,
      dot_id: sample.dot.id,
      target: { type: "topic", dot_id: sample.dot.id, root_run_id: run.root_run_id, run_id: run.id },
    };
    backend.on("dot_overview", () => [{ ...sample.overview, dot: sample.dot }]);
    backend.on("list_dots", () => [sample.dot]);
    backend.on("list_runs", () => [run]);
    backend.on("search", () => [inTopic, runHit]);
    const { user, router } = renderApp("/inbox");
    let { dialog, box } = await openSearch();
    await user.type(box, "lisbon");
    await user.click(await within(dialog).findByRole("option", { name: /Plan the/ }));
    await waitFor(() =>
      expect(router.state.location.pathname).toBe(`/home/${sample.dot.id}/${run.root_run_id}`),
    );

    ({ dialog, box } = await openSearch());
    expect(box).toHaveValue("");
    await user.type(box, "lisbon");
    await user.click(await within(dialog).findByRole("option", { name: /Booked a/ }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/run-9"));
  });

  it("says when nothing matches, and shows why a search failed", async () => {
    const { user } = renderApp("/home");
    const { dialog, box } = await openSearch();
    expect(within(dialog).getByText(/Type a word to find it/)).toBeVisible();
    await user.type(box, "zzz");
    expect(await within(dialog).findByText("Nothing matches “zzz”.")).toBeVisible();

    backend.on("search", () => {
      throw { kind: "internal", message: "The index could not be read." };
    });
    await user.type(box, "q");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The index could not be read.",
    );
  });

  it("does not search for marks alone", async () => {
    const { user } = renderApp("/home");
    const { box } = await openSearch();
    await user.type(box, "-- ** ::");
    await new Promise((r) => setTimeout(r, 300));
    expect(backend.callsTo("search")).toEqual([]);
    expect(screen.getByText(/Type a word to find it/)).toBeVisible();
  });
});

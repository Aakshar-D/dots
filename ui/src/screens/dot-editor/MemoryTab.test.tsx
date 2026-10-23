import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Memory } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

// A pinned memory the dot saved in a run, and one the person wrote.
const [dots, persons] = sample.memories as [Memory, Memory];

/// A backend holding the dot and `memories`, changed in place as the core would.
function holding(memories: Memory[]) {
  const held = { memories };
  backend.on("get_dot", () => sample.dot);
  backend.on("memories_list", (args) => (args.dotId === sample.dot.id ? held.memories : []));
  return held;
}

/// The editor of the dot, opened on its Memory tab.
async function openTab() {
  const view = renderApp(`/dots/${sample.dot.id}?tab=memory`);
  expect(await screen.findByRole("tab", { name: "Memory", selected: true })).toBeVisible();
  return view;
}

const items = () => within(screen.getByRole("list", { name: "Memories" })).getAllByRole("listitem");

describe("a dot's Memory tab", () => {
  it("lists what the dot remembers, pinned first, with who wrote each and the count", async () => {
    holding([dots, persons]);
    await openTab();
    await screen.findByRole("list", { name: "Memories" });
    const [first, second] = items();
    expect(first).toHaveAccessibleName(dots.text);
    expect(within(first!).getByRole("button", { name: "Pinned" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(first).toHaveTextContent("by the dot · from run");
    expect(within(first!).getByRole("link", { name: localDateTime(dots.created_at) })).toHaveAttribute(
      "href",
      `/runs/${dots.run_id}`,
    );
    expect(second).toHaveAccessibleName(persons.text);
    expect(second).toHaveTextContent("by you");
    expect(within(second!).getByRole("button", { name: "Pinned" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByText("2 of 200")).toBeVisible();
  });

  it("finds memories by the words in their text", async () => {
    holding([dots, persons]);
    const { user } = await openTab();
    const search = await screen.findByRole("searchbox", { name: "Search memories" });
    await user.type(search, "EXAMPLE staging");
    expect(items().map((i) => i.getAttribute("aria-label"))).toEqual([persons.text]);
    // The count is of every memory, not of those found.
    expect(screen.getByText("2 of 200")).toBeVisible();
    await user.clear(search);
    await user.type(search, "nothing like it");
    expect(screen.getByText("No memory has those words.")).toBeVisible();
  });

  it("says nothing is remembered yet", async () => {
    holding([]);
    await openTab();
    expect(
      await screen.findByText(
        "Nothing remembered yet. The dot saves what it learns here; you can add notes too.",
      ),
    ).toBeVisible();
    expect(screen.getByText("0 of 200")).toBeVisible();
  });

  it("adds a memory with a counter of its 500 characters, and shows the core's refusal", async () => {
    const held = holding([]);
    let refuse = true;
    backend.on("memory_add", (args) => {
      if (refuse) throw { kind: "conflict", message: "This dot already remembers that." };
      const saved = { ...persons, id: "01J0NEW", text: String(args.text).trim() };
      held.memories = [saved];
      return saved;
    });
    const { user } = await openTab();
    await user.click(await screen.findByRole("button", { name: "Add memory" }));
    const box = screen.getByRole("textbox", { name: "New memory" });
    const save = within(screen.getByRole("group", { name: "New memory" })).getByRole("button", {
      name: "Save",
    });
    expect(save).toBeDisabled();
    expect(screen.getByText("0 / 500")).toBeVisible();
    fireEvent.change(box, { target: { value: "x".repeat(501) } });
    expect(screen.getByText("501 / 500")).toBeVisible();
    expect(save).toBeDisabled();
    await user.clear(box);
    await user.type(box, " Sends on Fridays. ");
    expect(screen.getByText("17 / 500")).toBeVisible();
    await user.click(save);
    expect(await screen.findByRole("alert")).toHaveTextContent("This dot already remembers that.");
    refuse = false;
    await user.click(save);
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "New memory" })).toBeNull());
    expect(await screen.findByRole("listitem", { name: "Sends on Fridays." })).toBeVisible();
    expect(backend.callsTo("memory_add")).toEqual([
      { dotId: sample.dot.id, text: " Sends on Fridays. " },
      { dotId: sample.dot.id, text: " Sends on Fridays. " },
    ]);
  });

  it("edits, pins and deletes a memory, and asks before it deletes", async () => {
    const held = holding([dots, persons]);
    backend.on("memory_update", (args) => {
      const changed = held.memories.find((m) => m.id === args.id)!;
      const next = {
        ...changed,
        text: (args.text as string | null) ?? changed.text,
        pinned: (args.pinned as boolean | null) ?? changed.pinned,
      };
      held.memories = held.memories.map((m) => (m.id === next.id ? next : m));
      return next;
    });
    backend.on("memory_delete", (args) => {
      held.memories = held.memories.filter((m) => m.id !== args.id);
      return null;
    });
    const { user } = await openTab();
    const item = await screen.findByRole("listitem", { name: persons.text });
    await user.click(within(item).getByRole("button", { name: "Edit" }));
    const box = screen.getByRole("textbox", { name: "Edit memory" });
    expect(box).toHaveValue(persons.text);
    await user.clear(box);
    await user.type(box, "Staging moved to staging2.example.com.");
    await user.click(
      within(screen.getByRole("group", { name: "Edit memory" })).getByRole("button", { name: "Save" }),
    );
    const edited = await screen.findByRole("listitem", {
      name: "Staging moved to staging2.example.com.",
    });

    await user.click(within(edited).getByRole("button", { name: "Pinned" }));
    await waitFor(() =>
      expect(
        within(screen.getByRole("listitem", { name: "Staging moved to staging2.example.com." }))
          .getByRole("button", { name: "Pinned" }),
      ).toHaveAttribute("aria-pressed", "true"),
    );

    await user.click(
      within(screen.getByRole("listitem", { name: dots.text })).getByRole("button", {
        name: "Delete",
      }),
    );
    const dialog = await screen.findByRole("alertdialog", { name: "Delete this memory?" });
    expect(dialog).toHaveTextContent(dots.text);
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("listitem", { name: dots.text })).toBeNull());
    expect(backend.callsTo("memory_update")).toEqual([
      { id: persons.id, text: "Staging moved to staging2.example.com.", pinned: null },
      { id: persons.id, text: null, pinned: true },
    ]);
    expect(backend.callsTo("memory_delete")).toEqual([{ id: dots.id }]);
  });

  it("shows why a pin could not change", async () => {
    holding([dots]);
    backend.on("memory_update", () => {
      throw { kind: "not_found", message: "memory 01J0MEM1" };
    });
    const { user } = await openTab();
    const item = await screen.findByRole("listitem", { name: dots.text });
    await user.click(within(item).getByRole("button", { name: "Pinned" }));
    expect(await screen.findByText("memory 01J0MEM1")).toBeVisible();
  });

  it("refreshes when the core says the dot's memories changed", async () => {
    const held = holding([persons]);
    const { queryClient } = await openTab();
    await screen.findByRole("listitem", { name: persons.text });
    held.memories = [dots, persons];
    act(() => applyRuntimeEvent(queryClient, { type: "memories_updated", dot_id: sample.dot.id }));
    expect(await screen.findByRole("listitem", { name: dots.text })).toBeVisible();
    // Another dot's change leaves this list alone.
    const asked = backend.callsTo("memories_list").length;
    act(() => applyRuntimeEvent(queryClient, { type: "memories_updated", dot_id: "other" }));
    await new Promise((r) => setTimeout(r, 50));
    expect(backend.callsTo("memories_list")).toHaveLength(asked);
  });

  it("gives focus back where the person was after a save, a cancel or a delete", async () => {
    const held = holding([dots, persons]);
    backend.on("memory_add", (args) => {
      const saved = { ...persons, id: "01J0NEW", text: String(args.text).trim() };
      held.memories = [...held.memories, saved];
      return saved;
    });
    backend.on("memory_update", (args) => {
      held.memories = held.memories.map((m) =>
        m.id === args.id ? { ...m, text: String(args.text) } : m,
      );
      return held.memories.find((m) => m.id === args.id)!;
    });
    backend.on("memory_delete", (args) => {
      held.memories = held.memories.filter((m) => m.id !== args.id);
      return null;
    });
    const { user } = await openTab();
    const addButton = await screen.findByRole("button", { name: "Add memory" });

    // Add: Cancel, Escape and Save each return to Add memory.
    await user.click(addButton);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(addButton).toHaveFocus());
    await user.click(addButton);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("textbox", { name: "New memory" })).toBeNull();
    await waitFor(() => expect(addButton).toHaveFocus());
    await user.click(addButton);
    await user.type(screen.getByRole("textbox", { name: "New memory" }), "Sends on Fridays.");
    await user.click(
      within(screen.getByRole("group", { name: "New memory" })).getByRole("button", { name: "Save" }),
    );
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "New memory" })).toBeNull());
    await waitFor(() => expect(addButton).toHaveFocus());

    // Edit: Escape and Save each return to the row's Edit.
    const editOf = (name: string) =>
      within(screen.getByRole("listitem", { name })).getByRole("button", { name: "Edit" });
    await user.click(editOf(persons.text));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(editOf(persons.text)).toHaveFocus());
    await user.click(editOf(persons.text));
    const box = screen.getByRole("textbox", { name: "Edit memory" });
    await user.clear(box);
    await user.type(box, "Staging moved.");
    await user.click(
      within(screen.getByRole("group", { name: "Edit memory" })).getByRole("button", { name: "Save" }),
    );
    await waitFor(() => expect(editOf("Staging moved.")).toHaveFocus());

    // Delete: the next memory's Edit, or Add memory when none is left.
    await user.click(
      within(screen.getByRole("listitem", { name: dots.text })).getByRole("button", { name: "Delete" }),
    );
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }),
    );
    await waitFor(() => expect(editOf("Staging moved.")).toHaveFocus());
    for (const name of ["Staging moved.", "Sends on Fridays."]) {
      await user.click(
        within(screen.getByRole("listitem", { name })).getByRole("button", { name: "Delete" }),
      );
      await user.click(
        within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }),
      );
      await waitFor(() => expect(screen.queryByRole("listitem", { name })).toBeNull());
    }
    await waitFor(() => expect(addButton).toHaveFocus());
  });

  it("ties the counter and a refusal to the box", async () => {
    holding([persons]);
    backend.on("memory_add", () => {
      throw { kind: "conflict", message: "This dot already remembers that." };
    });
    const { user } = await openTab();
    await user.click(await screen.findByRole("button", { name: "Add memory" }));
    const box = screen.getByRole("textbox", { name: "New memory" });
    await user.type(box, "Again.");
    expect(box).toHaveAccessibleDescription("6 / 500");
    expect(box).not.toHaveAttribute("aria-invalid");
    await user.click(
      within(screen.getByRole("group", { name: "New memory" })).getByRole("button", { name: "Save" }),
    );
    await screen.findByRole("alert");
    expect(box).toHaveAttribute("aria-invalid", "true");
    expect(box).toHaveAccessibleDescription("6 / 500 This dot already remembers that.");
  });

  it("offers no Add memory once the dot remembers 200 things", async () => {
    holding(Array.from({ length: 200 }, (_, i) => ({ ...persons, id: `01J0M${i}`, text: `Fact ${i}.` })));
    await openTab();
    expect(await screen.findByText("200 of 200")).toBeVisible();
    expect(screen.getByRole("button", { name: "Add memory" })).toBeDisabled();
  });

  it("opens from the dot's ⚙ menu", async () => {
    home([]);
    holding([persons]);
    const { user, router } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("button", { name: "Dot settings" }));
    await user.click(await screen.findByRole("menuitem", { name: "Memory…" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/dots/${dot.id}`));
    expect(router.state.location.search).toBe("?tab=memory");
    expect(await screen.findByRole("tab", { name: "Memory", selected: true })).toBeVisible();
    expect(await screen.findByRole("listitem", { name: persons.text })).toBeVisible();
  });
});

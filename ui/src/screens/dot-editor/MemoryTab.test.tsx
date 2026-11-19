import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Memory, ProjectMemories } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

// A pinned memory the dot saved in a run, and one the person wrote.
const [dots, persons] = sample.memories as [Memory, Memory];

/// A backend holding the dot and `memories`, changed in place as the core would. The dot has
/// no project unless the test gives it one (`inProject`).
function holding(memories: Memory[]) {
  const held = { memories };
  backend.on("get_dot", () => sample.dot);
  backend.on("memories_list", (args) => (args.dotId === sample.dot.id ? held.memories : []));
  backend.on("project_memories", () => null);
  return held;
}

// A how-to a run saved about two files, and a line the capture step wrote.
const [howTo, episode] = sample.projectMemories.memories as [Memory, Memory];

/// The dot works in a project that remembers `project`, changed in place as the core would.
function inProject(project: ProjectMemories) {
  const held = { project };
  backend.on("project_memories", (args) => (args.dotId === sample.dot.id ? held.project : null));
  return held;
}

const projectItems = () =>
  within(screen.getByRole("list", { name: "Project memories" })).getAllByRole("listitem");

/// The editor of the dot, opened on its Memory tab.
async function openTab() {
  const view = renderApp(`/dots/${sample.dot.id}?tab=memory`);
  expect(await screen.findByRole("tab", { name: "Memory", selected: true })).toBeVisible();
  return view;
}

const items = () => within(screen.getByRole("list", { name: "Memories" })).getAllByRole("listitem");

describe("a sol's Memory tab", () => {
  it("lists what the sol remembers, pinned first, with who wrote each and the count", async () => {
    holding([dots, persons]);
    await openTab();
    await screen.findByRole("list", { name: "Memories" });
    const [first, second] = items();
    expect(first).toHaveAccessibleName(dots.text);
    expect(within(first!).getByRole("button", { name: "Pinned" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(first).toHaveTextContent("by the sol · from run");
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
        "Nothing remembered yet. The sol saves what it learns here; you can add notes too.",
      ),
    ).toBeVisible();
    expect(screen.getByText("0 of 200")).toBeVisible();
  });

  it("adds a memory with a counter of its 500 characters, and shows the core's refusal", async () => {
    const held = holding([]);
    let refuse = true;
    backend.on("memory_add", (args) => {
      if (refuse) throw { kind: "conflict", message: "This sol already remembers that." };
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
    expect(await screen.findByRole("alert")).toHaveTextContent("This sol already remembers that.");
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

  it("refreshes when the core says the sol's memories changed", async () => {
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
      throw { kind: "conflict", message: "This sol already remembers that." };
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
    expect(box).toHaveAccessibleDescription("6 / 500 This sol already remembers that.");
  });

  it("offers no Add memory once the sol remembers 200 things", async () => {
    holding(Array.from({ length: 200 }, (_, i) => ({ ...persons, id: `01J0M${i}`, text: `Fact ${i}.` })));
    await openTab();
    expect(await screen.findByText("200 of 200")).toBeVisible();
    expect(screen.getByRole("button", { name: "Add memory" })).toBeDisabled();
  });

  it("opens from the sol's menu", async () => {
    home([]);
    holding([persons]);
    const { user, router } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("button", { name: dot.name }));
    await user.click(await screen.findByRole("menuitem", { name: "Memory…" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/dots/${dot.id}`));
    expect(router.state.location.search).toBe("?tab=memory");
    expect(await screen.findByRole("tab", { name: "Memory", selected: true })).toBeVisible();
    expect(await screen.findByRole("listitem", { name: persons.text })).toBeVisible();
  });
});

describe("a sol's Memory tab, by kind", () => {
  const core = { ...persons, id: "01J0C", text: "The person wants short answers.", kind: "core" as const };
  const how = { ...persons, id: "01J0H", text: "Restart with pm2 restart api.", kind: "procedural" as const };

  it("groups the memories by kind, under a heading each", async () => {
    // As the core lists them: newest first, whatever the kind.
    holding([how, persons, core]);
    await openTab();
    await screen.findByRole("list", { name: "Memories" });
    expect(items().map((i) => i.getAttribute("aria-label"))).toEqual([
      core.text,
      persons.text,
      how.text,
    ]);
    const list = screen.getByRole("list", { name: "Memories" });
    expect(within(list).getAllByRole("heading").map((h) => h.textContent)).toEqual([
      "Core",
      "Facts and decisions",
      "How-to",
    ]);
  });

  it("adds a core memory, which is at most 160 characters", async () => {
    const held = holding([]);
    backend.on("memory_add", (args) => {
      const saved = { ...core, id: "01J0NEW", text: String(args.text).trim() };
      held.memories = [saved];
      return saved;
    });
    const { user } = await openTab();
    await user.click(await screen.findByRole("button", { name: "Add memory" }));
    const box = screen.getByRole("textbox", { name: "New memory" });
    expect(screen.getByRole("combobox", { name: "Kind" })).toHaveValue("semantic");
    await user.selectOptions(screen.getByRole("combobox", { name: "Kind" }), "core");
    expect(screen.getByText("0 / 160")).toBeVisible();
    const save = within(screen.getByRole("group", { name: "New memory" })).getByRole("button", {
      name: "Save",
    });
    fireEvent.change(box, { target: { value: "x".repeat(161) } });
    expect(screen.getByText("161 / 160")).toBeVisible();
    expect(save).toBeDisabled();
    fireEvent.change(box, { target: { value: "The shop is built with Next.js." } });
    await user.click(save);
    expect(await screen.findByRole("listitem", { name: "The shop is built with Next.js." })).toBeVisible();
    expect(backend.callsTo("memory_add")).toEqual([
      { dotId: sample.dot.id, text: "The shop is built with Next.js.", kind: "core" },
    ]);
  });

  it("changes a memory's kind in its box", async () => {
    const held = holding([persons]);
    backend.on("memory_update", (args) => {
      const next = { ...persons, kind: args.kind as Memory["kind"] };
      held.memories = [next];
      return next;
    });
    const { user } = await openTab();
    const item = await screen.findByRole("listitem", { name: persons.text });
    await user.click(within(item).getByRole("button", { name: "Edit" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Kind" }), "procedural");
    await user.click(
      within(screen.getByRole("group", { name: "Edit memory" })).getByRole("button", { name: "Save" }),
    );
    expect(await screen.findByRole("heading", { name: "How-to" })).toBeVisible();
    expect(backend.callsTo("memory_update")).toEqual([
      { id: persons.id, text: persons.text, pinned: null, kind: "procedural" },
    ]);
  });

  it("shows a memory's text as plain text", async () => {
    const markup = '<img src="x" onerror="alert(1)"> <a href="https://example.com">here</a> [go](javascript:alert(1))';
    holding([{ ...persons, text: markup }]);
    inProject({ ...sample.projectMemories, memories: [{ ...howTo, text: "<b>bold</b> <script>x()</script>" }] });
    await openTab();
    const item = await screen.findByRole("listitem", { name: markup });
    expect(item).toHaveTextContent(markup);
    const tab = screen.getByRole("tabpanel");
    expect(tab.querySelector("img, script, b")).toBeNull();
    // The only links are the ones the tab makes itself, to runs.
    for (const link of within(tab).queryAllByRole("link")) {
      expect(link.getAttribute("href")).toMatch(/^\/runs\//);
    }
    expect(await screen.findByRole("listitem", { name: "<b>bold</b> <script>x()</script>" })).toBeVisible();
  });
});

describe("the project's memory in a sol's Memory tab", () => {
  it("says so when the sol has no project", async () => {
    holding([persons]);
    await openTab();
    expect(
      await screen.findByText("This sol's folder is missing, so it has no project memory."),
    ).toBeVisible();
    expect(screen.queryByRole("list", { name: "Project memories" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open folder" })).toBeNull();
  });

  it("lists the project's memories by kind, with their files, their folder and what was skipped", async () => {
    holding([persons]);
    inProject(sample.projectMemories);
    const { user } = await openTab();
    await screen.findByRole("list", { name: "Project memories" });
    const [first, second] = projectItems();
    expect(first).toHaveAccessibleName(howTo.text);
    expect(first).toHaveTextContent("About ui/package.json, ui/vitest.config.ts");
    expect(first).toHaveTextContent("by a sol · from run");
    expect(second).toHaveAccessibleName(episode.text);
    expect(second).toHaveTextContent("saved by Sage after run");
    const list = screen.getByRole("list", { name: "Project memories" });
    expect(within(list).getAllByRole("heading").map((h) => h.textContent)).toEqual([
      "How-to",
      "What runs did",
    ]);
    expect(screen.getByText("2 of 300")).toBeVisible();
    expect(screen.getByText(sample.projectMemories.dir)).toBeVisible();
    expect(screen.getByText("1 file in the folder is not a memory Sage can read.")).toBeVisible();
    expect(screen.getByText(/Commit \.sage\/memory to share them/)).toBeVisible();
    // The sol's own list is still there, apart.
    expect(items().map((i) => i.getAttribute("aria-label"))).toEqual([persons.text]);

    backend.on("open_memory_folder", () => null);
    await user.click(screen.getByRole("button", { name: "Open folder" }));
    await waitFor(() =>
      expect(backend.callsTo("open_memory_folder")).toEqual([{ dotId: sample.dot.id }]),
    );
  });

  it("adds, edits, pins and deletes a project memory through the sol", async () => {
    holding([]);
    const held = inProject({ ...sample.projectMemories, memories: [howTo], skipped: 0 });
    const put = (next: Memory) => {
      const others = held.project.memories.filter((m) => m.id !== next.id);
      held.project = { ...held.project, memories: [next, ...others] };
      return next;
    };
    backend.on("project_memory_add", (args) =>
      put({ ...howTo, id: "01J0PNEW", text: String(args.text), kind: args.kind as Memory["kind"], source: "person", run_id: null, files: [] }),
    );
    backend.on("project_memory_update", (args) => {
      const now = held.project.memories.find((m) => m.id === args.id)!;
      return put({
        ...now,
        text: (args.text as string | null) ?? now.text,
        pinned: (args.pinned as boolean | null) ?? now.pinned,
      });
    });
    backend.on("project_memory_delete", (args) => {
      held.project = {
        ...held.project,
        memories: held.project.memories.filter((m) => m.id !== args.id),
      };
      return null;
    });
    const { user } = await openTab();
    await user.click(await screen.findByRole("button", { name: "Add project memory" }));
    await user.type(screen.getByRole("textbox", { name: "New memory" }), "We deploy on Fridays.");
    await user.click(
      within(screen.getByRole("group", { name: "New memory" })).getByRole("button", { name: "Save" }),
    );
    const added = await screen.findByRole("listitem", { name: "We deploy on Fridays." });
    expect(added).toHaveTextContent("by you");

    await user.click(within(added).getByRole("button", { name: "Edit" }));
    const box = screen.getByRole("textbox", { name: "Edit memory" });
    await user.clear(box);
    await user.type(box, "We deploy on Mondays.");
    await user.click(
      within(screen.getByRole("group", { name: "Edit memory" })).getByRole("button", { name: "Save" }),
    );
    const edited = await screen.findByRole("listitem", { name: "We deploy on Mondays." });
    await user.click(within(edited).getByRole("button", { name: "Pinned" }));
    await waitFor(() =>
      expect(
        within(screen.getByRole("listitem", { name: "We deploy on Mondays." })).getByRole("button", {
          name: "Pinned",
        }),
      ).toHaveAttribute("aria-pressed", "true"),
    );
    await user.click(
      within(screen.getByRole("listitem", { name: howTo.text })).getByRole("button", { name: "Delete" }),
    );
    await user.click(
      within(await screen.findByRole("alertdialog", { name: "Delete this memory?" })).getByRole(
        "button",
        { name: "Delete" },
      ),
    );
    await waitFor(() => expect(screen.queryByRole("listitem", { name: howTo.text })).toBeNull());
    expect(backend.callsTo("project_memory_add")).toEqual([
      { dotId: sample.dot.id, text: "We deploy on Fridays.", kind: "semantic", files: [] },
    ]);
    expect(backend.callsTo("project_memory_update")).toEqual([
      { dotId: sample.dot.id, id: "01J0PNEW", text: "We deploy on Mondays.", pinned: null, kind: null },
      { dotId: sample.dot.id, id: "01J0PNEW", text: null, pinned: true, kind: null },
    ]);
    expect(backend.callsTo("project_memory_delete")).toEqual([{ dotId: sample.dot.id, id: howTo.id }]);
    // Nothing of it went to the sol's own memory.
    expect(backend.callsTo("memory_add")).toEqual([]);
  });

  it("shows the core's refusal under the box, and an empty project's own words", async () => {
    holding([]);
    inProject({ ...sample.projectMemories, memories: [], skipped: 0 });
    backend.on("project_memory_add", () => {
      throw { kind: "invalid", message: "That looks like a secret. Don't store secrets in memory; ask the person to keep it in the vault." };
    });
    const { user } = await openTab();
    expect(await screen.findByText(/Nothing remembered for this project yet\./)).toBeVisible();
    expect(screen.getByText("0 of 300")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Add project memory" }));
    await user.type(screen.getByRole("textbox", { name: "New memory" }), "password: hunter2hunter2");
    await user.click(
      within(screen.getByRole("group", { name: "New memory" })).getByRole("button", { name: "Save" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("That looks like a secret.");
  });

  it("refreshes when the core says a project's memory changed", async () => {
    holding([]);
    const held = inProject({ ...sample.projectMemories, memories: [howTo] });
    const { queryClient } = await openTab();
    await screen.findByRole("listitem", { name: howTo.text });
    held.project = { ...held.project, memories: [howTo, episode] };
    act(() =>
      applyRuntimeEvent(queryClient, { type: "project_memories_updated", root: held.project.root }),
    );
    expect(await screen.findByRole("listitem", { name: episode.text })).toBeVisible();
    // It leaves the sol's own list alone.
    expect(backend.callsTo("memories_list")).toHaveLength(1);
  });
});

import { describe, expect, it } from "vitest";

import { commands } from "@/api/commands";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

describe("the memory commands", () => {
  it("list, add, change and delete a dot's memories as the core names them", async () => {
    const [pinned, persons] = sample.memories;
    backend.on("memories_list", () => sample.memories);
    backend.on("memory_add", () => persons);
    backend.on("memory_update", () => pinned);
    backend.on("memory_delete", () => null);
    expect(await commands.memoriesList("dot-a")).toEqual(sample.memories);
    expect(await commands.memoryAdd("dot-a", "A note.")).toEqual(persons);
    expect(await commands.memoryUpdate("m1", null, true)).toEqual(pinned);
    await commands.memoryUpdate("m1", "New text.", null);
    expect(await commands.memoryDelete("m1")).toBeNull();
    expect(backend.calls).toEqual([
      ["memories_list", { dotId: "dot-a" }],
      ["memory_add", { dotId: "dot-a", text: "A note." }],
      ["memory_update", { id: "m1", text: null, pinned: true }],
      ["memory_update", { id: "m1", text: "New text.", pinned: null }],
      ["memory_delete", { id: "m1" }],
    ]);
  });

  it("rejects with the core's sentence", async () => {
    backend.on("memory_add", () => {
      throw { kind: "conflict", message: "This dot already remembers that." };
    });
    await expect(commands.memoryAdd("dot-a", "Again.")).rejects.toMatchObject({
      kind: "conflict",
      message: "This dot already remembers that.",
    });
  });
});

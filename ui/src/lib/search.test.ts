import { describe, expect, it } from "vitest";

import type { SearchHit } from "@/api/types";
import { groupHits, HIT_END, HIT_START, hitPath, snippetParts } from "@/lib/search";

const hit = (over: Partial<SearchHit>): SearchHit => ({
  kind: "chat",
  dot_id: "d1",
  dot_name: "planner",
  target: { type: "topic", dot_id: "d1", root_run_id: "r1", run_id: "r2" },
  at: "2026-11-01T09:00:00.000Z",
  snippet: "x",
  ...over,
});

describe("search snippets", () => {
  it("splits a snippet into plain text and matched words", () => {
    expect(snippetParts(`Plan the ${HIT_START}Lisbon${HIT_END} trip to ${HIT_START}Lis${HIT_END}`)).toEqual([
      { text: "Plan the ", hit: false },
      { text: "Lisbon", hit: true },
      { text: " trip to ", hit: false },
      { text: "Lis", hit: true },
    ]);
  });

  it("keeps markup as text and survives a mark with no end", () => {
    expect(snippetParts(`<b>x</b> ${HIT_START}y`)).toEqual([
      { text: "<b>x</b> ", hit: false },
      { text: "y", hit: true },
    ]);
    expect(snippetParts("")).toEqual([]);
  });
});

describe("where a hit opens", () => {
  it("opens a chat topic, a run, a sol, a memory or a goal", () => {
    expect(hitPath(hit({}))).toBe("/home/d1/r1");
    expect(hitPath(hit({ target: { type: "run", run_id: "r 9" } }))).toBe("/runs/r%209");
    expect(hitPath(hit({ target: { type: "dot", dot_id: "d1" } }))).toBe("/home/d1");
    expect(hitPath(hit({ target: { type: "memory", dot_id: "d1", memory_id: "m1" } }))).toBe(
      "/dots/d1?tab=memory",
    );
    expect(hitPath(hit({ target: { type: "intent", intent_id: "g1" } }))).toBe("/intents");
  });
});

describe("grouping hits", () => {
  it("groups hits by kind in a fixed order, keeping their rank within a group", () => {
    const a = hit({ kind: "memory", snippet: "a" });
    const b = hit({ kind: "chat", snippet: "b" });
    const c = hit({ kind: "memory", snippet: "c" });
    const groups = groupHits([a, b, c]);
    expect(groups.map((g) => g.kind)).toEqual(["chat", "memory"]);
    expect(groups[1]!.hits).toEqual([a, c]);
    expect(groups[0]!.label).toBe("You said");
  });
});

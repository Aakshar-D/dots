import { describe, expect, it } from "vitest";

import { sample } from "@/test/fixtures";

import { diffTools, isReadOnly, parseTools } from "./tools";
import { looksSecret, parseLines, variablesIn } from "./text";

const tool = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  description: `${name} tool`,
  inputSchema: { type: "object" },
  ...extra,
});

describe("a tool list", () => {
  it("is read from the JSON the core keeps, and is empty when it is not a list of tools", () => {
    expect(parseTools([tool("a"), tool("b")]).map((t) => t.name)).toEqual(["a", "b"]);
    expect(parseTools(null)).toEqual([]);
    expect(parseTools({ tools: [] })).toEqual([]);
    expect(parseTools([{ nope: 1 }])).toEqual([]);
    const seen = sample.library[0]!.catalog!.seen_tools;
    expect(parseTools(seen)[0]).toMatchObject({ name: "search_issues" });
  });

  it("calls a tool read-only only when the server says so with a true hint", () => {
    expect(isReadOnly(tool("a", { annotations: { readOnlyHint: true } }))).toBe(true);
    expect(isReadOnly(tool("a", { annotations: { readOnlyHint: "yes" } }))).toBe(false);
    expect(isReadOnly(tool("a", { annotations: null }))).toBe(false);
    expect(isReadOnly(tool("a"))).toBe(false);
  });

  it("tells what was added, removed and changed, and which part of a changed tool", () => {
    const accepted = parseTools([
      tool("kept"),
      tool("gone"),
      tool("words"),
      tool("shape", { inputSchema: { type: "object", properties: { a: {} } } }),
      tool("hint", { annotations: { readOnlyHint: true } }),
    ]);
    const seen = parseTools([
      tool("kept", { title: "only the title differs" }),
      tool("words", { description: "other words" }),
      // The same schema with its keys in another order is the same schema.
      tool("shape", { inputSchema: { properties: { a: {} }, type: "object" } }),
      tool("hint", { annotations: { readOnlyHint: false } }),
      tool("new"),
    ]);
    const diff = diffTools(accepted, seen);
    expect(diff.added.map((t) => t.name)).toEqual(["new"]);
    expect(diff.removed.map((t) => t.name)).toEqual(["gone"]);
    expect(diff.changed.map((c) => [c.tool.name, c.parts])).toEqual([
      ["words", ["description"]],
      ["hint", ["annotations"]],
    ]);
    const inputs = diffTools(
      parseTools([tool("t")]),
      parseTools([tool("t", { inputSchema: { type: "string" } })]),
    );
    expect(inputs.changed.map((c) => c.parts)).toEqual([["inputs"]]);
  });
});

describe("the text of the server form", () => {
  it("reads one KEY=VALUE or Name: value per line, and says which line is wrong", () => {
    expect(parseLines("A=1\n\n B = x=y \n", "=")).toEqual({
      ok: true,
      entries: { A: "1", B: "x=y" },
    });
    expect(parseLines("Authorization: Bearer ${T}", ":")).toEqual({
      ok: true,
      entries: { Authorization: "Bearer ${T}" },
    });
    expect(parseLines("fine=1\nbroken", "=")).toEqual({ ok: false, line: 2 });
    expect(parseLines("=empty", "=")).toEqual({ ok: false, line: 1 });
  });

  it("finds the variables a config reads, once each", () => {
    expect(variablesIn(["${A}/x", "no", "${B}${A}", "$C", "${1X}"])).toEqual(["A", "B"]);
  });

  it("flags a key that looks like a secret", () => {
    for (const key of ["API_TOKEN", "secret", "MyKey", "PASSWORD", "Authorization"]) {
      expect(looksSecret(key)).toBe(true);
    }
    expect(looksSecret("MODE")).toBe(false);
  });
});

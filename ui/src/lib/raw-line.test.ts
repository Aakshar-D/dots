import { describe, expect, it } from "vitest";

import { rawSummary, rawView, thinkingBars, tokenCount } from "@/lib/raw-line";

const engine = (text: string) => ({ label: "Engine output", text, engine: true });
const thinking = (delta: number) =>
  engine(
    JSON.stringify({
      type: "system",
      subtype: "thinking_tokens",
      estimated_tokens: 1000 + delta,
      estimated_tokens_delta: delta,
      session_id: "s",
      uuid: "u",
    }),
  );

describe("rawView", () => {
  it("reads the engine's thinking progress as the tokens since its last count", () => {
    expect(rawView(thinking(333))).toEqual({ kind: "thinking", tokens: 333 });
  });

  it("counts a thinking line without a count since the last for nothing", () => {
    const line = engine('{"type":"system","subtype":"thinking_tokens","estimated_tokens":900}');
    expect(rawView(line)).toEqual({ kind: "thinking", tokens: 0 });
    expect(rawSummary([rawView(line)])).toBe("Thought it through");
  });

  it("titles an engine line by its type and leaves its ids out", () => {
    const line = engine(
      JSON.stringify({
        type: "system",
        subtype: "hook_started",
        hook_name: "PreToolUse",
        session_id: "s",
        uuid: "u",
        detail: { a: 1 },
      }),
    );
    expect(rawView(line)).toEqual({
      kind: "fields",
      title: "System · hook started",
      fields: [
        { key: "hook name", value: "PreToolUse" },
        { key: "detail", value: '{\n  "a": 1\n}' },
      ],
    });
  });

  it("keeps a type that is not a word among the fields, and reads odd values", () => {
    const line = engine('{"type":5,"subtype":"later","gone":null,"list":[1,"a"]}');
    expect(rawView(line)).toEqual({
      kind: "fields",
      title: "Later",
      fields: [
        { key: "type", value: "5" },
        { key: "gone", value: "null" },
        { key: "list", value: JSON.stringify([1, "a"], null, 2) },
      ],
    });
    expect(rawView(engine("{}"))).toEqual({ kind: "fields", title: "Engine output", fields: [] });
  });

  it("keeps text that is not a JSON object as text", () => {
    expect(rawView(engine("garbled"))).toEqual({
      kind: "text",
      title: "Engine output",
      text: "garbled",
    });
    expect(rawView(engine("[1, 2]")).kind).toBe("text");
  });

  it("shows what the core stored under its own label, with every field", () => {
    const line = { label: "Unknown event (later)", text: '{"kind":"later","n":1}', engine: false };
    expect(rawView(line)).toEqual({
      kind: "fields",
      title: "Unknown event (later)",
      fields: [
        { key: "kind", value: "later" },
        { key: "n", value: "1" },
      ],
    });
  });
});

describe("rawSummary", () => {
  it("adds up the thinking of a group", () => {
    const views = [thinking(50), thinking(333)].map(rawView);
    expect(rawSummary(views)).toBe("Thought it through · about 383 tokens");
  });

  it("names the one other line, and counts several", () => {
    const hook = engine('{"type":"system","subtype":"hook_started"}');
    expect(rawSummary([rawView(thinking(50)), rawView(hook)])).toBe(
      "Thought it through · about 50 tokens · System · hook started",
    );
    expect(rawSummary([hook, hook].map(rawView))).toBe("System · hook started × 2");
    expect(rawSummary([hook, engine("garbled")].map(rawView))).toBe("2 raw lines");
  });
});

describe("tokenCount", () => {
  it("writes thousands as k", () => {
    expect(tokenCount(383)).toBe("383");
    expect(tokenCount(1240)).toBe("1.2k");
    expect(tokenCount(2000)).toBe("2k");
  });
});

describe("thinkingBars", () => {
  it("draws a bar per count, and adds neighbours up past 24 counts", () => {
    expect(thinkingBars([50, 333])).toEqual([50, 333]);
    const bars = thinkingBars(new Array<number>(240).fill(1));
    expect(bars).toHaveLength(24);
    expect(bars.every((bar) => bar === 10)).toBe(true);
  });
});

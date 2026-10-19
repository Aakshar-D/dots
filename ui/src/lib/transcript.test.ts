import { describe, expect, it } from "vitest";

import type { RunEventRecord } from "@/api/types";
import { toolPreview, transcriptItems } from "@/lib/transcript";

let seq = 0;
const ev = (data: Record<string, unknown>): RunEventRecord => ({
  run_id: "r1",
  seq: ++seq,
  ts: "2026-10-09T12:00:00.000Z",
  kind: String(data.kind),
  data,
});

describe("the transcript", () => {
  it("pairs a tool call with its result and keeps the order of the conversation", () => {
    const items = transcriptItems([
      ev({ kind: "session_started", session_id: "s" }),
      ev({ kind: "assistant_text", text: "Looking." }),
      ev({ kind: "tool_call", id: "a", tool: "Read", input: { file_path: "a.rs" } }),
      ev({ kind: "tool_call", id: "b", tool: "Bash", input: { command: "cargo test" } }),
      ev({ kind: "tool_result", id: "b", output: "ok", is_error: false }),
      ev({ kind: "tool_result", id: "a", output: "denied", is_error: true }),
      ev({ kind: "usage", tokens_in: 1, tokens_out: 2 }),
      ev({ kind: "assistant_text", text: "Done." }),
      ev({ kind: "finished", summary: "Done." }),
    ]);
    expect(items.map((i) => i.type)).toEqual(["text", "tool", "tool", "text"]);
    expect(items[1]).toMatchObject({ tool: "Read", result: { output: "denied", isError: true } });
    expect(items[2]).toMatchObject({ tool: "Bash", result: { output: "ok", isError: false } });
  });

  it("shows a call that has no result yet as pending", () => {
    const [call] = transcriptItems([ev({ kind: "tool_call", id: "a", tool: "Read", input: {} })]);
    expect(call).toMatchObject({ type: "tool", result: null });
  });

  it("hides the local engine's message records and keeps failures", () => {
    const items = transcriptItems([
      ev({ kind: "message", message: { role: "user", content: "hi" } }),
      ev({ kind: "failed", error: "timeout" }),
    ]);
    expect(items).toEqual([{ type: "error", seq: expect.any(Number), text: "timeout" }]);
  });

  it("shows a notice about the run as a note", () => {
    const items = transcriptItems([ev({ kind: "notice", text: "skill pdf: folder not found" })]);
    expect(items).toEqual([
      { type: "note", seq: expect.any(Number), text: "skill pdf: folder not found" },
    ]);
  });

  it("keeps what it cannot read as raw instead of dropping it", () => {
    const items = transcriptItems([
      ev({ kind: "raw", line: '{"type":"future"}' }),
      ev({ kind: "from_a_newer_core", payload: 1 }),
      ev({ kind: "tool_result", id: "nobody", output: "orphan", is_error: false }),
      ev({ kind: "assistant_text", text: 5 }),
    ]);
    expect(items.map((i) => (i.type === "raw" ? i.label : i.type))).toEqual([
      "Unparsed output",
      "Unknown event (from_a_newer_core)",
      "Tool result",
      "Unknown event (assistant_text)",
    ]);
  });

  it("previews a tool call by its most telling argument", () => {
    expect(toolPreview({ command: "git push", description: "x" })).toBe("git push");
    expect(toolPreview({ file_path: "src/a.rs", content: "…" })).toBe("src/a.rs");
    expect(toolPreview({ pattern: "TODO", path: "src" })).toBe("src");
    expect(toolPreview({ n: 1 })).toBe('{"n":1}');
    expect(toolPreview({ text: "x".repeat(200) })).toHaveLength(81);
    expect(toolPreview(undefined)).toBe("");
  });
});

import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { RunEventRecord } from "@/api/types";
import { memoryNote } from "@/lib/transcript";
import { sample } from "@/test/fixtures";
import { chat, dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const event = (seq: number, data: Record<string, unknown>): RunEventRecord => ({
  run_id: "a",
  seq,
  ts: sample.run.queued_at,
  kind: String(data.kind),
  data,
});

/// A call of `tool` with `input` and its result.
const step = (seq: number, tool: string, input: unknown, output: string, isError = false) => [
  event(seq, { kind: "tool_call", id: `t${seq}`, tool, input }),
  event(seq + 1, { kind: "tool_result", id: `t${seq}`, output, is_error: isError }),
];

describe("a memory tool step in the conversation", () => {
  it("says what the dot remembered or that it forgot one, under the chip, with a link", async () => {
    home(
      [chat("a", "10:00")],
      [
        ...step(1, "Memory", { text: "Prefers invoices as PDF." }, "Remembered 01J0MEM1."),
        ...step(3, "mcp__dots__forget", { id: "01J0MEM2" }, "Forgot 01J0MEM2."),
        ...step(5, "mcp__dots__recall", { query: "pdf" }, "[01J0MEM1] Prefers invoices as PDF."),
        ...step(7, "Memory", { text: "Again." }, "This dot already remembers that.", true),
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const remembered = await view.findByText("Remembered: Prefers invoices as PDF.");
    const forgot = view.getByText("Forgot a memory");
    // The notes show while the steps line is closed.
    expect(remembered).toBeVisible();
    expect(forgot).toBeVisible();
    // Each chip shows as every tool's does; the notes follow the row of chips.
    await user.click(
      view.getByText("Used Memory › remember 2 times · Used dots › forget · Used dots › recall"),
    );
    const chips = view.getByRole("group", { name: "Tool steps" });
    expect(within(chips).getAllByText("done")).toHaveLength(3);
    expect(within(chips).getByText("failed")).toBeVisible();
    expect(
      chips.compareDocumentPosition(remembered) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    for (const note of [remembered, forgot]) {
      expect(within(note.parentElement!).getByRole("link", { name: "Memory" })).toHaveAttribute(
        "href",
        `/dots/${dot.id}?tab=memory`,
      );
    }
    // `recall` and a refused `remember` are plain chips.
    expect(view.getAllByRole("link", { name: "Memory" })).toHaveLength(2);
    expect(view.queryByText("Remembered: Again.")).toBeNull();
  });

  it("tells the tools apart by name or, for the local engine, by input", () => {
    const done = { output: "ok", isError: false };
    expect(memoryNote("Memory", { text: " A fact. " }, done)).toEqual({
      kind: "remembered",
      text: "A fact.",
    });
    expect(memoryNote("mcp__dots__remember", { text: "A fact." }, done)).toEqual({
      kind: "remembered",
      text: "A fact.",
    });
    expect(memoryNote("Memory", { id: "x" }, done)).toEqual({ kind: "forgot" });
    expect(memoryNote("Memory", { query: "x" }, done)).toBeNull();
    expect(memoryNote("mcp__dots__recall", {}, done)).toBeNull();
    expect(memoryNote("Bash", { text: "x" }, done)).toBeNull();
    // Not done yet, refused, or parked for approval.
    expect(memoryNote("Memory", { text: "x" }, null)).toBeNull();
    expect(
      memoryNote("Memory", { text: "x" }, { output: "Queued for human approval #1.", isError: true }),
    ).toBeNull();
  });
  it("names a local memory call's chip by the tool it ran", async () => {
    home(
      [chat("a", "10:00")],
      [
        ...step(1, "Memory", { text: "Prefers PDF." }, "Remembered 01J0MEM1."),
        ...step(3, "Memory", { id: "01J0MEM1" }, "Forgot 01J0MEM1."),
        ...step(5, "Memory", { query: "pdf" }, "Nothing remembered about that."),
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const line = await view.findByText(
      "Used Memory › remember · Used Memory › forget · Used Memory › recall",
    );
    await user.click(line);
    const chips = view.getByRole("group", { name: "Tool steps" });
    for (const name of ["Memory › remember", "Memory › forget", "Memory › recall"]) {
      expect(within(chips).getByText(name)).toBeVisible();
    }
  });
});

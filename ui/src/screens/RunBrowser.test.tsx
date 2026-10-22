import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Run, RunEventRecord } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const run: Run = { ...sample.run, branch: null, base_commit: null };
let seq = 0;
const ev = (data: Record<string, unknown>): RunEventRecord => ({
  run_id: run.id,
  seq: ++seq,
  ts: run.queued_at,
  kind: String(data.kind),
  data,
});

describe("browser calls in the run view", () => {
  it("shows each call as one line, and a screenshot as an image", async () => {
    const events = [
      ev({
        kind: "tool_call",
        id: "b1",
        tool: "mcp__browser__browser_navigate",
        input: { url: "https://shop.com/cart" },
      }),
      ev({
        kind: "tool_result",
        id: "b1",
        output: "Dots: Opened shop.com/cart\n### Page\n- Page URL: https://shop.com/cart",
        is_error: false,
      }),
      ev({ kind: "tool_call", id: "b2", tool: "mcp__browser__browser_click", input: { target: "e5" } }),
      ev({
        kind: "tool_result",
        id: "b2",
        output: 'Dots: Clicked "Add to cart"\nDots: Downloaded statement.pdf\n### Page',
        is_error: false,
      }),
      ev({
        kind: "tool_call",
        id: "b3",
        tool: "mcp__browser__browser_type",
        input: { target: "e11", text: "shoes" },
      }),
      ev({ kind: "tool_result", id: "b3", output: 'Dots: Typed into "Search"\n### Page', is_error: false }),
      ev({
        kind: "tool_call",
        id: "b4",
        tool: "mcp__browser__browser_take_screenshot",
        input: { scale: "css" },
      }),
      ev({
        kind: "tool_result",
        id: "b4",
        output:
          "Dots: Took a screenshot\n### Result\n- [Screenshot of viewport](downloads\\page-2026-10-21T12-16-35-575Z.png)",
        is_error: false,
      }),
    ];
    backend.on("run_thread", () => [run]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", (args) =>
      events.filter((e) => e.seq > (args.afterSeq as number)),
    );
    backend.on("approvals_for_run", () => []);
    backend.on("worktree_diff", () => sample.diff);
    backend.on("browser_screenshot", () => "data:image/png;base64,iVBORw==");
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("Opened shop.com/cart")).toBeVisible();
    expect(screen.getByText('Clicked "Add to cart" · Downloaded statement.pdf')).toBeVisible();
    expect(screen.getByText('Typed into "Search"')).toBeVisible();
    expect(screen.getByText("Took a screenshot")).toBeVisible();
    const image = await screen.findByRole("img", { name: "Screenshot of the dot's browser" });
    expect(image).toHaveAttribute("src", "data:image/png;base64,iVBORw==");
    expect(backend.callsTo("browser_screenshot")).toEqual([
      { runId: run.id, name: "page-2026-10-21T12-16-35-575Z.png" },
    ]);
    // Not as raw tool calls.
    expect(screen.queryByText("browser › browser_click")).toBeNull();
  });
});

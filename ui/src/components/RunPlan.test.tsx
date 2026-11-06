import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type { Run, RunEventRecord } from "@/api/types";
import { RunPlan } from "@/components/RunPlan";
import { sample } from "@/test/fixtures";

const running: Run = {
  ...sample.run,
  status: "running",
  started_at: "2026-11-06T10:00:00.000Z",
  ended_at: null,
};
const ended: Run = { ...running, status: "succeeded", ended_at: "2026-11-06T10:04:12.000Z" };

let seq = 0;
const ev = (data: Record<string, unknown>): RunEventRecord => ({
  run_id: running.id,
  seq: ++seq,
  ts: "2026-11-06T10:01:00.000Z",
  kind: String(data.kind),
  data,
});
const todos = (...steps: [string, string][]) =>
  steps.map(([content, status]) => ({ content, status, activeForm: `Doing ${content}` }));
const plan = (id: string, steps: [string, string][], parent?: string) =>
  ev({
    kind: "tool_call",
    id,
    tool: "TodoWrite",
    input: { todos: todos(...steps) },
    ...(parent ? { parent } : {}),
  });

const show = (run: Run, events: RunEventRecord[]) => (
  <RunPlan run={run} events={events} dotName="Scout" owners={new Map()} />
);
const card = () => screen.getByRole("region", { name: /of Scout$/ });

describe("the plan card", () => {
  it("shows the dot's steps, the one it works on, and how far it is", () => {
    render(
      show(running, [
        plan("p1", [
          ["Read the code", "completed"],
          ["Fix the bug", "in_progress"],
          ["Run the tests", "pending"],
        ]),
      ]),
    );
    const plan_ = within(card()).getByRole("group", { name: "Plan of Scout" });
    const steps = within(plan_).getAllByRole("listitem");
    expect(steps.map((s) => s.textContent)).toEqual([
      "✓Read the code: Done",
      "Doing Fix the bug…: Working on itNow",
      "○Run the tests: To do",
    ]);
    // The state is in words, not only in a colour or a mark.
    expect(within(plan_).getByText(": Working on it")).toHaveClass("sr-only");
    expect(within(card()).getAllByText("1 of 3 done").length).toBeGreaterThan(0);
    expect(within(card()).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "1");
    // The spinner holds still with reduced motion.
    expect(card().querySelector(".animate-spin")).toHaveClass("motion-reduce:animate-none");
  });

  it("follows the plan as new events arrive", () => {
    const first = plan("p1", [
      ["Read", "in_progress"],
      ["Fix", "pending"],
    ]);
    const { rerender } = render(show(running, [first]));
    expect(screen.getByText("0 of 2 steps done")).toBeInTheDocument();
    const next = plan("p2", [
      ["Read", "completed"],
      ["Fix", "in_progress"],
    ]);
    rerender(show(running, [first, next]));
    expect(screen.getByText("1 of 2 steps done")).toHaveAttribute("aria-live", "polite");
    expect(screen.getByText("Doing Fix…")).toBeVisible();
  });

  it("closes to one line once the run has ended, and opens again on a click", async () => {
    const user = userEvent.setup();
    render(
      show(ended, [
        plan("p1", [
          ["Read", "completed"],
          ["Fix", "in_progress"],
        ]),
      ]),
    );
    const toggle = within(card()).getByRole("button", { name: "Plan" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(within(card()).getByText("Took 4m 12s")).toBeVisible();
    expect(screen.queryByRole("group", { name: "Plan of Scout" })).toBeNull();
    await user.click(toggle);
    const steps = within(screen.getByRole("group", { name: "Plan of Scout" })).getAllByRole(
      "listitem",
    );
    // A step left half way in an ended run stopped there.
    expect(steps[1]).toHaveTextContent("Fix: Stopped");
  });

  it("nests a helper's plan under the dot's, named by what it was asked", () => {
    render(
      show(running, [
        plan("p1", [["Plan the work", "in_progress"]]),
        ev({ kind: "tool_call", id: "t1", tool: "Task", input: { description: "Check the logs" } }),
        plan("h1", [["Open the logs", "completed"]], "t1"),
      ]),
    );
    const helper = screen.getByRole("group", { name: "Plan of Helper: Check the logs" });
    expect(within(helper).getByText("Open the logs")).toBeVisible();
    // The card counts every step.
    expect(within(card()).getAllByText("1 of 2 done").length).toBeGreaterThan(0);
  });

  it("shows the timeline when the plan's toggle opens it, with each step's owner", async () => {
    const user = userEvent.setup();
    render(
      show(running, [
        plan("p1", [["Look", "in_progress"]]),
        ev({ kind: "tool_call", id: "t1", tool: "Task", input: { description: "Check the logs" } }),
        ev({ kind: "tool_call", id: "r1", tool: "Read", input: { file_path: "a" }, parent: "t1" }),
      ]),
    );
    const timelineToggle = screen.getByRole("button", { name: "Timeline · 2 steps" });
    expect(screen.queryByRole("list", { name: "Timeline" })).toBeNull();
    await user.click(timelineToggle);
    const timeline = screen.getByRole("list", { name: "Timeline" });
    expect(within(timeline).getByText("Handed to a helper: Check the logs")).toBeVisible();
    expect(within(timeline).getByText("Helper: Check the logs")).toBeVisible();
    expect(within(timeline).getByText("Read a file")).toBeVisible();
  });

  it("shows the timeline alone, closed, for a busy run without a plan, and nothing for a quick one", async () => {
    const user = userEvent.setup();
    const busy = [
      ev({ kind: "tool_call", id: "a", tool: "Read", input: {} }),
      ev({ kind: "tool_result", id: "a", output: "ok", is_error: false }),
      ev({ kind: "assistant_text", text: "Now the tests." }),
      ev({ kind: "tool_call", id: "b", tool: "Bash", input: { command: "cargo test" } }),
      ev({ kind: "tool_call", id: "c", tool: "Bash", input: { command: "cargo fmt" } }),
    ];
    const { unmount } = render(show(running, busy));
    expect(screen.getByRole("region", { name: "Timeline of Scout" })).toBeVisible();
    // Closed, so the transcript's own lines are not in the page twice.
    expect(screen.queryByRole("list", { name: "Timeline" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Timeline" }));
    const timeline = screen.getByRole("list", { name: "Timeline" });
    expect(within(timeline).getByText("Read a file")).toBeVisible();
    expect(within(timeline).getByText("Ran 2 commands")).toBeVisible();
    expect(screen.queryByRole("progressbar")).toBeNull();
    unmount();

    render(show(running, busy.slice(0, 2)));
    expect(screen.queryByRole("region")).toBeNull();
  });
});

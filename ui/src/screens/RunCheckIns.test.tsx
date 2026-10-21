import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Run, RunEventRecord } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const run: Run = { ...sample.run, branch: null, base_commit: null };
let seq = 0;
const ev = (data: Record<string, unknown>, runId = run.id): RunEventRecord => ({
  run_id: runId,
  seq: ++seq,
  ts: run.queued_at,
  kind: String(data.kind),
  data,
});

/// Answers the run view's queries for one thread.
function thread(runs: Run[], events: RunEventRecord[] = []) {
  backend.on("run_thread", () => runs);
  backend.on("get_dot", () => sample.dot);
  backend.on("list_run_events", (args) =>
    events.filter((e) => e.run_id === args.runId && e.seq > (args.afterSeq as number)),
  );
  backend.on("approvals_for_run", () => []);
  backend.on("worktree_diff", () => sample.diff);
}

describe("check-ins in the run view", () => {
  it("shows each check-in call as one readable line, under either engine's name", async () => {
    thread(
      [run],
      [
        ev({ kind: "tool_call", id: "c1", tool: "Notify", input: { text: "Half way there." } }),
        ev({ kind: "tool_result", id: "c1", output: "Sent.", is_error: false }),
        ev({
          kind: "tool_call",
          id: "c2",
          tool: "mcp__dots__ask",
          input: { question: "Ship it?", choices: ["yes", "no"] },
        }),
        ev({ kind: "tool_result", id: "c2", output: "Your question was sent.", is_error: false }),
        ev({
          kind: "tool_call",
          id: "c3",
          tool: "FollowUp",
          input: { note: "Check the build.", in: "2h" },
        }),
        ev({
          kind: "tool_result",
          id: "c3",
          output: "Follow-up 01J0FUP1 scheduled for Tue 21 Oct 09:00.",
          is_error: false,
        }),
        ev({
          kind: "tool_call",
          id: "c4",
          tool: "mcp__dots__follow_up",
          input: { note: "Too soon.", in: "1m" },
        }),
        ev({
          kind: "tool_result",
          id: "c4",
          output: "A follow-up must be at least 5 minutes away.",
          is_error: true,
        }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("Sent a note: Half way there.")).toBeVisible();
    expect(screen.getByText("Asked: Ship it?")).toBeVisible();
    expect(
      screen.getByText("Scheduled a follow-up for Tue 21 Oct 09:00: Check the build."),
    ).toBeVisible();
    const refused = screen.getByText("Scheduled a follow-up: Too soon.").closest("details")!;
    expect(within(refused).getByText("failed")).toBeInTheDocument();
    expect(
      within(refused).getByText("A follow-up must be at least 5 minutes away."),
    ).toBeInTheDocument();
    expect(screen.queryByText("mcp__dots__ask")).toBeNull();
    expect(screen.queryByText("FollowUp")).toBeNull();
  });

  it("says that a follow-up run is one the dot scheduled, with its note", async () => {
    const followUp: Run = {
      ...run,
      trigger: "follow_up",
      payload: {
        message: "Follow-up you scheduled on Mon 20 Oct 09:00: Check the build.",
        follow_up_id: "01J0FUP1",
      },
    };
    thread([followUp]);
    renderApp(`/runs/${followUp.id}`);
    expect(await screen.findByText("Follow-up the dot scheduled")).toBeVisible();
    expect(
      screen.getByText("Follow-up you scheduled on Mon 20 Oct 09:00: Check the build."),
    ).toBeVisible();
  });
});

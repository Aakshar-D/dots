import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { LimitsState, Run } from "@/api/types";
import { RunNumbers } from "@/components/RunNumbers";
import { resetTime } from "@/lib/usage-format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const ended: Run = {
  ...sample.run,
  started_at: "2026-10-26T12:00:00.000Z",
  ended_at: "2026-10-26T12:12:04.000Z",
  tokens_in: 170_000,
  tokens_out: 12_000,
  cost_usd: 1.32,
};

describe("a run's numbers", () => {
  it("say its time, tokens and API-equivalent cost", () => {
    render(<RunNumbers run={ended} />);
    const line = screen.getByText(/tokens/);
    expect(line).toHaveTextContent("12m 04s · 182k tokens (170k in, 12k out) · $1.32 API-eq.");
    expect(within(line).getByText("API-eq.")).toHaveAttribute(
      "title",
      "Your subscription does not bill per run: this is what the run would cost on the API.",
    );
  });

  it("leave out the cost a run has not got, and the time of a run still going", () => {
    render(<RunNumbers run={{ ...ended, cost_usd: null, ended_at: null }} />);
    expect(screen.getByText(/tokens/)).toHaveTextContent(/^182k tokens \(170k in, 12k out\)$/);
  });

  // Self-QA: a held run's card said "0 tokens (0 in, 0 out)" above why it waits.
  it("say nothing for a run that has not started", () => {
    const queued: Run = {
      ...ended,
      status: "queued",
      started_at: null,
      ended_at: null,
      tokens_in: 0,
      tokens_out: 0,
      cost_usd: null,
    };
    const { container } = render(<RunNumbers run={queued} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("a run held for Claude usage", () => {
  const [holding] = sample.limits as [LimitsState];
  const held: Run = {
    ...sample.run,
    id: holding.held_run_ids[0]!,
    status: "queued",
    started_at: null,
    ended_at: null,
    tokens_in: 0,
    tokens_out: 0,
    cost_usd: null,
  };

  it("says why it waits on its page, and Run anyway releases it", async () => {
    backend.on("limits_get", () => holding);
    backend.on("run_thread", () => [held]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    backend.on("approvals_for_run", () => []);
    backend.on("worktree_diff", () => sample.diff);
    backend.on("run_release_hold", () => ({ ...held, hold_released: true }));
    const { user } = renderApp(`/runs/${held.id}`);
    expect(
      await screen.findByText(`Held: Claude 5-hour at 64%, starts after ${resetTime(holding.hold_until!)}`),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Run anyway" }));
    expect(backend.callsTo("run_release_hold")).toEqual([{ runId: held.id }]);
  });

  it("offers no Run anyway while Claude's limit is reached", async () => {
    const until = holding.hold_until!;
    backend.on("limits_get", () => ({ ...holding, holding: false, rejected_until: until }));
    backend.on("run_thread", () => [held]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    backend.on("approvals_for_run", () => []);
    backend.on("worktree_diff", () => sample.diff);
    renderApp(`/runs/${held.id}`);
    expect(
      await screen.findByText(`Held: Claude's limit is reached, starts after ${resetTime(until)}`),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Run anyway" })).toBeNull();
  });
});

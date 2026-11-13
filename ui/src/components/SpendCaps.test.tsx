import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { specOf } from "@/lib/dot-rules";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// The run page of `run`, with nothing else to show.
function runPage(run: Run) {
  backend.on("run_thread", () => [run]);
  backend.on("get_dot", () => sample.dot);
  backend.on("list_run_events", () => []);
  backend.on("approvals_for_run", () => []);
  backend.on("worktree_diff", () => sample.diff);
  return renderApp(`/runs/${run.id}`);
}

describe("a run waiting for a spending cap", () => {
  it("names the cap, and offers Raise cap and Run once anyway for a chat", async () => {
    const run = sample.runCapped;
    backend.on("run_release_cap", () => ({ ...run, cap_released: true }));
    const { user } = runPage(run);
    expect(
      await screen.findByText("Waiting: nightly-tests reached its $2.00 daily cap"),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Raise cap" })).toHaveAttribute(
      "href",
      `/dots/${run.dot_id}`,
    );
    await user.click(screen.getByRole("button", { name: "Run once anyway" }));
    await waitFor(() => expect(backend.callsTo("run_release_cap")).toEqual([{ runId: run.id }]));
  });

  it("offers a scheduled run no Run once anyway", async () => {
    runPage({ ...sample.runCapped, trigger: "schedule" });
    expect(await screen.findByRole("link", { name: "Raise cap" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Run once anyway" })).toBeNull();
  });
});

describe("a sol's spending caps in its editor", () => {
  function stored() {
    let dot = sample.dot;
    backend.on("get_dot", () => dot);
    backend.on("update_dot", (args) => {
      dot = { ...dot, ...(args.spec as object) };
      return dot;
    });
    backend.on("spend_today", () => sample.spend);
  }

  it("show what the sol spent today against its cap", async () => {
    stored();
    renderApp(`/dots/${sample.dot.id}`);
    const line = await screen.findByText(/^Today: \$2\.00 of \$2\.00/);
    expect(line).toHaveTextContent("estimated · 400,000 tokens · Cap reached");
    expect(screen.getByLabelText("Daily cost cap (USD)")).toHaveValue("2");
  });

  it("save a token cap, and refuse one out of bounds", async () => {
    stored();
    const { user } = renderApp(`/dots/${sample.dot.id}`);
    const tokens = await screen.findByLabelText("Daily token cap");
    await user.type(tokens, "0");
    expect(await screen.findByText("A token cap is 1 to 10,000,000,000 tokens.")).toBeVisible();
    await user.clear(tokens);
    await user.type(tokens, "300000");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_dot")).toEqual([
        {
          id: sample.dot.id,
          spec: {
            ...specOf(sample.dot),
            spend_cap: { daily_usd: 2, daily_tokens: 300000 },
          },
        },
      ]),
    );
  });
});

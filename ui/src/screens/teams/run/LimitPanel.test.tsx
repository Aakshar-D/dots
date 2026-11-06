import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type { WorkflowRun } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

import { LimitPanel } from "./LimitPanel";

const [waiting, , held, , budget] = sample.workflowRuns as [
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
];

/// The limit's actions on their own, as a conversation's workflow card shows them.
function show(run: WorkflowRun, stop?: boolean) {
  const queryClient = createQueryClient();
  const view = render(
    <QueryClientProvider client={queryClient}>
      <LimitPanel run={run} stop={stop} />
    </QueryClientProvider>,
  );
  return { ...view, queryClient, user: userEvent.setup() };
}

describe("the limit's actions", () => {
  it("say every limit the run reached and what Add budget gives, then add it", async () => {
    const spent: WorkflowRun = { ...budget, spent_tokens: 1_600_000, active_ms: 121 * 60_000 };
    backend.on("workflow_add_budget", () => ({ ...spent, status: "running", waiting_on: null }));
    const { user, queryClient } = show(spent);
    const wait = within(screen.getByRole("region", { name: "Budget reached" }));
    expect(
      wait.getByText(
        "It reached its limits: runs 3 / 3, tokens 1.6M / 1.5M and minutes 121 / 120. Add budget gives it 10 runs, 1.5M tokens and 120 minutes more, once.",
      ),
    ).toBeVisible();
    expect(wait.getByRole("button", { name: "Stop" })).toBeVisible();
    await user.click(wait.getByRole("button", { name: "Add budget" }));
    expect(backend.callsTo("workflow_add_budget")).toEqual([{ id: "01J0WFR5" }]);
    expect(queryClient.getQueryData(keys.workflowRun("01J0WFR5"))).toMatchObject({
      status: "running",
    });
  });

  it("say what the person's own held send-back was, and allow one more", async () => {
    const mine: WorkflowRun = {
      ...held,
      send_back: { from: "review", to: "build", note: "Smaller.", by: "person", held: true },
    };
    backend.on("workflow_allow_send_back", () => ({
      ...mine,
      status: "running",
      waiting_on: null,
    }));
    const { user } = show(mine, false);
    const wait = within(screen.getByRole("region", { name: "Send-back limit reached" }));
    expect(wait.getByText("You sent it back from Review to Build: “Smaller.”")).toBeVisible();
    expect(wait.queryByRole("button", { name: "Stop" })).toBeNull();
    await user.click(wait.getByRole("button", { name: "Allow one more send-back" }));
    expect(backend.callsTo("workflow_allow_send_back")).toEqual([{ id: "01J0WFR3" }]);
  });

  it("show nothing for a run that waits on something else or is not waiting", () => {
    expect(show(waiting).container).toBeEmptyDOMElement();
    const going: WorkflowRun = { ...budget, status: "running", waiting_on: null };
    expect(show(going).container).toBeEmptyDOMElement();
  });

  it("stop the run after asking", async () => {
    backend.on("workflow_stop", () => ({ ...budget, status: "cancelled", waiting_on: null }));
    const { user } = show(budget);
    await user.click(screen.getByRole("button", { name: "Stop" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Stop run" }),
    );
    expect(backend.callsTo("workflow_stop")).toEqual([{ id: "01J0WFR5" }]);
  });
});

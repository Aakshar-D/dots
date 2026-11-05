import { act, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox, WaitingWorkflowRun, WorkflowRun } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// "Fix and ship" of crew, waiting on the person's check at its step "Review".
const [check] = sample.inbox.workflows as [WaitingWorkflowRun];
/// A test run of a deleted workflow, held by its budget before any step, with a long task.
const budget: WaitingWorkflowRun = {
  ...check,
  id: "01J0WFR5",
  workflow_id: null,
  workflow_name: null,
  task: "Update the dependencies.\nThen run the tests.",
  waiting_on: "budget",
  step_name: null,
  origin_dot_id: null,
  test: true,
};
const empty: Inbox = {
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held: [],
  instructions: [],
  app_reviews: [],
  team_scope: [],
  workflows: [],
};
const only = (workflows: WaitingWorkflowRun[]): Inbox => ({ ...empty, workflows });

describe("waiting workflow runs in the Inbox", () => {
  it("lists each with its team, workflow, task, step and what it waits for, opening its page", async () => {
    backend.on("inbox", () => only([check, budget]));
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Workflows" });
    expect(screen.queryByText(/Nothing waits for you/)).toBeNull();
    const [first, second] = within(section).getAllByRole("listitem");

    const one = within(first!);
    expect(first).toHaveAccessibleName("Fix and ship of crew waiting");
    expect(one.getByText(check.task)).toBeVisible();
    expect(first).toHaveTextContent(/crew · Fix and ship · at Review · started /);
    expect(one.getByText("Needs your check")).toBeVisible();
    expect(one.queryByText("Test")).toBeNull();
    expect(one.getByRole("link", { name: "crew" })).toHaveAttribute("href", "/teams/01J0TEAM");
    expect(one.getByRole("link", { name: "Open the run" })).toHaveAttribute(
      "href",
      "/workflow-runs/01J0WFR",
    );

    // A deleted workflow, no step yet, the task's first line only, and the Test label.
    const two = within(second!);
    expect(second).toHaveAccessibleName("A deleted workflow of crew waiting");
    expect(two.getByText("Update the dependencies.")).toBeVisible();
    expect(second).not.toHaveTextContent(/Then run the tests/);
    expect(second).toHaveTextContent(/crew · A deleted workflow · started /);
    expect(second).not.toHaveTextContent(/ · at /);
    expect(two.getByText("Hit its budget")).toBeVisible();
    expect(two.getByText("Test")).toBeVisible();
    expect(two.getByRole("link", { name: "Open the run" })).toHaveAttribute(
      "href",
      "/workflow-runs/01J0WFR5",
    );
  });

  it("says what a send-back limit and a failed step wait for", async () => {
    backend.on("inbox", () =>
      only([
        { ...check, id: "01J0WFR3", waiting_on: "send_backs", step_name: "Build" },
        { ...check, id: "01J0WFR4", waiting_on: "step_failed", step_name: "Ship" },
      ]),
    );
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Workflows" });
    const [limit, failed] = within(section).getAllByRole("listitem");
    expect(within(limit!).getByText("Hit its send-back limit")).toBeVisible();
    expect(limit).toHaveTextContent(/ · at Build · /);
    expect(within(failed!).getByText("A step failed")).toBeVisible();
    expect(failed).toHaveTextContent(/ · at Ship · /);
  });

  it("stands after the apps to review and before the new instructions", async () => {
    backend.on("inbox", () => ({
      ...sample.inbox,
      review_runs: [],
      attention: [],
      tools_changed: [],
      watchers: [],
      held: [],
      team_scope: [],
    }));
    renderApp("/inbox");
    await screen.findByRole("region", { name: "Workflows" });
    const groups = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(groups).toEqual(["Approvals", "Apps to review", "Workflows", "New instructions"]);
  });

  it("leaves the Inbox when the run goes on, read again on its event", async () => {
    let inbox = only([check]);
    backend.on("inbox", () => inbox);
    const { listening } = renderApp("/inbox");
    await screen.findByRole("region", { name: "Workflows" });
    await listening();
    const [waiting] = sample.workflowRuns as [WorkflowRun];
    inbox = empty;
    act(() =>
      backend.emit("runtime", {
        type: "workflow_run_updated",
        run: { ...waiting, status: "running", waiting_on: null },
      }),
    );
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(screen.queryByRole("region", { name: "Workflows" })).toBeNull();
  });
});

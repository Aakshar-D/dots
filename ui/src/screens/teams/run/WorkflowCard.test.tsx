import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import type { Team, Workflow, WorkflowRun } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

import { WorkflowCard } from "./WorkflowCard";

const [crew] = sample.teams as [Team, Team];
const [fixAndShip] = sample.workflows as [Workflow, Workflow, Workflow, Workflow];
const [waiting, done, , failed, budget] = sample.workflowRuns as [
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
];
const TASK = "The tray icon is invisible on a light taskbar.";

/// The card of `run` by itself, of crew's Fix and ship.
function show(run: WorkflowRun) {
  backend.on("team_get", () => crew);
  backend.on("workflow_get", () => fixAndShip);
  backend.on("workflow_check", () => sample.check);
  const view = render(
    <QueryClientProvider client={createQueryClient()}>
      <MemoryRouter>
        <WorkflowCard run={run} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { ...view, user: userEvent.setup() };
}

const card = () => within(screen.getByRole("article", { name: `Workflow run: ${TASK}` }));
/// The names of the card's steps, each with its state.
const marks = () =>
  within(card().getByRole("list", { name: "Steps" }))
    .getAllByRole("listitem")
    .map((item) => item.getAttribute("aria-label"));

describe("a workflow's card in the chat", () => {
  it("shows the team, the workflow, the task, the status, the spend and the steps' marks", async () => {
    show(waiting);
    expect(await card().findByText("· Fix and ship")).toBeVisible();
    expect(card().getByText("crew")).toBeVisible();
    expect(card().getByText(TASK)).toBeVisible();
    expect(card().getByText("Needs your check")).toHaveAttribute("data-status", "waiting");
    expect(card().getByText("3 runs · 120k tokens")).toBeVisible();
    expect(card().getByRole("list", { name: "Steps" })).toBeVisible();
    // The branch took its "fixed" path: its steps stand in its place.
    expect(marks()).toEqual(["Build: Done", "Review: Waiting for you", "Ship: Not reached"]);
    // The check's actions, one Stop for the card, and Open run.
    const check = within(card().getByRole("region", { name: "Your check" }));
    expect(await check.findByRole("button", { name: "Approve" })).toBeEnabled();
    expect(
      check.getByRole("button", { name: "View diff of dots/nightly-tests/01J0RUN5" }),
    ).toBeVisible();
    expect(check.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(card().getAllByRole("button", { name: "Stop" })).toHaveLength(1);
    expect(card().getByRole("link", { name: "Open run" })).toHaveAttribute(
      "href",
      "/workflow-runs/01J0WFR",
    );
    expect(card().queryByRole("button", { name: "Retry step" })).toBeNull();
  });

  it("approves the check with the branches and commits it showed", async () => {
    backend.on("workflow_approve", () => ({ ...waiting, status: "running", waiting_on: null }));
    const { user } = show(waiting);
    await user.click(await card().findByRole("button", { name: "Approve" }));
    expect(backend.callsTo("workflow_approve")).toEqual([
      {
        id: "01J0WFR",
        note: null,
        tips: sample.check.branches.map(({ branch, commit }) => ({ branch, commit })),
      },
    ]);
  });

  it("offers Add budget at the budget, with one Stop, for a run whose workflow was deleted", async () => {
    show(budget);
    expect(await card().findByText("crew")).toBeVisible();
    expect(card().getByText("· A deleted workflow")).toBeVisible();
    expect(card().getByRole("region", { name: "Budget reached" })).toBeVisible();
    expect(card().getByRole("button", { name: "Add budget" })).toBeEnabled();
    expect(card().getAllByRole("button", { name: "Stop" })).toHaveLength(1);
    expect(card().queryByRole("region", { name: "Your check" })).toBeNull();
    expect(backend.callsTo("workflow_get")).toEqual([]);
  });

  it("shows a failed step's error on a test run, and retries the step", async () => {
    backend.on("workflow_retry_step", () => ({ ...failed, status: "running", waiting_on: null }));
    const { user } = show(failed);
    expect(card().getByText("Test")).toBeVisible();
    expect(card().getByText("A step failed")).toBeVisible();
    expect(card().getByText("Ship: gh: could not create the pull request")).toBeVisible();
    // Before the branch decided, the path whose step ran stands in its place.
    expect(marks()).toEqual(["Build: Not reached", "Review: Not reached", "Ship: Failed"]);
    await user.click(card().getByRole("button", { name: "Retry step" }));
    expect(backend.callsTo("workflow_retry_step")).toEqual([{ id: "01J0WFR4" }]);
  });

  it("spins on a running step, and leaves a finished run with Open run only", async () => {
    const steps = waiting.steps.map((s) =>
      s.step_id === "review" ? { ...s, status: "running" as const } : s,
    );
    const running: WorkflowRun = { ...waiting, status: "running", waiting_on: null, steps };
    const { unmount } = show(running);
    const review = card().getByRole("listitem", { name: "Review: Running" });
    expect(review).toHaveAttribute("data-status", "running");
    expect(review.querySelector("svg")).toHaveClass("animate-spin", "motion-reduce:animate-none");
    expect(card().getByRole("button", { name: "Stop" })).toBeVisible();
    expect(card().queryByRole("region", { name: "Your check" })).toBeNull();
    unmount();

    show(done);
    expect(card().getByText("Succeeded")).toBeVisible();
    expect(marks()).toEqual(["Build: Done", "Review: Done", "Ship: Done"]);
    expect(card().queryAllByRole("button")).toEqual([]);
    expect(card().getByRole("link", { name: "Open run" })).toHaveAttribute(
      "href",
      "/workflow-runs/01J0WFR2",
    );
  });
});

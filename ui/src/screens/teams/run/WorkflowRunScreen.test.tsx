import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { StepDotRun, Team, Workflow, WorkflowRun } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [crew] = sample.teams as [Team, Team];
const [fixAndShip] = sample.workflows as [Workflow, Workflow, Workflow, Workflow];
const [waiting, done, held, failed, budget] = sample.workflowRuns as [
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
];
const LEAD = "dots/nightly-tests/01J0RUN5";
const MEMBER = "dots/reviewer/01J0RUN6";

/// The page of `run`, of crew's Fix and ship; its steps' dot runs are `runs`.
function runPage(run: WorkflowRun, runs: StepDotRun[] = []) {
  backend.on("workflow_run_get", () => run);
  backend.on("team_get", () => crew);
  backend.on("workflow_get", () => fixAndShip);
  backend.on("workflow_check", () => sample.check);
  backend.on("workflow_run_dot_runs", () => runs);
  return renderApp(`/workflow-runs/${run.id}`);
}

const step = (name: string) => screen.getByRole("group", { name });
/// The run's status badge in the header.
const status = (text: string) => screen.findByText(text, { selector: "[data-status]" });

describe("the workflow run page", () => {
  it("shows the workflow, the task, the project, the status and the budget spent", async () => {
    runPage(waiting);
    const title = await screen.findByRole("heading", { level: 1, name: "Fix and ship" });
    expect(within(title).getByRole("link", { name: "Fix and ship" })).toHaveAttribute(
      "href",
      "/teams/01J0TEAM/workflows/01J0WFL",
    );
    expect(screen.getByText("The tray icon is invisible on a light taskbar.")).toBeVisible();
    expect(screen.getByText("On Sage")).toBeVisible();
    expect(screen.getByText("Needs your check")).toBeVisible();
    expect(await screen.findByRole("link", { name: "crew" })).toHaveAttribute(
      "href",
      "/teams/01J0TEAM",
    );
    expect(screen.getByText("· Started from a chat")).toBeVisible();
    expect(screen.getByRole("link", { name: "· Open the chat" })).toHaveAttribute(
      "href",
      "/runs/01J0RUN",
    );
    expect(screen.queryByText("Test")).toBeNull();

    const budget = within(screen.getByRole("list", { name: "Budget" }));
    expect(budget.getByRole("listitem", { name: "Runs: 3 of 10" })).toHaveTextContent("3 / 10");
    expect(
      budget.getByRole("listitem", {
        name: "Tokens: 120k of 1.5M, and 900k read from the cache",
      }),
    ).toHaveTextContent("Tokens· 900k cache reads120k / 1.5M");
    expect(budget.getByRole("listitem", { name: "Minutes: 5 of 120" })).toBeVisible();
    expect(budget.getByRole("listitem", { name: "Send-backs: 0 of 3" })).toBeVisible();
  });

  it("lays out the steps with their state, the path taken, the members and their sol runs", async () => {
    runPage(waiting, sample.stepDotRuns);
    const build = within(await screen.findByRole("group", { name: "Step 1: Build" }));
    expect(build.getByText("Done")).toBeVisible();
    expect(build.getByText("Delegate")).toBeVisible();
    expect(await build.findByText("may start reviewer · 2 at once · 4 total")).toBeVisible();
    expect(build.getByText("fixed")).toBeVisible();
    // One link per thread of the step's own runs: the lead's (continued once), and the member's
    // that handed back no branch. The member with a branch is linked from the member.
    const dotRuns = within(await build.findByRole("list", { name: "Sol runs of Build" }));
    const links = dotRuns.getAllByRole("link");
    expect(links.map((l) => [l.textContent, l.getAttribute("href")])).toEqual([
      ["Run of nightly-tests", "/runs/01J0RUN5"],
      ["Run of reviewer", "/runs/01J0RUN8"],
    ]);
    expect(dotRuns.getAllByRole("listitem").map((i) => i.textContent)).toEqual([
      "Run of nightly-testsSucceeded",
      "Run of reviewerFailed",
    ]);
    const members = within(build.getByRole("list", { name: "Members Build started" }));
    const windows = within(
      members.getByRole("listitem", { name: "Check the icon on Windows 10." }),
    );
    expect(windows.getByText("reviewer")).toBeVisible();
    expect(windows.getByText("Succeeded")).toBeVisible();
    expect(windows.getByText(MEMBER)).toBeVisible();
    expect(windows.getByRole("link", { name: "Open run" })).toHaveAttribute(
      "href",
      "/runs/01J0RUN6",
    );
    const contrast = within(
      members.getByRole("listitem", { name: "Check the high-contrast theme." }),
    );
    expect(contrast.getByText("Failed")).toBeVisible();
    expect(contrast.getByText("No branch")).toBeVisible();
    expect(contrast.queryByRole("link")).toBeNull();

    const route = within(step("Step 2: Fixed?"));
    expect(route.getByText("Done")).toBeVisible();
    expect(route.getByRole("region", { name: "Path fixed" })).toHaveTextContent("fixed · Taken");
    expect(route.getByRole("region", { name: "Otherwise" })).toHaveTextContent(
      "Otherwise · Not taken",
    );
    expect(within(step("Step 2.1: Review")).getByText("Waiting for you")).toBeVisible();
    expect(step("Step 2.1: Review")).toHaveAttribute("data-status", "waiting");
    expect(within(step("Step 2.2: Ship")).getByText("Not reached")).toBeVisible();
    expect(within(step("Step 2.1: Design")).getByText("Not reached")).toBeVisible();
    // The check is acted on where it waits; the page has one Stop, in its header.
    const check = within(step("Step 2.1: Review")).getByRole("region", { name: "Your check" });
    expect(await within(check).findByText("Made the icon dark on a light taskbar.")).toBeVisible();
    expect(within(check).queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Stop" })).toHaveLength(1);
    expect(backend.callsTo("workflow_run_dot_runs")).toContainEqual({ id: "01J0WFR" });
    // The check runs no dot run of its own.
    expect(
      within(step("Step 2.1: Review")).queryByRole("list", { name: "Sol runs of Review" }),
    ).toBeNull();
  });

  it("approves the check on its card, then shows the run going on", async () => {
    const after: WorkflowRun = { ...done, id: waiting.id };
    backend.on("workflow_approve", () => after);
    const { user } = runPage(waiting);
    const check = within(await screen.findByRole("region", { name: "Your check" }));
    await check.findByRole("listitem", { name: LEAD });
    await user.click(check.getByRole("button", { name: "Approve" }));
    expect(backend.callsTo("workflow_approve")).toEqual([
      {
        id: "01J0WFR",
        note: null,
        stepRunId: "01J0STP3",
        tips: [
          { branch: LEAD, commit: "2222222222222222222222222222222222222222" },
          { branch: MEMBER, commit: null },
        ],
      },
    ]);
    expect(await status("Succeeded")).toBeVisible();
    expect(screen.queryByRole("region", { name: "Your check" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("stops the run after asking", async () => {
    backend.on("workflow_stop", () => ({ ...waiting, status: "cancelled", waiting_on: null }));
    const { user } = runPage(waiting);
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    const ask = await screen.findByRole("alertdialog", { name: "Stop this workflow run?" });
    expect(ask).toHaveTextContent("Its branches stay, and nothing already pushed is undone.");
    await user.click(within(ask).getByRole("button", { name: "Stop run" }));
    expect(backend.callsTo("workflow_stop")).toEqual([{ id: "01J0WFR" }]);
    expect(await status("Cancelled")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("asks for more budget on a run whose workflow was deleted, and opens its ready branch", async () => {
    backend.on("workflow_add_budget", () => ({ ...budget, status: "running", waiting_on: null }));
    backend.on("open_project_folder", () => null);
    backend.on("workflow_branch_diff", () => ({ exists: true, truncated: false, files: [] }));
    const { user } = runPage(budget);
    expect(
      await screen.findByRole("heading", { level: 1, name: "A deleted workflow" }),
    ).toBeVisible();
    expect(screen.queryByRole("link", { name: "A deleted workflow" })).toBeNull();
    expect(backend.callsTo("workflow_get")).toEqual([]);
    expect(screen.getByText("On Site")).toBeVisible();
    expect(screen.getByRole("listitem", { name: "Runs: 3 of 3, reached" })).toHaveAttribute(
      "data-reached",
      "true",
    );

    const wait = within(screen.getByRole("region", { name: "Budget reached" }));
    expect(wait.getByText("It hit its budget")).toBeVisible();
    expect(
      wait.getByText("It reached its limit: runs 3 / 3. Add budget gives it 10 runs more, once."),
    ).toBeVisible();
    expect(wait.queryByRole("button", { name: "Stop" })).toBeNull();

    const site = within(screen.getByRole("region", { name: "Path Site" }));
    const ready = within(
      within(site.getByRole("group", { name: "Step 3.1: Ship" })).getByRole("listitem", {
        name: LEAD,
      }),
    );
    expect(ready.getByText("Ready")).toBeVisible();
    await user.click(ready.getByRole("button", { name: `Open folder of ${LEAD}` }));
    expect(backend.callsTo("open_project_folder")).toEqual([
      { projectId: null, workflowRunId: "01J0WFR5", branch: LEAD },
    ]);
    await user.click(ready.getByRole("button", { name: `View diff of ${LEAD}` }));
    const dialog = await screen.findByRole("dialog", { name: `Diff of ${LEAD}` });
    expect(await within(dialog).findByText("No changes.")).toBeVisible();
    expect(backend.callsTo("workflow_branch_diff")).toEqual([
      { id: "01J0WFR5", branch: LEAD, commit: "2222222222222222222222222222222222222222" },
    ]);
    await user.keyboard("{Escape}");

    await user.click(wait.getByRole("button", { name: "Add budget" }));
    expect(backend.callsTo("workflow_add_budget")).toEqual([{ id: "01J0WFR5" }]);
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Budget reached" })).toBeNull(),
    );
    expect(await status("Running")).toBeVisible();
  });

  it("holds a send-back at its limit until one more is allowed", async () => {
    backend.on("workflow_allow_send_back", () => ({
      ...held,
      status: "running",
      waiting_on: null,
    }));
    const { user } = runPage(held);
    const wait = within(await screen.findByRole("region", { name: "Send-back limit reached" }));
    expect(wait.getByText("The work was sent back 3 times, and its limit is 3.")).toBeVisible();
    expect(
      wait.getByText("Design asks to send it back to Build: “The icon needs a new shape first.”"),
    ).toBeVisible();
    expect(within(step("Step 2.1: Design")).getByText("Waiting")).toBeVisible();
    expect(
      within(step("Step 2.1: Design")).getByText(
        "Sends the work back to Build: “The icon needs a new shape first.”",
      ),
    ).toBeVisible();
    await user.click(wait.getByRole("button", { name: "Allow one more send-back" }));
    expect(backend.callsTo("workflow_allow_send_back")).toEqual([{ id: "01J0WFR3" }]);
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Send-back limit reached" })).toBeNull(),
    );
  });

  it("shows a failed deploy's error once, what it pushed, and retries the step", async () => {
    backend.on("workflow_retry_step", () => ({ ...failed, status: "running", waiting_on: null }));
    const { user } = runPage(failed);
    const ship = within(await screen.findByRole("group", { name: "Step 2.2: Ship" }));
    expect(screen.getByText("Test")).toBeVisible();
    expect(ship.getByText("Failed")).toBeVisible();
    expect(ship.getAllByText("gh: could not create the pull request")).toHaveLength(1);
    // The run's own error names the step, which its card already says.
    expect(screen.queryByText("Ship: gh: could not create the pull request")).toBeNull();
    expect(within(ship.getByRole("listitem", { name: LEAD })).getByText("Pushed")).toBeVisible();
    expect(
      within(ship.getByRole("listitem", { name: MEMBER })).getByText("Not pushed"),
    ).toBeVisible();
    expect(ship.queryByRole("button", { name: `Open folder of ${LEAD}` })).toBeNull();
    await user.click(ship.getByRole("button", { name: "Retry step" }));
    expect(backend.callsTo("workflow_retry_step")).toEqual([{ id: "01J0WFR4" }]);
    await waitFor(() => expect(ship.queryByRole("button", { name: "Retry step" })).toBeNull());
  });

  it("shows a finished run's attempts, the check's decision and the pull request it opened", async () => {
    runPage(done);
    const build = within(await screen.findByRole("group", { name: "Step 1: Build" }));
    expect(build.getByText("· attempt 2")).toBeVisible();
    expect(build.getByText("Its earlier chat could not go on, so it started fresh.")).toBeVisible();
    expect(within(step("Step 2.1: Review")).getByText("Approved")).toBeVisible();
    const ship = within(within(step("Step 2.2: Ship")).getByRole("listitem", { name: LEAD }));
    expect(ship.getByRole("link", { name: "Pull request" })).toHaveAttribute(
      "href",
      "https://github.com/Aakshar-D/dots/pull/40",
    );
    expect(ship.getByRole("link", { name: "Pull request" })).toHaveAttribute("target", "_blank");
    expect(ship.getByText("merges once its checks pass")).toBeVisible();
    expect(ship.getByRole("button", { name: `Copy the branch ${LEAD}` })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Your check" })).toBeNull();
    // The approved check's hand-off names the lead's branch, but the check ran no dot run, so
    // its card links none: the lead's run is linked from Build.
    expect(
      within(step("Step 2.1: Review")).queryByRole("list", { name: "Sol runs of Review" }),
    ).toBeNull();
  });

  it("follows the run's events: what the core says now is shown", async () => {
    const { listening } = runPage(waiting);
    await screen.findByText("Needs your check");
    await listening();
    const moved: WorkflowRun = { ...waiting, status: "running", waiting_on: null };
    backend.on("workflow_run_get", () => moved);
    act(() => backend.emit("runtime", { type: "workflow_run_updated", run: moved }));
    expect(await status("Running")).toBeVisible();
    expect(screen.queryByRole("region", { name: "Your check" })).toBeNull();
  });

  it("follows its steps' sol runs: a step's run that changes shows its new state", async () => {
    const lead = { ...sample.stepDotRuns[0], status: "running" as const };
    const { listening } = runPage(waiting, [lead]);
    const build = within(await screen.findByRole("group", { name: "Step 1: Build" }));
    const dotRuns = within(await build.findByRole("list", { name: "Sol runs of Build" }));
    expect(dotRuns.getByText("Running")).toBeVisible();
    await listening();
    backend.on("workflow_run_dot_runs", () => [{ ...lead, status: "succeeded" }]);
    act(() =>
      backend.emit("runtime", {
        type: "run_updated",
        run: { ...sample.run, id: lead.run_id, workflow_step_run_id: lead.step_run_id },
      }),
    );
    expect(await dotRuns.findByText("Succeeded")).toBeVisible();
    expect(dotRuns.queryByText("Running")).toBeNull();
  });

  it("says why a run cannot be shown", async () => {
    backend.on("workflow_run_get", () => {
      throw { kind: "not_found", message: "workflow run 01J0NOPE" };
    });
    renderApp("/workflow-runs/01J0NOPE");
    expect(await screen.findByRole("alert")).toHaveTextContent("workflow run 01J0NOPE");
    expect(screen.getByRole("heading", { level: 1, name: "Workflow run" })).toBeVisible();
  });
});

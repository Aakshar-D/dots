import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Step, Team, Workflow, WorkflowProblem, WorkflowSpec } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

import { allSteps } from "./draft";

const [crew] = sample.teams as [Team, Team];
const [fix, pushIt, nightly, onIssue] = sample.workflows as [
  Workflow,
  Workflow,
  Workflow,
  Workflow,
];

/// The builder of `workflow`, whose draft the live check answers with `problem(spec)`, and
/// whose save answers with what it was sent.
function open(
  workflow: Workflow,
  problem: (spec: WorkflowSpec) => WorkflowProblem | null = () => null,
) {
  backend.on("team_get", () => crew);
  backend.on("workflow_get", () => workflow);
  backend.on("workflow_check_spec", (args) => problem(args.spec as WorkflowSpec));
  backend.on("workflow_update", (args) => ({ ...workflow, ...(args.spec as WorkflowSpec) }));
  return renderApp(`/teams/${crew.id}/workflows/${workflow.id}`);
}

const card = (name: string) => screen.getByRole("group", { name });
const save = () => screen.getByRole("button", { name: "Save" });
/// The ids of the steps the last save sent, depth first.
const savedIds = () => {
  const spec = backend.callsTo("workflow_update").at(-1)!.spec as WorkflowSpec;
  return allSteps(spec.definition.steps).map((s: Step) => s.id);
};

describe("the workflow builder", () => {
  it("shows the header, the trigger, the numbered steps in their kind's look, and the budget", async () => {
    open(fix);
    expect(await screen.findByRole("textbox", { name: "Workflow name" })).toHaveValue(
      "Fix and ship",
    );
    expect(screen.getByRole("heading", { name: "Fix and ship" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "crew" })).toHaveAttribute("href", "/teams/01J0TEAM");
    expect(screen.getByRole("switch", { name: "On" })).toBeChecked();
    expect(screen.getByText("1 run in flight uses the previous version")).toBeVisible();
    // Nothing changed yet.
    expect(save()).toBeDisabled();

    const trigger = card("Trigger");
    expect(trigger).toHaveTextContent("When: Started from a chat");
    expect(trigger).toHaveTextContent("When you chat with nightly-tests");
    expect(trigger.className).toContain("border-primary");

    // A delegate: dashed in the accent, with what it may start, by name.
    const build = card("Step 1: Build");
    expect(build).toHaveTextContent("nightly-tests · Fix the bug and say whether it is fixed.");
    expect(build).toHaveTextContent("may start reviewer · 2 at once · 4 total");
    expect(build).toHaveTextContent("Delegate");
    expect(build.className).toContain("border-dashed");
    expect(build.className).toContain("border-primary");

    // A branch's paths are labelled columns holding cards of their own.
    const route = card("Step 2: Fixed?");
    expect(route).toHaveTextContent("Goes on by the outcome reported before it");
    const fixed = within(within(route).getByRole("region", { name: "Path fixed" }));
    const review = fixed.getByRole("group", { name: "Step 2.1: Review" });
    expect(review.className).toContain("border-needs-border");
    expect(review).toHaveTextContent("Approve, or send back to Build with a note");
    expect(fixed.getByRole("group", { name: "Step 2.2: Ship" })).toHaveTextContent(
      "Pushes, opens a pull request and merges it (squash)",
    );
    const otherwise = within(within(route).getByRole("region", { name: "Otherwise" }));
    expect(otherwise.getByRole("group", { name: "Step 2.1: Design" })).toHaveTextContent(
      "reviewer · Propose a design.",
    );

    const budget = within(screen.getByRole("region", { name: "Budget" }));
    expect(budget.getByRole("spinbutton", { name: "Runs" })).toHaveValue(10);
    expect(budget.getByRole("spinbutton", { name: "Tokens" })).toHaveValue(1_500_000);
    expect(budget.getByRole("spinbutton", { name: "Minutes" })).toHaveValue(120);
    expect(budget.getByRole("spinbutton", { name: "Send-backs" })).toHaveValue(3);
  });

  it("shows each trigger and deploy mode, and a member who left by id", async () => {
    open(onIssue);
    expect(await screen.findByRole("group", { name: "Trigger" })).toHaveTextContent(
      "When: On a webhookLook at the newest issue., unless the call says the task",
    );
    expect(card("Step 3: Ship")).toHaveTextContent("reviewer deploys: Release what was approved.");
    expect(screen.queryByText(/in flight/)).not.toBeInTheDocument();
  });

  it("shows a schedule's task and project, and a branch on project", async () => {
    open(nightly);
    expect(await screen.findByRole("group", { name: "Trigger" })).toHaveTextContent(
      "Update the dependencies. · on Sage",
    );
    const branch = card("Step 3: Which project");
    expect(branch).toHaveTextContent("Goes on by the project");
    expect(
      within(within(branch).getByRole("region", { name: "Path Sage" })).getByRole("group", {
        name: "Step 3.1: Ship",
      }),
    ).toHaveTextContent("Pushes the branches and opens a pull request");
    expect(
      within(within(branch).getByRole("region", { name: "Path Site" })).getByRole("group", {
        name: "Step 3.1: Ship",
      }),
    ).toHaveTextContent("Keeps the branches: nothing is pushed");
  });

  it("adds a step of each kind between the cards and at the end, with ids it makes", async () => {
    const { user } = open(nightly);
    await screen.findByRole("group", { name: "Step 1: Work" });

    await user.click(screen.getByRole("button", { name: "Add step" }));
    await user.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Check" }));
    const added = card("Step 4: Your check");
    expect(added).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Add step after Work" }));
    expect(
      within(screen.getByRole("menu"))
        .getAllByRole("menuitem")
        .map((item) => item.textContent),
    ).toEqual(["Agent step", "Check", "Deploy", "Branch"]);
    await user.click(screen.getByRole("menuitem", { name: "Agent step" }));
    expect(card("Step 2: Work")).toHaveTextContent("nightly-tests · Work on the task.");
    expect(card("Step 3: Review")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Add step before Work" }));
    await user.click(screen.getByRole("menuitem", { name: "Branch" }));
    const branch = card("Step 1: Branch");
    expect(within(branch).getByRole("region", { name: "Path Sage" })).toBeInTheDocument();
    expect(within(branch).getByRole("region", { name: "Path Site" })).toBeInTheDocument();

    const site = within(within(branch).getByRole("region", { name: "Path Site" }));
    await user.click(site.getByRole("button", { name: "Add step to the path Site" }));
    await user.click(screen.getByRole("menuitem", { name: "Deploy" }));
    expect(card("Step 1.1: Deploy")).toHaveTextContent("Keeps the branches: nothing is pushed");

    await waitFor(() => expect(save()).toBeEnabled());
    await user.click(save());
    expect(savedIds()).toEqual([
      "branch-1",
      "deploy-1",
      "work",
      "agent-1",
      "check",
      "by-project",
      "pr",
      "keep",
      "check-1",
    ]);
    // Saved: nothing is left to save.
    await waitFor(() => expect(save()).toBeDisabled());
  });

  it("moves a step with its buttons and Alt+Up and Alt+Down, within its own list", async () => {
    const { user } = open(fix);
    await screen.findByRole("group", { name: "Step 1: Build" });
    expect(screen.getByRole("button", { name: "Move Build up" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Move Build down" }));
    expect(card("Step 2: Build")).toHaveFocus();
    expect(card("Step 1: Fixed?")).toBeInTheDocument();

    await user.keyboard("{Alt>}{ArrowUp}{/Alt}");
    expect(card("Step 1: Build")).toHaveFocus();

    // A card in a path moves on its path; the branch around it stays.
    const review = card("Step 2.1: Review");
    act(() => review.focus());
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(card("Step 2.2: Review")).toHaveFocus();
    expect(card("Step 2.1: Ship")).toBeInTheDocument();
    expect(card("Step 2: Fixed?")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Move Review down" })).toBeDisabled();
    // At the end of its path it stays there.
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(card("Step 2.2: Review")).toHaveFocus();

    await waitFor(() => expect(save()).toBeEnabled());
    await user.click(save());
    expect(savedIds()).toEqual(["build", "route", "ship", "review", "design"]);
  });

  it("deletes a step at once, and a branch with steps in it after asking", async () => {
    const { user } = open(nightly);
    await user.click(await screen.findByRole("button", { name: "Delete Review" }));
    expect(screen.queryByRole("group", { name: "Step 2: Review" })).not.toBeInTheDocument();
    expect(card("Step 2: Which project")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete Which project" }));
    const ask = await screen.findByRole("alertdialog", { name: "Delete Which project?" });
    expect(ask).toHaveTextContent(
      "The 2 steps inside it go with it. Nothing is saved until you press Save.",
    );
    await user.click(within(ask).getByRole("button", { name: "Delete step" }));
    expect(screen.queryByRole("group", { name: "Step 2: Which project" })).not.toBeInTheDocument();

    await waitFor(() => expect(save()).toBeEnabled());
    await user.click(save());
    expect(savedIds()).toEqual(["work"]);
  });

  it("puts the live check's problem on the card it names, else on the header, and keeps Save off", async () => {
    const noCheck: WorkflowProblem = {
      step_id: "pr",
      message: 'step "Ship": a deploy needs a check before it on every path',
    };
    const { user } = open(nightly, (spec) => {
      if (spec.definition.budget.max_runs === 0) {
        return { step_id: null, message: "every budget limit must be at least 1" };
      }
      return allSteps(spec.definition.steps).some((s) => s.kind === "check") ? null : noCheck;
    });
    await user.click(await screen.findByRole("button", { name: "Delete Review" }));
    const ship = within(screen.getByRole("region", { name: "Path Sage" })).getByRole("group", {
      name: "Step 2.1: Ship",
    });
    await waitFor(() => expect(ship).toHaveAccessibleDescription(noCheck.message));
    expect(ship).toHaveAttribute("aria-invalid", "true");
    expect(ship.className).toContain("border-destructive");
    expect(save()).toBeDisabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    // Put the check back: the problem goes, and Save comes on.
    await user.click(screen.getByRole("button", { name: "Add step after Work" }));
    await user.click(screen.getByRole("menuitem", { name: "Check" }));
    await waitFor(() => expect(save()).toBeEnabled());
    expect(ship).not.toHaveAttribute("aria-invalid");

    // A problem that names no step shows on the header.
    const runs = screen.getByRole("spinbutton", { name: "Runs" });
    await user.clear(runs);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "every budget limit must be at least 1",
    );
    expect(save()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Test run" })).toBeDisabled();
    // The check is asked once the draft settles, not on each key.
    const sent = backend.callsTo("workflow_check_spec").map((c) => c.spec as WorkflowSpec);
    expect(sent.at(-1)!.definition.budget.max_runs).toBe(0);
    expect(backend.callsTo("workflow_check_spec").at(-1)!.teamId).toBe("01J0TEAM");
    await user.type(runs, "6");
    await waitFor(() => expect(save()).toBeEnabled());
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows a saved workflow's problem on its card when it opens", async () => {
    const [notMember] = sample.workflowProblems as [WorkflowProblem, WorkflowProblem];
    open(pushIt, () => notMember);
    const work = await screen.findByRole("group", { name: "Step 1: Work" });
    await waitFor(() => expect(work).toHaveAccessibleDescription(notMember.message));
    // The member is shown by id: it left the team.
    expect(work).toHaveTextContent("01J0GONE · Do the task.");
    expect(screen.getByRole("button", { name: "Test run" })).toBeDisabled();
  });

  it("saves the name, the switch and the budget, and shows the core's refusal on the header", async () => {
    const { user } = open(nightly);
    const name = await screen.findByRole("textbox", { name: "Workflow name" });
    await user.clear(name);
    await user.type(name, " Fix and ship ");
    await user.click(screen.getByRole("switch", { name: "On" }));
    const tokens = screen.getByRole("spinbutton", { name: "Tokens" });
    await user.clear(tokens);
    await user.type(tokens, "2000000");
    backend.on("workflow_update", () => {
      throw { kind: "conflict", message: "the team already has a workflow named Fix and ship" };
    });
    await waitFor(() => expect(save()).toBeEnabled());
    await user.click(save());
    expect(backend.callsTo("workflow_update")).toEqual([
      {
        id: "01J0WFL3",
        spec: {
          name: "Fix and ship",
          enabled: false,
          definition: {
            ...nightly.definition,
            budget: { ...nightly.definition.budget, max_tokens: 2_000_000 },
          },
        },
      },
    ]);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "the team already has a workflow named Fix and ship",
    );
    // The draft is kept, and the next edit takes the refusal away.
    expect(name).toHaveValue(" Fix and ship ");
    await user.type(name, "2");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("opens a step's settings in a side sheet, and renames the step in the draft", async () => {
    const { user } = open(fix);
    await user.click(await screen.findByRole("button", { name: "Settings of Ship" }));
    const sheet = await screen.findByRole("dialog", { name: "Deploy settings" });
    const field = within(sheet).getByRole("textbox", { name: "Name" });
    expect(field).toHaveValue("Ship");
    await user.clear(field);
    await user.type(field, "Merge");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(card("Step 2.2: Merge")).toBeInTheDocument();

    // A click on a card, or Enter on it, opens its settings too.
    await user.click(screen.getByText("reviewer · Propose a design."));
    expect(await screen.findByRole("dialog", { name: "Agent step settings" })).toBeVisible();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    act(() => card("Step 2.1: Review").focus());
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("dialog", { name: "Check settings" })).toBeVisible();
    await user.keyboard("{Escape}");

    await waitFor(() => expect(save()).toBeEnabled());
    await user.click(save());
    const spec = backend.callsTo("workflow_update")[0]!.spec as WorkflowSpec;
    expect(allSteps(spec.definition.steps).map((s) => s.name)).toEqual([
      "Build",
      "Fixed?",
      "Review",
      "Merge",
      "Design",
    ]);
  });

  it("follows the saved workflow while the draft is untouched, and keeps an edited draft", async () => {
    const { user, listening } = open(nightly);
    const name = await screen.findByRole("textbox", { name: "Workflow name" });
    await listening();
    backend.on("workflow_get", () => ({ ...nightly, name: "Nightly deps" }));
    act(() =>
      backend.emit("runtime", {
        type: "workflow_updated",
        team_id: crew.id,
        workflow_id: nightly.id,
      }),
    );
    await waitFor(() => expect(name).toHaveValue("Nightly deps"));
    expect(save()).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Delete Review" }));
    backend.on("workflow_get", () => ({ ...nightly, name: "Nightly again" }));
    act(() =>
      backend.emit("runtime", {
        type: "workflow_updated",
        team_id: crew.id,
        workflow_id: nightly.id,
      }),
    );
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "Nightly again" })).toBeInTheDocument(),
    );
    expect(name).toHaveValue("Nightly deps");
    expect(screen.queryByRole("group", { name: "Step 2: Review" })).not.toBeInTheDocument();
  });

  it("saves the draft before a test run, then asks for its task, on or off", async () => {
    const { user } = open(nightly);
    const name = await screen.findByRole("textbox", { name: "Workflow name" });
    await user.type(name, " 2");
    await waitFor(() => expect(screen.getByRole("button", { name: "Test run" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Test run" }));
    expect(await screen.findByRole("dialog", { name: "Test run Nightly 2" })).toBeVisible();
    expect(backend.callsTo("workflow_update")).toHaveLength(1);
    expect(backend.callsTo("workflow_update")[0]).toMatchObject({ spec: { name: "Nightly 2" } });
    await user.keyboard("{Escape}");

    // A Test run starts while the workflow is off, so turning it off holds Test run back no
    // more: it saves the switch, then asks for the task.
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await user.click(screen.getByRole("switch", { name: "On" }));
    const test = screen.getByRole("button", { name: "Test run" });
    await waitFor(() => expect(test).toBeEnabled());
    expect(test).not.toHaveAttribute("title");
    await user.click(test);
    expect(await screen.findByRole("dialog", { name: "Test run Nightly 2" })).toBeVisible();
    expect(backend.callsTo("workflow_update")).toHaveLength(2);
    expect(backend.callsTo("workflow_update")[1]).toMatchObject({ spec: { enabled: false } });
  });

  it("deletes the workflow after asking, then goes back to the team page", async () => {
    backend.on("workflows_list", () => sample.workflows);
    backend.on("workflow_runs_list", () => []);
    backend.on("workflow_delete", () => null);
    const { user, router } = open(fix);
    await user.click(await screen.findByRole("button", { name: "Delete workflow" }));
    const ask = await screen.findByRole("alertdialog", { name: "Delete Fix and ship?" });
    expect(ask).toHaveTextContent(
      "Its runs stay on the team page, as runs of a deleted workflow. Runs in flight go on " +
        "with the version they started with: 1 run now.",
    );
    await user.click(within(ask).getByRole("button", { name: "Delete workflow" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/teams/01J0TEAM"));
    expect(backend.callsTo("workflow_delete")).toEqual([{ id: "01J0WFL" }]);
  });

  it("shows why a workflow cannot be shown", async () => {
    backend.on("team_get", () => crew);
    backend.on("workflow_get", () => {
      throw { kind: "not_found", message: "workflow nope" };
    });
    renderApp(`/teams/${crew.id}/workflows/nope`);
    expect(await screen.findByRole("alert")).toHaveTextContent("workflow nope");
  });
});

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot, Step, Team, Workflow, WorkflowSpec } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

import { findStep } from "./draft";

const [crew] = sample.teams as [Team, Team];
const [fix, , nightly] = sample.workflows as [Workflow, Workflow, Workflow, Workflow];

/// The builder of `workflow`, with no problem in any draft, whose save answers with what it
/// was sent.
function open(workflow: Workflow) {
  backend.on("team_get", () => crew);
  backend.on("workflow_get", () => workflow);
  backend.on("workflow_check_spec", () => null);
  backend.on("workflow_update", (args) => ({ ...workflow, ...(args.spec as WorkflowSpec) }));
  return renderApp(`/teams/${crew.id}/workflows/${workflow.id}`);
}

/// Opens the settings of the step `name` and gives its sheet.
async function settings(user: ReturnType<typeof open>["user"], name: string, title: string) {
  await user.click(await screen.findByRole("button", { name: `Settings of ${name}` }));
  return within(await screen.findByRole("dialog", { name: title }));
}

/// Closes the sheet, saves the draft, and gives the step `id` the save sent.
async function saved(user: ReturnType<typeof open>["user"], id: string): Promise<Step> {
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  const save = screen.getByRole("button", { name: "Save" });
  await waitFor(() => expect(save).toBeEnabled());
  await user.click(save);
  const spec = backend.callsTo("workflow_update").at(-1)!.spec as WorkflowSpec;
  return findStep(spec.definition.steps, id)!;
}

const dot = (id: string, name: string, mode: Dot["workspace_mode"]): Dot => ({
  ...sample.dot,
  id,
  name,
  workspace_mode: mode,
});

describe("a step's settings sheet", () => {
  it("edits an agent step's member, job, send-backs and outcomes in the draft", async () => {
    const { user } = open(fix);
    const sheet = await settings(user, "Design", "Agent step settings");
    expect(sheet.getByRole("textbox", { name: "Name" })).toHaveAttribute("maxlength", "80");
    const member = sheet.getByRole("combobox", { name: "Member" });
    expect(member).toHaveValue("01J0DOT2");
    expect(within(member).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "nightly-tests",
      "reviewer",
    ]);
    await user.selectOptions(member, "nightly-tests");
    const job = sheet.getByRole("textbox", { name: "Job" });
    expect(job).toHaveValue("Propose a design.");
    expect(job).toHaveAttribute("maxlength", "8000");
    await user.clear(job);
    await user.type(job, "Draw it.");

    // Only the agent step before it on its path: Build, not the steps of the path beside it.
    const back = within(sheet.getByRole("group", { name: "May send work back to" }));
    expect(back.getAllByRole("checkbox").map((b) => b.getAttribute("aria-label"))).toEqual([
      "Build",
    ]);
    expect(back.getByRole("checkbox", { name: "Build" })).toBeChecked();
    await user.click(back.getByRole("checkbox", { name: "Build" }));

    const outcomes = within(sheet.getByRole("group", { name: "Outcomes" }));
    const next = outcomes.getByRole("textbox", { name: "New outcome" });
    expect(next).toHaveAttribute("maxlength", "40");
    await user.type(next, " done {Enter}");
    await user.type(next, "later");
    await user.click(outcomes.getByRole("button", { name: "Add outcome" }));
    // The same label twice is not taken.
    await user.type(next, "done");
    expect(outcomes.getByRole("button", { name: "Add outcome" })).toBeDisabled();
    expect(outcomes.getByText("It is listed already.")).toBeVisible();
    await user.clear(next);
    await user.click(outcomes.getByRole("button", { name: "Remove the outcome later" }));
    expect(outcomes.getByRole("list", { name: "Outcomes" })).toHaveTextContent("done");
    expect(outcomes.getByRole("list", { name: "Outcomes" })).not.toHaveTextContent("later");

    // The card follows the draft.
    expect(screen.getByRole("group", { name: "Step 2.1: Design", hidden: true })).toHaveTextContent(
      "nightly-tests · Draw it.",
    );
    expect(await saved(user, "design")).toEqual({
      id: "design",
      name: "Design",
      kind: "agent",
      member: "01J0DOT",
      job: "Draw it.",
      can_send_back_to: [],
      outcomes: ["done"],
      delegate: null,
    });
  });

  it("makes an agent step a delegate with what it may start, and back", async () => {
    const { user } = open(fix);
    let sheet = await settings(user, "Build", "Agent step settings");
    const delegate = sheet.getByRole("switch", { name: "It may start other members" });
    expect(delegate).toBeChecked();
    const may = within(sheet.getByRole("group", { name: "May start" }));
    expect(may.getByRole("checkbox", { name: "reviewer" })).toBeChecked();
    expect(may.getByRole("checkbox", { name: "nightly-tests" })).not.toBeChecked();
    await user.click(may.getByRole("checkbox", { name: "nightly-tests" }));
    expect(sheet.getByRole("spinbutton", { name: "At once" })).toHaveValue(2);
    const total = sheet.getByRole("spinbutton", { name: "Total" });
    expect(total).toHaveValue(4);
    await user.clear(total);
    await user.type(total, "5");
    expect(screen.getByRole("group", { name: "Step 1: Build", hidden: true })).toHaveTextContent(
      "may start reviewer, nightly-tests · 2 at once · 5 total",
    );
    expect(await saved(user, "build")).toMatchObject({
      delegate: { may_start: ["01J0DOT2", "01J0DOT"], at_once: 2, total: 5 },
    });

    // Off, then on again: the section 5.3 defaults, with no member yet.
    sheet = await settings(user, "Build", "Agent step settings");
    await user.click(sheet.getByRole("switch", { name: "It may start other members" }));
    expect(sheet.queryByRole("group", { name: "May start" })).not.toBeInTheDocument();
    expect(await saved(user, "build")).toMatchObject({ delegate: null });
    sheet = await settings(user, "Build", "Agent step settings");
    await user.click(sheet.getByRole("switch", { name: "It may start other members" }));
    expect(sheet.getByRole("spinbutton", { name: "At once" })).toHaveValue(3);
    expect(sheet.getByRole("spinbutton", { name: "Total" })).toHaveValue(6);
    expect(await saved(user, "build")).toMatchObject({
      delegate: { may_start: [], at_once: 3, total: 6 },
    });
  });

  it("warns about a member that works in the folder itself", async () => {
    backend.on("list_dots", () => [
      dot("01J0DOT", "nightly-tests", "worktree"),
      dot("01J0DOT2", "reviewer", "folder"),
    ]);
    const { user } = open(fix);
    const warning =
      "reviewer works in the folder itself: in a workflow it changes the project's own " +
      "checkout, and leaves no branch to check or deploy.";
    let sheet = await settings(user, "Design", "Agent step settings");
    expect(await sheet.findByRole("note")).toHaveTextContent(warning);
    await user.selectOptions(sheet.getByRole("combobox", { name: "Member" }), "nightly-tests");
    expect(sheet.queryByRole("note")).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    // A member a delegate may start works in the project too.
    sheet = await settings(user, "Build", "Agent step settings");
    expect(await sheet.findByRole("note")).toHaveTextContent(warning);
  });

  it("picks the agent step a check sends back to, or the nearest one", async () => {
    const { user } = open(fix);
    const sheet = await settings(user, "Review", "Check settings");
    const to = sheet.getByRole("combobox", { name: "Send back to" });
    expect(to).toHaveValue("build");
    expect(within(to).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "The nearest agent step before it",
      "Build",
    ]);
    await user.selectOptions(to, "The nearest agent step before it");
    expect(screen.getByRole("group", { name: "Step 2.1: Review", hidden: true })).toHaveTextContent(
      "Approve, or send back with a note",
    );
    expect(await saved(user, "review")).toEqual({
      id: "review",
      name: "Review",
      kind: "check",
      send_back_to: null,
    });
  });

  it("changes a deploy's mode, its merge method, and the member who deploys", async () => {
    const { user } = open(fix);
    let sheet = await settings(user, "Ship", "Deploy settings");
    const mode = sheet.getByRole("combobox", { name: "Mode" });
    expect(mode).toHaveValue("pr_merge");
    expect(within(mode).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Keep the branches",
      "Push",
      "Push and open a pull request",
      "Push, open a pull request and merge it",
      "A member deploys",
    ]);
    const method = sheet.getByRole("combobox", { name: "Merge method" });
    expect(method).toHaveValue("squash");
    await user.selectOptions(method, "Rebase");
    expect(await saved(user, "ship")).toMatchObject({ mode: "pr_merge", merge_method: "rebase" });

    sheet = await settings(user, "Ship", "Deploy settings");
    await user.selectOptions(sheet.getByRole("combobox", { name: "Mode" }), "A member deploys");
    expect(sheet.queryByRole("combobox", { name: "Merge method" })).not.toBeInTheDocument();
    expect(sheet.getByRole("combobox", { name: "Member" })).toHaveValue("01J0DOT");
    await user.selectOptions(sheet.getByRole("combobox", { name: "Member" }), "reviewer");
    const job = sheet.getByRole("textbox", { name: "Job" });
    await user.clear(job);
    await user.type(job, "Tag a release.");
    expect(await saved(user, "ship")).toEqual({
      id: "ship",
      name: "Ship",
      kind: "deploy",
      mode: "member",
      member: "01J0DOT2",
      job: "Tag a release.",
    });

    sheet = await settings(user, "Ship", "Deploy settings");
    await user.selectOptions(sheet.getByRole("combobox", { name: "Mode" }), "Push");
    expect(sheet.queryByRole("combobox", { name: "Member" })).not.toBeInTheDocument();
    expect(sheet.getByText("Pushes the branches")).toBeVisible();
    expect(await saved(user, "ship")).toEqual({
      id: "ship",
      name: "Ship",
      kind: "deploy",
      mode: "push",
    });
  });

  it("adds, renames and removes a branch's paths, and says what Otherwise holds", async () => {
    const { user } = open(fix);
    const sheet = await settings(user, "Fixed?", "Branch settings");
    const on = sheet.getByRole("combobox", { name: "Goes by" });
    expect(on).toHaveValue("outcome");
    expect(
      sheet.getByText("A path's label is an outcome of Build: fixed, needs_design."),
    ).toBeVisible();
    expect(
      sheet.getByText("Otherwise holds 1 step, run when no path's label matches."),
    ).toBeVisible();
    expect(sheet.getByRole("textbox", { name: "Label of path 1" })).toHaveValue("fixed");

    // A new path takes the outcome no path has yet; its steps are added on its cards.
    await user.click(sheet.getByRole("button", { name: "Add path" }));
    const second = sheet.getByRole("textbox", { name: "Label of path 2" });
    expect(second).toHaveValue("needs_design");
    expect(second).toHaveAttribute("maxlength", "40");
    expect(
      screen.getByRole("region", { name: "Path needs_design", hidden: true }),
    ).toBeInTheDocument();
    await user.clear(second);
    await user.type(second, "later");
    expect(screen.getByRole("region", { name: "Path later", hidden: true })).toBeInTheDocument();

    // A path with steps on it asks before it goes; an empty one goes at once.
    await user.click(sheet.getByRole("button", { name: "Remove the path fixed" }));
    const ask = await screen.findByRole("alertdialog", { name: "Remove the path fixed?" });
    expect(ask).toHaveTextContent(
      "The 2 steps on it go with it. Nothing is saved until you press Save.",
    );
    await user.click(within(ask).getByRole("button", { name: "Remove path" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(sheet.getByRole("textbox", { name: "Label of path 1" })).toHaveValue("later");

    await user.selectOptions(sheet.getByRole("combobox", { name: "Goes by" }), "The project");
    expect(sheet.getByText("A path's label is a project's name: Sage, Site.")).toBeVisible();
    await user.click(sheet.getByRole("button", { name: "Remove the path later" }));
    expect(sheet.queryByRole("textbox", { name: "Label of path 1" })).not.toBeInTheDocument();
    await user.click(sheet.getByRole("button", { name: "Add path" }));
    expect(sheet.getByRole("textbox", { name: "Label of path 1" })).toHaveValue("Sage");

    expect(await saved(user, "route")).toMatchObject({
      on: "project",
      paths: [{ label: "Sage", steps: [] }],
      otherwise: [findStep(fix.definition.steps, "design")],
    });
  });

  it("says what a branch on outcome can go by when the step before declares none", async () => {
    const { user } = open(nightly);
    const sheet = await settings(user, "Which project", "Branch settings");
    expect(
      sheet.getByText(
        "Otherwise is empty: when no path's label matches, the workflow goes on after the branch.",
      ),
    ).toBeVisible();
    expect(sheet.getByText("A path's label is a project's name: Sage, Site.")).toBeVisible();
    await user.selectOptions(
      sheet.getByRole("combobox", { name: "Goes by" }),
      "The outcome reported before it",
    );
    // Work, the agent step before it, declares no outcome.
    expect(sheet.getByText("A path's label is an outcome of Work: it has none yet.")).toBeVisible();
  });
});

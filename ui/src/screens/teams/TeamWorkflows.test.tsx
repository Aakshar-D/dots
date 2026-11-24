import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Team, Workflow, WorkflowRun, WorkflowSpec } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [crew] = sample.teams as [Team, Team];
const [, pushIt, nightly] = sample.workflows as [Workflow, Workflow, Workflow, Workflow];
const [waiting, , , testRun, budget] = sample.workflowRuns as [
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
];

/// The crew's page with its four workflows, and the runs `runs`.
function crewPage(over: { team?: Team; runs?: WorkflowRun[] } = {}) {
  const team = over.team ?? crew;
  backend.on("team_get", () => team);
  backend.on("workflows_list", () => sample.workflows);
  backend.on("workflow_runs_list", () => over.runs ?? []);
  const view = renderApp(`/teams/${team.id}`);
  // Every path the app went to, the ones it was sent on from included.
  const visited: string[] = [];
  view.router.subscribe((state) => visited.push(state.location.pathname));
  return { ...view, visited };
}

const row = (name: string) =>
  within(screen.getByRole("region", { name: "Workflows" })).getByRole("listitem", { name });

describe("the team page's workflows", () => {
  it("lists each workflow with its trigger, switch, last run and problem", async () => {
    crewPage();
    const section = within(await screen.findByRole("region", { name: "Workflows" }));
    const fix = await section.findByRole("listitem", { name: "Fix and ship" });
    expect(within(fix).getByRole("link", { name: "Fix and ship" })).toHaveAttribute(
      "href",
      "/teams/01J0TEAM/workflows/01J0WFL",
    );
    expect(fix).toHaveTextContent("Started from a chat");
    expect(fix).toHaveTextContent("1 run in flight");
    expect(within(fix).getByRole("switch", { name: "Fix and ship is on" })).toBeChecked();
    expect(fix).toHaveTextContent("Last runWaiting");
    expect(within(fix).getByRole("link", { name: "The last run of Fix and ship" })).toHaveAttribute(
      "href",
      "/workflow-runs/01J0WFR",
    );
    expect(within(fix).getByRole("button", { name: "Run Fix and ship now" })).toBeEnabled();

    const push = row("Push it");
    expect(push).toHaveTextContent("Run by hand");
    expect(push).toHaveTextContent('step "Work": 01J0GONE is not a member of the team');
    expect(within(push).getByRole("switch", { name: "Push it is on" })).not.toBeChecked();
    expect(within(push).getByText("Test")).toBeVisible();
    expect(within(push).getByRole("button", { name: "Run Push it now" })).toBeDisabled();

    expect(row("Nightly")).toHaveTextContent("On a schedule");
    expect(row("Nightly")).toHaveTextContent("Not run yet");
    expect(row("On issue")).toHaveTextContent("On a webhook");
  });

  it("turns a workflow off with the rest of it as saved", async () => {
    backend.on("workflow_update", () => ({ ...nightly, enabled: false }));
    const { user } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(within(row("Nightly")).getByRole("switch", { name: "Nightly is on" }));
    expect(backend.callsTo("workflow_update")).toEqual([
      {
        id: "01J0WFL3",
        spec: { name: "Nightly", definition: nightly.definition, enabled: false },
      },
    ]);
  });

  it("makes a workflow done by the first member, off, and opens it in the builder", async () => {
    const made: Workflow = { ...pushIt, id: "01J0WFL9", name: "New workflow", problem: null };
    backend.on("workflow_create_draft", () => made);
    const { user, visited } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "New workflow" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New workflow" }));
    await user.click(dialog.getByRole("button", { name: "Start with an empty one" }));
    // What will be made is shown first.
    expect(within(dialog.getByRole("list", { name: "Its steps" })).getAllByRole("listitem"))
      .toHaveLength(1);
    await user.click(dialog.getByRole("button", { name: "Make it" }));
    await waitFor(() => expect(visited).toContain("/teams/01J0TEAM/workflows/01J0WFL9"));
    expect(backend.callsTo("workflow_create_draft")).toEqual([
      {
        teamId: "01J0TEAM",
        spec: {
          name: "New workflow",
          enabled: false,
          definition: {
            trigger: { kind: "manual" },
            steps: [
              {
                id: "work",
                name: "Work",
                kind: "agent",
                member: "01J0DOT",
                job: "Work on the task.",
                can_send_back_to: [],
                outcomes: [],
                delegate: null,
                options: { model: null, lean: false, reads: null },
                produces: [],
              },
            ],
            budget: { max_runs: 10, max_tokens: 1_500_000, max_minutes: 120, max_send_backs: 3 },
            inputs: [],
          },
        },
      },
    ]);
  });

  it("asks for a member before a new workflow, and runs one without a project", async () => {
    crewPage({ team: { ...crew, members: [], projects: [] } });
    const section = within(await screen.findByRole("region", { name: "Workflows" }));
    expect(section.getByRole("button", { name: "New workflow" })).toBeDisabled();
    // A run needs no project: it works in the team's own folder.
    expect(within(row("Fix and ship")).getByRole("button", { name: "Run Fix and ship now" }))
      .toBeEnabled();
  });

  it("runs a workflow with a task on a project, then opens the run's page", async () => {
    backend.on("workflow_run_start", () => waiting);
    const { user, visited } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "Run Fix and ship now" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Run Fix and ship" }));
    const start = dialog.getByRole("button", { name: "Start run" });
    expect(start).toBeDisabled();
    expect(dialog.getByLabelText("Project")).toHaveValue("01J0PRJ");
    await user.type(dialog.getByLabelText("Task"), "  Fix the tray icon.  ");
    await user.selectOptions(dialog.getByLabelText("Project"), "01J0PRJ2");
    await user.click(start);
    await waitFor(() => expect(visited).toContain("/workflow-runs/01J0WFR"));
    expect(backend.callsTo("workflow_run_start")).toEqual([
      { workflowId: "01J0WFL", projectId: "01J0PRJ2", task: "Fix the tray icon.", test: false },
    ]);
  });

  it("asks for the workflow's inputs, and runs in the team's own folder", async () => {
    backend.on("workflow_run_start", () => waiting);
    const { user, visited } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "Run Announce now" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Run Announce" }));
    const start = dialog.getByRole("button", { name: "Start run" });
    await user.type(dialog.getByLabelText("Task"), "Announce the launch.");
    // A needed input holds the start back, and says which.
    expect(start).toBeDisabled();
    expect(start).toHaveAttribute("title", `Give "Product name" first.`);
    await user.type(dialog.getByLabelText("Product name"), "Sage");
    // A default is filled in, and an optional one may stay empty.
    expect(dialog.getByLabelText("Notes (optional)")).toHaveValue("Keep it short.");
    // The wait's date and time is needed too.
    expect(start).toHaveAttribute("title", `Give "Publish at" first.`);
    await user.type(dialog.getByLabelText("Publish at"), "2026-12-01T09:00");
    await user.selectOptions(dialog.getByLabelText("Project"), "The team's own folder");
    await user.click(start);
    await waitFor(() => expect(visited).toContain("/workflow-runs/01J0WFR"));
    expect(backend.callsTo("workflow_run_start")).toEqual([
      {
        workflowId: "01J0WFL5",
        projectId: null,
        task: "Announce the launch.",
        test: false,
        inputs: { product: "Sage", notes: "Keep it short.", publish_at: "2026-12-01T09:00" },
      },
    ]);
  });

  it("writes a workflow from a description, shows it with what is left to do, and makes it", async () => {
    const made: Workflow = { ...pushIt, id: "01J0WFL9", problem: null };
    backend.on("workflow_draft", () => sample.workflowDraft);
    backend.on("workflow_create_draft", () => made);
    const { user, visited } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "New workflow" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New workflow" }));
    const write = dialog.getByRole("button", { name: "Write it for me" });
    expect(write).toBeDisabled();
    await user.type(dialog.getByLabelText("Describe it"), "Answer my clients on Telegram.");
    await user.click(write);
    const shown = within(await dialog.findByRole("region", { name: "The workflow to make" }));
    // The team has a workflow of that name: the new one's is made free.
    expect(shown.getByText("Answer clients 2")).toBeVisible();
    expect(
      within(shown.getByRole("list", { name: "Its steps" }))
        .getAllByRole("listitem")
        .map((step) => step.textContent),
    ).toEqual(["1.Write the answerAgent step", "2.Tell themAction"]);
    expect(shown.getByRole("list", { name: "Before it can run" })).toHaveTextContent(
      "Keep your bot's token as a secret variable named BOT_TOKEN.",
    );
    expect(backend.callsTo("workflow_draft")).toEqual([
      expect.objectContaining({ teamId: "01J0TEAM", description: "Answer my clients on Telegram." }),
    ]);
    await user.click(dialog.getByRole("button", { name: "Make it" }));
    await waitFor(() => expect(visited).toContain("/teams/01J0TEAM/workflows/01J0WFL9"));
    expect(backend.callsTo("workflow_create_draft")).toEqual([
      {
        teamId: "01J0TEAM",
        spec: {
          name: "Answer clients 2",
          definition: sample.workflowDraft.definition,
          enabled: false,
        },
      },
    ]);
  });

  it("says why a workflow could not be written, and keeps the description", async () => {
    backend.on("workflow_draft", () => {
      throw { kind: "other", message: "Claude took too long to write the workflow." };
    });
    const { user } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "New workflow" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New workflow" }));
    await user.type(dialog.getByLabelText("Describe it"), "Post every week.");
    await user.click(dialog.getByRole("button", { name: "Write it for me" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent(
      "Claude took too long to write the workflow.",
    );
    expect(dialog.getByLabelText("Describe it")).toHaveValue("Post every week.");
  });

  it("starts from a ready workflow, with the team's first member and what is left to do", async () => {
    const made: Workflow = { ...pushIt, id: "01J0WFL9", problem: null };
    backend.on("workflow_create_draft", () => made);
    const { user } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "New workflow" }));
    const dialog = within(await screen.findByRole("dialog", { name: "New workflow" }));
    const ready = within(dialog.getByRole("list", { name: "Ready workflows" }));
    expect(ready.getAllByRole("listitem")).toHaveLength(5);
    await user.click(ready.getByRole("button", { name: /Answer messages on Telegram/ }));
    expect(dialog.getByRole("list", { name: "Before it can run" })).toHaveTextContent(
      "Make a bot with @BotFather in Telegram",
    );
    // Back, and another one.
    await user.click(dialog.getByRole("button", { name: "Back" }));
    await user.click(
      within(dialog.getByRole("list", { name: "Ready workflows" })).getByRole("button", {
        name: /Write a post, approve it, publish it at a time/,
      }),
    );
    await user.click(dialog.getByRole("button", { name: "Make it" }));
    const [call] = backend.callsTo("workflow_create_draft") as { spec: WorkflowSpec }[];
    expect(call.spec.name).toBe("Post at a time");
    expect(call.spec.enabled).toBe(false);
    expect(call.spec.definition.steps.map((s) => s.kind)).toEqual([
      "agent",
      "check",
      "wait",
      "action",
    ]);
    expect(call.spec.definition.steps[0]).toMatchObject({ member: "01J0DOT", produces: ["post"] });
  });

  it("makes a copy, saves a workflow as a file, and brings one in", async () => {
    backend.on("workflow_create_draft", () => ({ ...nightly, id: "01J0WFL9" }));
    backend.on("dialog_save", () => "C:/out/Nightly.sage-workflow.json");
    backend.on("workflow_export", () => null);
    backend.on("dialog_open", () => "C:/in/theirs.json");
    backend.on("workflow_import", () => ({ ...nightly, id: "01J0WFL8", name: "Theirs" }));
    const { user, visited } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(within(row("Nightly")).getByRole("button", { name: "Make a copy of Nightly" }));
    await waitFor(() =>
      expect(backend.callsTo("workflow_create_draft")).toEqual([
        {
          teamId: "01J0TEAM",
          spec: { name: "Nightly copy", definition: nightly.definition, enabled: false },
        },
      ]),
    );
    await user.click(within(row("Nightly")).getByRole("button", { name: "Save Nightly as a file" }));
    await waitFor(() =>
      expect(backend.callsTo("workflow_export")).toEqual([
        { id: "01J0WFL3", path: "C:/out/Nightly.sage-workflow.json" },
      ]),
    );
    await user.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(visited).toContain("/teams/01J0TEAM/workflows/01J0WFL8"));
    expect(backend.callsTo("workflow_import")).toEqual([
      { teamId: "01J0TEAM", path: "C:/in/theirs.json" },
    ]);
  });

  it("keeps the run dialog open with the core's refusal", async () => {
    backend.on("workflow_run_start", () => {
      throw { kind: "invalid", message: "C:/Users/u/src/dots is not a git repository" };
    });
    const { user } = crewPage();
    await screen.findByRole("region", { name: "Workflows" });
    await user.click(screen.getByRole("button", { name: "Run Fix and ship now" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Run Fix and ship" }));
    await user.type(dialog.getByLabelText("Task"), "Fix it.");
    await user.click(dialog.getByRole("button", { name: "Start run" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("is not a git repository");
  });
});

describe("the team page's recent runs", () => {
  it("lists the newest runs with status, task, workflow, project and spend", async () => {
    crewPage({ runs: [waiting, testRun, budget] });
    const section = within(await screen.findByRole("region", { name: "Recent runs" }));
    const rows = await section.findAllByRole("row");
    // The header, then one row per run, newest first as the core answers.
    expect(rows).toHaveLength(4);
    expect(backend.callsTo("workflow_runs_list")).toContainEqual({
      filter: { team_id: "01J0TEAM", limit: 20 },
    });
    const first = within(rows[1]!);
    expect(rows[1]).toHaveTextContent("Needs your check");
    expect(first.getByRole("link", { name: "The tray icon is invisible on a light taskbar." }))
      .toHaveAttribute("href", "/workflow-runs/01J0WFR");
    expect(rows[1]).toHaveTextContent("Fix and ship");
    expect(rows[1]).toHaveTextContent("Sage");
    expect(rows[1]).toHaveTextContent("3 runs · 120k tokens");
    expect(rows[2]).toHaveTextContent("A step failed");
    expect(within(rows[2]!).getByText("Test")).toBeVisible();
    // Its workflow and project were deleted since; the run keeps the project's name.
    expect(rows[3]).toHaveTextContent("Hit its budget");
    expect(rows[3]).toHaveTextContent("A deleted workflow");
    expect(rows[3]).toHaveTextContent("Site");
  });

  it("says when the team has not run", async () => {
    crewPage();
    const section = within(await screen.findByRole("region", { name: "Recent runs" }));
    expect(await section.findByText("No runs yet.")).toBeVisible();
  });

  it("follows a workflow run's event: the runs and the workflows are read again", async () => {
    const { listening } = crewPage({ runs: [waiting] });
    await screen.findByRole("region", { name: "Recent runs" });
    await listening();
    const runs = backend.callsTo("workflow_runs_list").length;
    const workflows = backend.callsTo("workflows_list").length;
    backend.on("workflow_runs_list", () => [{ ...waiting, status: "succeeded", waiting_on: null }]);
    act(() =>
      backend.emit("runtime", {
        type: "workflow_run_updated",
        run: { ...waiting, status: "succeeded", waiting_on: null },
      }),
    );
    const section = within(screen.getByRole("region", { name: "Recent runs" }));
    expect(await section.findByText("Succeeded")).toBeVisible();
    expect(backend.callsTo("workflow_runs_list").length).toBe(runs + 1);
    expect(backend.callsTo("workflows_list").length).toBe(workflows + 1);
  });
});

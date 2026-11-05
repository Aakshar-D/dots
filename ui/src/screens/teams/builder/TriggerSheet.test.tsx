import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Team, Trigger, Workflow, WorkflowSpec } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [crew] = sample.teams as [Team, Team];
const [fix, pushIt, nightly, onIssue] = sample.workflows as [
  Workflow,
  Workflow,
  Workflow,
  Workflow,
];

/// The builder of `workflow`, with no problem in any draft, whose save answers with what it
/// was sent.
function open(workflow: Workflow) {
  backend.on("team_get", () => crew);
  backend.on("workflow_get", () => workflow);
  backend.on("workflow_check_spec", () => null);
  backend.on("workflow_update", (args) => ({ ...workflow, ...(args.spec as WorkflowSpec) }));
  return renderApp(`/teams/${crew.id}/workflows/${workflow.id}`);
}

type User = ReturnType<typeof open>["user"];

/// Opens the trigger's settings from its card and gives the sheet.
async function settings(user: User) {
  await user.click(await screen.findByRole("button", { name: "Settings of the trigger" }));
  return within(await screen.findByRole("dialog", { name: "Trigger settings" }));
}

/// Closes the sheet, saves the draft, and gives the trigger the save sent.
async function saved(user: User): Promise<Trigger> {
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  const save = screen.getByRole("button", { name: "Save" });
  await waitFor(() => expect(save).toBeEnabled());
  await user.click(save);
  return (backend.callsTo("workflow_update").at(-1)!.spec as WorkflowSpec).definition.trigger;
}

const trigger = () => screen.getByRole("group", { name: "Trigger", hidden: true });

describe("the trigger's settings sheet", () => {
  it("opens from the trigger card and picks the members who start it from a chat", async () => {
    const { user } = open(fix);
    // A click on the card, or Enter on it, opens the settings too.
    await user.click(await screen.findByText("When you chat with nightly-tests"));
    expect(await screen.findByRole("dialog", { name: "Trigger settings" })).toBeVisible();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    act(() => trigger().focus());
    await user.keyboard("{Enter}");
    const sheet = within(await screen.findByRole("dialog", { name: "Trigger settings" }));

    const kind = sheet.getByRole("combobox", { name: "What starts it" });
    expect(kind).toHaveValue("chat");
    expect(within(kind).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Run by hand",
      "Started from a chat",
      "On a schedule",
      "On a webhook",
    ]);
    const by = within(sheet.getByRole("group", { name: "Started by" }));
    expect(by.getByRole("checkbox", { name: "nightly-tests" })).toBeChecked();
    expect(by.getByRole("checkbox", { name: "reviewer" })).not.toBeChecked();
    await user.click(by.getByRole("checkbox", { name: "reviewer" }));
    expect(trigger()).toHaveTextContent("When you chat with nightly-tests, reviewer");
    // It is on: no note that the trigger starts nothing.
    expect(sheet.queryByRole("note")).not.toBeInTheDocument();
    expect(await saved(user)).toEqual({ kind: "chat", starters: ["01J0DOT", "01J0DOT2"] });
  });

  it("puts a workflow on a schedule with the schedule editor, a task and a project", async () => {
    const { user } = open(pushIt);
    const sheet = await settings(user);
    // Push it is off: its trigger starts nothing until it is on, and a Test run still works.
    expect(sheet.getByRole("note")).toHaveTextContent(
      "The workflow is off: this trigger starts nothing until you turn it on and save. A Test " +
        "run works while it is off.",
    );
    const kinds = sheet.getByRole("combobox", { name: "What starts it" });
    await user.selectOptions(kinds, "On a schedule");
    // Always a schedule, and never a catch-up (section 5.1).
    const kind = sheet.getByRole("combobox", { name: "Kind" });
    expect(kind).toHaveValue("daily");
    expect(within(kind).queryByRole("option", { name: "None" })).not.toBeInTheDocument();
    expect(sheet.queryByLabelText("Catch up a missed run")).not.toBeInTheDocument();
    expect(await sheet.findByRole("list", { name: "Next runs" })).toBeInTheDocument();
    const time = sheet.getByLabelText("Time");
    await user.clear(time);
    await user.type(time, "07:30");

    const task = sheet.getByRole("textbox", { name: "Task" });
    expect(task).toHaveAttribute("maxlength", "8000");
    await user.type(task, "Update the dependencies.");
    const project = sheet.getByRole("combobox", { name: "Project" });
    expect(project).toHaveValue("Dots");
    await user.selectOptions(project, "Site");
    expect(trigger()).toHaveTextContent("Update the dependencies. · on Site");
    expect(await saved(user)).toEqual({
      kind: "schedule",
      schedule: { rule: { kind: "daily", time: "07:30" }, catch_up: false, until: null },
      task: "Update the dependencies.",
      project: "Site",
    });
    expect(backend.callsTo("preview_schedule").at(-1)).toMatchObject({ count: 3 });
  });

  it("shows a webhook's URL, makes a new token shown once, and sets its fallbacks", async () => {
    const { user } = open(onIssue);
    const sheet = await settings(user);
    expect(await sheet.findByRole("textbox", { name: "Webhook URL" })).toHaveValue(
      "http://127.0.0.1:47321/workflows/01J0WFL4/trigger",
    );
    expect(sheet.getByRole("button", { name: "Copy the webhook URL" })).toBeVisible();
    expect(
      sheet.getByText("This workflow has a token. It was shown once, when it was made."),
    ).toBeVisible();
    // Saved with this trigger: calls reach it.
    expect(sheet.queryByRole("note")).not.toBeInTheDocument();

    backend.on("regenerate_workflow_token", () => "wf-token-2");
    await user.click(sheet.getByRole("button", { name: "Regenerate" }));
    const ask = await screen.findByRole("alertdialog", { name: "Make a new token?" });
    await user.click(within(ask).getByRole("button", { name: "Make a new token" }));
    expect(await sheet.findByRole("textbox", { name: "Token" })).toHaveValue("wf-token-2");
    expect(sheet.getByText("Copy it now: you won't see this again.")).toBeVisible();
    expect(backend.callsTo("regenerate_workflow_token")).toEqual([{ id: "01J0WFL4" }]);

    const task = sheet.getByRole("textbox", { name: "Task when the call has none" });
    expect(task).toHaveValue("Look at the newest issue.");
    await user.clear(task);
    const project = sheet.getByRole("combobox", { name: "Project when the call has none" });
    expect(project).toHaveValue("");
    expect(within(project).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "None: the call names it",
      "Dots",
      "Site",
    ]);
    await user.selectOptions(project, "Site");
    expect(trigger()).toHaveTextContent("The call says the task");
    expect(await saved(user)).toEqual({ kind: "webhook", task: null, project: "Site" });
    // The token is shown once: the sheet opened again does not show it.
    const again = await settings(user);
    expect(again.queryByRole("textbox", { name: "Token" })).not.toBeInTheDocument();
    expect(
      again.getByText("This workflow has a token. It was shown once, when it was made."),
    ).toBeVisible();
  });

  it("says a webhook not saved yet refuses calls, and why its URL is missing", async () => {
    backend.on("workflow_webhook_url", () => {
      throw { kind: "conflict", message: "the local server is not running" };
    });
    const { user } = open(nightly);
    const sheet = await settings(user);
    const kinds = sheet.getByRole("combobox", { name: "What starts it" });
    await user.selectOptions(kinds, "On a webhook");
    expect(sheet.getByRole("note")).toHaveTextContent(
      "Calls are refused until the workflow is saved with this trigger.",
    );
    expect(await sheet.findByText("the local server is not running")).toBeVisible();
    expect(
      sheet.getByText("This workflow has no token yet, so its webhook refuses every call."),
    ).toBeVisible();
    backend.on("regenerate_workflow_token", () => "wf-token-1");
    await user.click(sheet.getByRole("button", { name: "Make a token" }));
    expect(await sheet.findByRole("textbox", { name: "Token" })).toHaveValue("wf-token-1");
    // The schedule's task and project carry over as the fallbacks.
    expect(sheet.getByRole("textbox", { name: "Task when the call has none" })).toHaveValue(
      "Update the dependencies.",
    );
    expect(sheet.getByRole("combobox", { name: "Project when the call has none" })).toHaveValue(
      "Dots",
    );
    expect(await saved(user)).toEqual({
      kind: "webhook",
      task: "Update the dependencies.",
      project: "Dots",
    });
  });
});

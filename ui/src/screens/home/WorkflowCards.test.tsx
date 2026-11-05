import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Team, Workflow, WorkflowRun } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply, said } from "@/test/home";
import { renderApp } from "@/test/render";

const [crew] = sample.teams as [Team, Team];
const [fixAndShip] = sample.workflows as [Workflow, Workflow, Workflow, Workflow];
const [waiting] = sample.workflowRuns as [WorkflowRun];
const CARD = "Workflow run: The tray icon is invisible on a light taskbar.";

/// A chat topic of two turns, the first of which started crew's Fix and ship, waiting on the
/// check; `started` holds what `workflow_runs_for_origins` answers.
function topic() {
  const first = chat("current", "10:00");
  const second = reply("current2", first, "10:05");
  home(
    [first, second],
    [said("current", 1, "I handed it to the team."), said("current2", 1, "It waits on you.")],
  );
  const started: { runs: WorkflowRun[] } = { runs: [{ ...waiting, origin_run_id: "current" }] };
  backend.on("workflow_runs_for_origins", () => started.runs);
  backend.on("team_get", () => crew);
  backend.on("workflow_get", () => fixAndShip);
  backend.on("workflow_check", () => sample.check);
  return { started, view: renderApp(`/home/${dot.id}`) };
}

const conversation = () => screen.findByRole("region", { name: "Conversation" });
const follows = (a: Node, b: Node) =>
  (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;

describe("a workflow started from a chat", () => {
  it("shows its card after the turn whose run started it, from one query for the topic", async () => {
    const { view } = topic();
    const shown = within(await conversation());
    const card = await shown.findByRole("article", { name: CARD });
    expect(await within(card).findByText("· Fix and ship")).toBeVisible();
    // After the first turn's words, before the second turn.
    expect(follows(shown.getByText("I handed it to the team."), card)).toBe(true);
    expect(follows(card, shown.getByText("reply current2"))).toBe(true);
    expect(shown.getAllByRole("article", { name: CARD })).toHaveLength(1);
    await view.listening();
    const calls = backend.callsTo("workflow_runs_for_origins");
    expect(calls.length).toBeGreaterThan(0);
    for (const call of calls) expect(call).toEqual({ runIds: ["current", "current2"] });
  });

  it("acts on the workflow only: approving does not wake the chat's dot", async () => {
    const { started, view } = topic();
    const card = within(await within(await conversation()).findByRole("article", { name: CARD }));
    await view.listening();
    const moved: WorkflowRun = { ...started.runs[0]!, status: "running", waiting_on: null };
    backend.on("workflow_approve", () => moved);
    started.runs = [moved];
    await view.user.click(await card.findByRole("button", { name: "Approve" }));
    expect(await card.findByText("Running")).toHaveAttribute("data-status", "running");
    expect(backend.callsTo("workflow_approve")).toHaveLength(1);
    expect(backend.callsTo("chat")).toEqual([]);
  });

  it("follows the workflow run's events", async () => {
    const { started, view } = topic();
    const card = within(await within(await conversation()).findByRole("article", { name: CARD }));
    expect(await card.findByRole("region", { name: "Your check" })).toBeVisible();
    await view.listening();
    const finished: WorkflowRun = {
      ...started.runs[0]!,
      status: "succeeded",
      waiting_on: null,
      current_step: null,
    };
    started.runs = [finished];
    act(() => backend.emit("runtime", { type: "workflow_run_updated", run: finished }));
    expect(await card.findByText("Succeeded")).toHaveAttribute("data-status", "succeeded");
    await waitFor(() => expect(card.queryByRole("region", { name: "Your check" })).toBeNull());
    expect(card.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("keeps the cards while a new run joins the conversation and its answer is pending", async () => {
    const { started, view } = topic();
    const shown = within(await conversation());
    await shown.findByRole("article", { name: CARD });
    await view.listening();
    // The answer for the longer list of ids is held back.
    let release: (runs: WorkflowRun[]) => void = () => {};
    const held = new Promise<WorkflowRun[]>((resolve) => (release = resolve));
    backend.on("workflow_runs_for_origins", (args) =>
      (args.runIds as string[]).length > 2 ? held : started.runs,
    );
    const third = reply("current3", chat("current", "10:00"), "10:10");
    backend.on("list_runs", () => [third, reply("current2", chat("current", "10:00"), "10:05"), chat("current", "10:00")]);
    backend.on("run_thread", () => [
      chat("current", "10:00"),
      reply("current2", chat("current", "10:00"), "10:05"),
      third,
    ]);
    act(() => backend.emit("runtime", { type: "run_updated", run: third }));
    await waitFor(() =>
      expect(backend.callsTo("workflow_runs_for_origins").at(-1)).toEqual({
        runIds: ["current", "current2", "current3"],
      }),
    );
    expect(shown.getByRole("article", { name: CARD })).toBeVisible();
    await act(async () => release(started.runs));
    expect(await shown.findByRole("article", { name: CARD })).toBeVisible();
  });
});

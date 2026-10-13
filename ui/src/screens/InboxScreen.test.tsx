import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [approval] = sample.inbox.approvals;
const [worktreeRun] = sample.inbox.review_runs; // succeeded, left a worktree
const failedRun = {
  ...worktreeRun,
  id: "run-failed",
  status: "failed" as const,
  branch: null,
  worktree_exists: false,
  summary: null,
  error: "timeout",
};

/// An inbox the test can empty, the way a decision or a review does.
function inbox(initial: Inbox) {
  let current = initial;
  backend.on("inbox", () => current);
  return {
    set: (next: Inbox) => {
      current = next;
    },
  };
}

describe("the Inbox", () => {
  it("lists what waits: approvals with their dot, and runs to review", async () => {
    inbox({ approvals: [approval], review_runs: [worktreeRun, failedRun] });
    renderApp("/inbox");
    const approvals = await screen.findByRole("region", { name: "Approvals" });
    expect(within(approvals).getByRole("link", { name: approval.dot_name })).toHaveAttribute(
      "href",
      `/dots/${approval.dot_id}`,
    );
    expect(within(approvals).getByRole("link", { name: "Open the run" })).toHaveAttribute(
      "href",
      `/runs/${approval.run_id}`,
    );
    expect(within(approvals).getByRole("article", { name: "Approval for Bash" })).toBeVisible();

    const runs = within(screen.getByRole("region", { name: "Runs to review" })).getAllByRole(
      "listitem",
    );
    expect(runs).toHaveLength(2);
    expect(within(runs[0]).getByText("Succeeded")).toBeVisible();
    expect(within(runs[0]).getByRole("link", { name: "Open" })).toHaveAttribute(
      "href",
      `/runs/${worktreeRun.id}`,
    );
    expect(within(runs[1]).getByText(/timeout/)).toBeVisible();
    // Only a run with a worktree can be discarded.
    expect(within(runs[0]).getByRole("button", { name: "Discard worktree" })).toBeVisible();
    expect(within(runs[1]).queryByRole("button", { name: "Discard worktree" })).toBeNull();
  });

  it("offers no discard for a failed run whose worktree is already gone", async () => {
    inbox({
      approvals: [],
      review_runs: [{ ...failedRun, branch: "dots/gone" }],
    });
    renderApp("/inbox");
    expect(await screen.findByRole("button", { name: "Mark reviewed" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
  });

  it("approves a tool call and drops it from the list", async () => {
    const box = inbox({ approvals: [approval], review_runs: [] });
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.approval, status: "approved" };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("decide_approval")).toEqual([
      { id: approval.id, approved: true, note: null },
    ]);
  });

  /// Lets what a command's answer set off reach the screen.
  const settle = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));

  it("keeps both buttons of a decided approval off until the list has reloaded", async () => {
    inbox({ approvals: [approval], review_runs: [] });
    backend.on("decide_approval", () => ({ ...sample.approval, status: "approved" }));
    const { user } = renderApp("/inbox");
    const approve = await screen.findByRole("button", { name: "Approve" });

    // From now on the Inbox's reload is held until the test lets it answer.
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    const loads = backend.callsTo("inbox").length;
    backend.on("inbox", async () => {
      await held;
      return { approvals: [], review_runs: [] };
    });

    await user.click(approve);
    // The decision has succeeded: the reload it starts is on its way.
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(loads));
    await settle();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();

    act(() => release());
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  // A double-click is two clicks at one place. The first removes its card or row, the next one
  // moves up under the pointer, and the second click (`detail` 2) lands on that one. user-event
  // cannot say this: it counts clicks per element.
  it.each(["Approve", "Deny"])(
    "decides one approval when %s is double-clicked and the next card moves up",
    async (button) => {
      const next = { ...approval, id: "apr-next", tool: "Write", input: { file_path: "a.txt" } };
      const box = inbox({ approvals: [approval, next], review_runs: [] });
      backend.on("decide_approval", (args) => {
        box.set({ approvals: [next], review_runs: [] });
        return { ...sample.approval, id: args.id, status: "approved" };
      });
      const { user } = renderApp("/inbox");
      const first = await screen.findByRole("article", { name: "Approval for Bash" });
      fireEvent.click(within(first).getByRole("button", { name: button }), { detail: 1 });
      await waitFor(() =>
        expect(screen.queryByRole("article", { name: "Approval for Bash" })).toBeNull(),
      );
      const moved = within(screen.getByRole("article", { name: "Approval for Write" }));
      fireEvent.click(moved.getByRole("button", { name: button }), { detail: 2 });
      await settle();
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: approval.id, approved: button === "Approve", note: null },
      ]);

      // A click of its own decides the card that moved up.
      await user.click(moved.getByRole("button", { name: button }));
      await waitFor(() => expect(backend.callsTo("decide_approval")).toHaveLength(2));
      expect(backend.callsTo("decide_approval")[1]).toMatchObject({ id: next.id });
    },
  );

  it("marks a run as reviewed and drops it from the list", async () => {
    const box = inbox({ approvals: [], review_runs: [failedRun] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.run, id: failedRun.id, reviewed_at: sample.run.ended_at };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Mark reviewed" }));
    expect(await screen.findByText("Marked as reviewed")).toBeVisible();
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("mark_reviewed")).toEqual([{ runId: failedRun.id }]);
  });

  it("marks one run as reviewed when the button is double-clicked and the next row moves up", async () => {
    const next = { ...failedRun, id: "run-next" };
    const box = inbox({ approvals: [], review_runs: [failedRun, next] });
    backend.on("mark_reviewed", (args) => {
      box.set({ approvals: [], review_runs: [next] });
      return { ...sample.run, id: args.runId, reviewed_at: sample.run.ended_at };
    });
    renderApp("/inbox");
    const rows = await screen.findAllByRole("listitem", { name: /run to review/ });
    fireEvent.click(within(rows[0]).getByRole("button", { name: "Mark reviewed" }), { detail: 1 });
    await waitFor(() =>
      expect(screen.getAllByRole("listitem", { name: /run to review/ })).toHaveLength(1),
    );
    fireEvent.click(screen.getByRole("button", { name: "Mark reviewed" }), { detail: 2 });
    await settle();
    expect(backend.callsTo("mark_reviewed")).toEqual([{ runId: failedRun.id }]);
  });

  it("discards a worktree only after a confirmation", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [] });
      return { ...sample.run, reviewed_at: sample.run.ended_at };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(worktreeRun.branch!);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("discard_worktree")).toEqual([]);

    await user.click(screen.getByRole("button", { name: "Discard worktree" }));
    await user.click(await screen.findByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText("Worktree discarded")).toBeVisible();
    expect(backend.callsTo("discard_worktree")).toEqual([{ runId: worktreeRun.id }]);
  });

  it("shows a refused action and keeps the run in the list", async () => {
    inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("mark_reviewed", () => {
      throw { kind: "conflict", message: "the thread is still running" };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Mark reviewed" }));
    expect(await screen.findByText("the thread is still running")).toBeVisible();
    expect(screen.getByRole("listitem", { name: /run to review/ })).toBeVisible();
  });

  it("drops a run whose review was refused once the Inbox has reloaded without it", async () => {
    // The run is gone (its dot was deleted elsewhere): the core refuses, and the Inbox it
    // answers next no longer holds the run.
    const box = inbox({ approvals: [], review_runs: [failedRun] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [] });
      throw { kind: "not_found", message: `run ${failedRun.id}` };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Mark reviewed" }));
    expect(await screen.findByText(`run ${failedRun.id}`)).toBeVisible();
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  it("drops a run whose discard was refused once the Inbox has reloaded without it", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [] });
      throw { kind: "not_found", message: `run ${worktreeRun.id}` };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    await user.click(await screen.findByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText(`run ${worktreeRun.id}`)).toBeVisible();
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  it("shows a new approval as soon as the dot asks", async () => {
    const box = inbox({ approvals: [], review_runs: [] });
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(0));
    box.set({ approvals: [approval], review_runs: [] });
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
  });
});

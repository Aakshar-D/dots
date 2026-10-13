import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval, Run, RunEventRecord } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const run = sample.run; // a succeeded scheduled worktree run
const folderRun: Run = { ...run, branch: null, base_commit: null };
let seq = 0;
const ev = (data: Record<string, unknown>, runId = run.id): RunEventRecord => ({
  run_id: runId,
  seq: ++seq,
  ts: run.queued_at,
  kind: String(data.kind),
  data,
});

/// Answers the run view's queries for one thread.
function thread(runs: Run[], events: RunEventRecord[] = [], approvals: Approval[] = []) {
  backend.on("run_thread", () => runs);
  backend.on("get_dot", () => sample.dot);
  backend.on("list_run_events", (args) =>
    events.filter((e) => e.run_id === args.runId && e.seq > (args.afterSeq as number)),
  );
  backend.on("approvals_for_run", (args) => approvals.filter((a) => a.run_id === args.runId));
  backend.on("worktree_diff", () => sample.diff);
}

describe("the run view", () => {
  it("shows the run's state, numbers and summary", async () => {
    thread([run]);
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("heading", { name: `${sample.dot.name} run` })).toBeVisible();
    expect(screen.getByText("Succeeded")).toBeVisible();
    expect(screen.getByText("1200 in, 340 out")).toBeVisible();
    expect(screen.getByText(run.branch!)).toBeVisible();
    expect(within(screen.getByRole("region", { name: "Summary" })).getByText(run.summary!)).toBeVisible();
    expect(screen.getByRole("link", { name: sample.dot.name })).toHaveAttribute(
      "href",
      `/dots/${sample.dot.id}`,
    );
  });

  it("shows the conversation with each tool call folded around its result", async () => {
    thread(
      [run],
      [
        ev({ kind: "assistant_text", text: "Running the **tests**." }),
        ev({ kind: "tool_call", id: "t1", tool: "Bash", input: { command: "cargo test" } }),
        ev({ kind: "tool_result", id: "t1", output: "test result: ok", is_error: false }),
        ev({ kind: "message", message: { role: "user", content: "hidden" } }),
        ev({ kind: "raw", line: "garbled" }),
      ],
    );
    const { user } = renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("tests")).toBeVisible();
    const call = screen.getByText("Bash").closest("details")!;
    expect(call).not.toHaveAttribute("open");
    expect(within(call).getByText("cargo test")).toBeInTheDocument();
    expect(within(call).getByText("done")).toBeInTheDocument();
    await user.click(screen.getByText("Bash"));
    expect(call).toHaveAttribute("open");
    expect(within(call).getByText("test result: ok")).toBeInTheDocument();
    expect(screen.queryByText("hidden")).toBeNull();
    expect(screen.getByText("Unparsed output").closest("details")).not.toHaveAttribute("open");
  });

  it("follows a live run: new events and the new status arrive without a reload", async () => {
    const live: Run = { ...folderRun, status: "running", ended_at: null, summary: null };
    thread([live], [ev({ kind: "assistant_text", text: "Starting." })]);
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("Starting.")).toBeVisible();
    expect(screen.getByText("Running")).toBeVisible();
    const loads = backend.callsTo("list_run_events").length;

    act(() =>
      backend.emit("runtime", {
        type: "run_event",
        event: ev({ kind: "assistant_text", text: "Half way." }),
      }),
    );
    expect(await screen.findByText("Half way.")).toBeVisible();
    // The event was appended, not fetched.
    expect(backend.callsTo("list_run_events")).toHaveLength(loads);

    const done: Run = { ...live, status: "succeeded", ended_at: run.ended_at, summary: "Done." };
    backend.on("run_thread", () => [done]);
    act(() => backend.emit("runtime", { type: "run_updated", run: done }));
    expect(await screen.findByText("Succeeded")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Cancel run" })).toBeNull();
  });

  it("cancels the run that is still going", async () => {
    const live: Run = { ...folderRun, status: "running", ended_at: null };
    thread([live]);
    backend.on("cancel_run", () => ({ ...live, status: "cancelled" }));
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Cancel run" }));
    await waitFor(() => expect(backend.callsTo("cancel_run")).toEqual([{ runId: live.id }]));
  });

  it("offers no review or discard while a worktree run is still going", async () => {
    const live: Run = { ...run, status: "running", ended_at: null, summary: null };
    thread([live]);
    const { user } = renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("button", { name: "Cancel run" })).toBeVisible();
    // Judge only once the diff has arrived and said that the worktree exists.
    await user.click(screen.getByRole("tab", { name: "Changes" }));
    expect(await screen.findByText("src/lib.rs")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
  });

  it("offers neither discard nor review for a worktree that is already gone", async () => {
    thread([run]);
    backend.on("worktree_diff", () => ({ files: [], truncated: false, exists: false }));
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    expect(
      await screen.findByText(
        "This run's worktree was removed: it had no changes, or it was discarded.",
      ),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
  });

  it("shows what a worktree run changed", async () => {
    thread([run]);
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    expect(await screen.findByText("src/lib.rs")).toBeVisible();
    expect(screen.getByText("Binary file.")).toBeVisible();
    expect(backend.callsTo("worktree_diff")).toEqual([{ runId: run.id }]);
  });

  it("has no changes tab, discard or review for a folder run that succeeded", async () => {
    thread([folderRun]);
    renderApp(`/runs/${run.id}`);
    await screen.findByRole("heading", { name: `${sample.dot.name} run` });
    expect(screen.queryByRole("tab", { name: "Changes" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
    expect(backend.callsTo("worktree_diff")).toEqual([]);
    expect(screen.getByRole("button", { name: "Open folder" })).toBeVisible();
  });

  it("discards the worktree only after a confirmation that names the branch", async () => {
    thread([run]);
    backend.on("discard_worktree", () => ({ ...run, reviewed_at: run.ended_at }));
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Discard this worktree?" });
    expect(dialog).toHaveTextContent(run.branch!);
    expect(backend.callsTo("discard_worktree")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText("Worktree discarded")).toBeVisible();
    expect(backend.callsTo("discard_worktree")).toEqual([{ runId: run.id }]);
  });

  it("marks a failed run as reviewed and shows why it failed", async () => {
    const failed: Run = { ...folderRun, status: "failed", summary: null, error: "timeout" };
    thread([failed]);
    backend.on("mark_reviewed", () => ({ ...failed, reviewed_at: run.ended_at }));
    const { user } = renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("timeout");
    await user.click(screen.getByRole("button", { name: "Mark reviewed" }));
    expect(await screen.findByText("Marked as reviewed")).toBeVisible();
    expect(backend.callsTo("mark_reviewed")).toEqual([{ runId: run.id }]);
  });

  it("opens the run's folder, and says so when it is gone", async () => {
    thread([run]);
    backend.on("open_run_folder", () => null);
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Open folder" }));
    await waitFor(() => expect(backend.callsTo("open_run_folder")).toEqual([{ runId: run.id }]));

    backend.on("open_run_folder", () => {
      throw { kind: "not_found", message: "the run's folder no longer exists" };
    });
    await user.click(screen.getByRole("button", { name: "Open folder" }));
    expect(await screen.findByText("the run's folder no longer exists")).toBeVisible();
  });

  it("asks for a pending approval in place, and sends the decision with its note", async () => {
    const waiting: Run = { ...folderRun, status: "awaiting_approval", ended_at: null };
    thread([waiting], [], [sample.approval]);
    backend.on("decide_approval", () => ({ ...sample.approval, status: "denied" }));
    const { user } = renderApp(`/runs/${run.id}`);
    const card = await screen.findByRole("article", { name: "Approval for Bash" });
    expect(within(card).getAllByText(/git push origin main/)[0]).toBeVisible();
    await user.type(within(card).getByLabelText("Note for the dot"), "not on main");
    await user.click(within(card).getByRole("button", { name: "Deny" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: sample.approval.id, approved: false, note: "not on main" },
      ]),
    );
  });

  it("shows a thread of several runs as one conversation", async () => {
    const parent: Run = { ...folderRun, id: "run-a", trigger: "chat", payload: { message: "Fix the bug." } };
    const child: Run = {
      ...folderRun,
      id: "run-b",
      trigger: "resume",
      parent_run_id: "run-a",
      root_run_id: "run-a",
      tokens_in: 10,
      tokens_out: 5,
    };
    thread(
      [{ ...parent, root_run_id: "run-a" }, child],
      [
        ev({ kind: "assistant_text", text: "I need to push." }, "run-a"),
        ev({ kind: "assistant_text", text: "Pushed." }, "run-b"),
      ],
    );
    const { listening } = renderApp("/runs/run-b");
    expect(await screen.findByText("Fix the bug.")).toBeVisible();
    await screen.findByText("Pushed.");
    const sections = screen.getAllByRole("region", { name: /Chat message|Continued after approval/ });
    expect(sections).toHaveLength(2);
    expect(within(sections[0]).getByText("I need to push.")).toBeVisible();
    expect(within(sections[1]).getByText("Pushed.")).toBeVisible();
    expect(screen.getByText("1210 in, 345 out")).toBeVisible();
    // The thread is asked for by the run in the address, and again once the app listens.
    await listening();
    expect(backend.callsTo("run_thread")).toEqual([{ runId: "run-b" }, { runId: "run-b" }]);
  });

  it("renders model text without live links, images or raw HTML", async () => {
    thread(
      [folderRun],
      [
        ev({
          kind: "assistant_text",
          text: 'See [the docs](https://example.com/x) ![pic](https://example.com/p.png) <script>alert(1)</script><b id="raw">bold</b>',
        }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    const docs = await screen.findByText("the docs");
    expect(docs.tagName).toBe("SPAN");
    expect(docs).toHaveAttribute("title", "https://example.com/x");
    expect(document.querySelector("main a[href^='http']")).toBeNull();
    expect(document.querySelector("main img")).toBeNull();
    expect(document.querySelector("script")).toBeNull();
    expect(document.getElementById("raw")).toBeNull();
  });

  it("shows a failed load of the run's events", async () => {
    thread([folderRun], [], [sample.approval]);
    backend.on("list_run_events", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    // What did load is still shown.
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
  });

  it("shows a failed load of the run's approvals", async () => {
    thread([folderRun], [ev({ kind: "assistant_text", text: "Starting." })]);
    // An answer this UI cannot read.
    backend.on("approvals_for_run", () => [{ id: 7 }]);
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("alert")).toHaveTextContent(/^approvals_for_run: unexpected/);
    expect(await screen.findByText("Starting.")).toBeVisible();
  });

  it("shows one failure once when it stops the same load for every run of the thread", async () => {
    const child: Run = { ...folderRun, id: "run-b", parent_run_id: run.id, trigger: "resume" };
    thread([folderRun, child]);
    backend.on("list_run_events", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    await waitFor(() => expect(backend.callsTo("list_run_events")).toHaveLength(2));
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("shows why a run could not be loaded", async () => {
    backend.on("run_thread", () => {
      throw { kind: "not_found", message: "run nope" };
    });
    renderApp("/runs/nope");
    expect(await screen.findByRole("alert")).toHaveTextContent("run nope");
  });
});

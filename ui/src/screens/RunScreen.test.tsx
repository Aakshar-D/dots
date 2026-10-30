import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ApplyState, Approval, Run, RunEventRecord, WorktreeDiff } from "@/api/types";
import { localDateTime, localTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const run = sample.run; // a succeeded scheduled worktree run
const folderRun: Run = { ...run, branch: null, base_commit: null };
/// What the core answers for a worktree that was removed.
const gone: WorktreeDiff = { files: [], truncated: false, exists: false };
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

/// Lets what a command's answer set off reach the screen.
const settle = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));

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

  it("shows a chat run's tool steps as chips, with its header and the dot's name label", async () => {
    const asked: Run = { ...folderRun, trigger: "chat", payload: { message: "Run the tests." } };
    thread(
      [asked],
      [
        ev({ kind: "tool_call", id: "t1", tool: "Read", input: { file_path: "a.rs" } }),
        ev({ kind: "tool_result", id: "t1", output: "fn a", is_error: false }),
        ev({ kind: "tool_call", id: "t2", tool: "Bash", input: { command: "cargo test" } }),
        ev({ kind: "tool_result", id: "t2", output: "ok", is_error: false }),
        ev({ kind: "assistant_text", text: "The tests pass." }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("The tests pass.")).toBeVisible();
    // The conversation's look is the home's: the run screen keeps its own.
    const chips = screen.getByRole("group", { name: "Tool steps" });
    expect(within(chips).getByText("Read")).toBeVisible();
    expect(within(chips).getByText("Bash")).toBeVisible();
    expect(chips.closest("details")).toBeNull();
    expect(screen.queryByText("Read a file · Ran a command")).toBeNull();
    expect(screen.getByText("Chat message")).toBeVisible();
    expect(screen.getByText(`${sample.dot.name} · ${localTime(asked.queued_at)}`)).toBeVisible();
    expect(screen.getByText("Run the tests.")).toHaveClass("max-w-[80%]");
  });

  it("names an MCP call by its server and tool, a plugin's server by its plugin", async () => {
    backend.on("library_list", () => sample.library);
    const [, pluginServer] = sample.library;
    thread(
      [run],
      [
        ev({ kind: "tool_call", id: "m1", tool: "mcp__github__search_issues", input: { query: "bug" } }),
        ev({ kind: "tool_result", id: "m1", output: "3 issues", is_error: false }),
        ev({ kind: "tool_call", id: "m2", tool: `mcp__${pluginServer!.name}__page`, input: { n: 1 } }),
        ev({ kind: "tool_call", id: "m3", tool: "mcp__unknown__ping", input: {} }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("github › search_issues")).toBeVisible();
    expect(await screen.findByText("pdf-tools › page")).toBeVisible();
    expect(screen.getByText("unknown › ping")).toBeVisible();
    expect(screen.queryByText("mcp__github__search_issues")).toBeNull();
  });

  it("reads a notice as a note, not as a raw item", async () => {
    thread([run], [ev({ kind: "notice", text: "MCP server github: its tools changed, they ask first" })]);
    renderApp(`/runs/${run.id}`);
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("MCP server github: its tools changed, they ask first");
    expect(screen.queryByText("Note")?.closest("details")).toBeFalsy();
  });

  it("says that a tool call without a result is running only while its run is going", async () => {
    // The first run was stopped in the middle of a call; the second is in the middle of one.
    const stopped: Run = { ...folderRun, id: "run-a", status: "failed", error: "timeout after 5s" };
    const live: Run = {
      ...folderRun,
      id: "run-b",
      parent_run_id: "run-a",
      trigger: "resume",
      status: "running",
      ended_at: null,
      summary: null,
    };
    thread(
      [stopped, live],
      [
        ev({ kind: "tool_call", id: "t1", tool: "Bash", input: { command: "sleep 60" } }, "run-a"),
        ev({ kind: "tool_call", id: "t2", tool: "Read", input: { file_path: "a.rs" } }, "run-b"),
      ],
    );
    renderApp("/runs/run-b");
    const first = (await screen.findByText("Bash")).closest("details")!;
    const second = (await screen.findByText("Read")).closest("details")!;
    expect(within(first).getByText("no result")).toBeInTheDocument();
    expect(within(first).queryByText("running")).toBeNull();
    expect(within(second).getByText("running")).toBeInTheDocument();

    const cancelled: Run = { ...live, status: "cancelled", ended_at: run.ended_at };
    backend.on("run_thread", () => [stopped, cancelled]);
    act(() => backend.emit("runtime", { type: "run_updated", run: cancelled }));
    expect(
      await within(screen.getByText("Read").closest("details")!).findByText("no result"),
    ).toBeInTheDocument();
    expect(screen.queryByText("running")).toBeNull();
  });

  it("says how long a run has been going, and how long it took once it has ended", async () => {
    const started = Date.now() - 4000;
    const live: Run = {
      ...folderRun,
      status: "running",
      started_at: new Date(started).toISOString(),
      ended_at: null,
      summary: null,
    };
    thread([live]);
    renderApp(`/runs/${run.id}`);
    expect((await screen.findByText("Running for")).nextElementSibling).toHaveTextContent(/^\ds$/);
    expect(screen.queryByText("Took")).toBeNull();

    const done: Run = {
      ...live,
      status: "succeeded",
      ended_at: new Date(started + 300_000).toISOString(),
    };
    backend.on("run_thread", () => [done]);
    act(() => backend.emit("runtime", { type: "run_updated", run: done }));
    expect((await screen.findByText("Took")).nextElementSibling).toHaveTextContent("5m 00s");
    expect(screen.queryByText("Running for")).toBeNull();
  });

  it("does not say that a run that waits to start is running", async () => {
    const queued: Run = {
      ...folderRun,
      status: "queued",
      started_at: null,
      ended_at: null,
      summary: null,
    };
    thread([queued]);
    renderApp(`/runs/${run.id}`);
    expect((await screen.findByText("Took")).nextElementSibling).toHaveTextContent("not started");
    expect(screen.queryByText("Running for")).toBeNull();
  });

  it("shows a failure once when the run also stored it as an event", async () => {
    const failed: Run = { ...folderRun, status: "failed", summary: null, error: "engine: boom" };
    thread(
      [failed],
      [
        ev({ kind: "assistant_text", text: "Trying." }),
        ev({ kind: "failed", error: "engine: boom" }),
      ],
    );
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("Trying.")).toBeVisible();
    expect(screen.getAllByText("engine: boom")).toHaveLength(1);
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("keeps an earlier run's failure in its place, and a stored failure that says something else", async () => {
    const earlier: Run = {
      ...folderRun,
      id: "run-a",
      status: "failed",
      summary: null,
      error: "rate limited",
    };
    const latest: Run = {
      ...folderRun,
      id: "run-b",
      parent_run_id: "run-a",
      trigger: "resume",
      status: "failed",
      summary: null,
      error: "rate limited",
    };
    // Both runs failed the same way; the last one stored a second failure too.
    thread(
      [earlier, latest],
      [
        ev({ kind: "failed", error: "rate limited" }, "run-a"),
        ev({ kind: "failed", error: "interrupted" }, "run-b"),
        ev({ kind: "failed", error: "rate limited" }, "run-b"),
      ],
    );
    renderApp("/runs/run-b");
    const sections = await screen.findAllByRole("region", {
      name: /Scheduled run|Continued after approval/,
    });
    expect(await within(sections[0]).findByRole("alert")).toHaveTextContent("rate limited");
    expect(await within(sections[1]).findByRole("alert")).toHaveTextContent("interrupted");
    // Above the transcript and in the earlier run, not in the last run again.
    expect(screen.getAllByText("rate limited")).toHaveLength(2);
    expect(screen.getAllByRole("alert")).toHaveLength(3);
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

  it("offers no discard, review or folder for a worktree that is already gone", async () => {
    thread([run]);
    backend.on("worktree_diff", () => gone);
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    expect(
      await screen.findByText(
        "This run's worktree was removed: it had no changes, or it was discarded.",
      ),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open folder" })).toBeNull();
  });

  it("offers the folder, but no review, while the changes are still loading", async () => {
    thread([run]);
    const diff = later<WorktreeDiff>();
    backend.on("worktree_diff", () => diff.answer);
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("button", { name: "Open folder" })).toBeVisible();
    await waitFor(() => expect(backend.callsTo("worktree_diff")).toHaveLength(1));
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();

    act(() => diff.give(sample.diff));
    expect(await screen.findByRole("button", { name: "Mark reviewed" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Discard worktree" })).toBeVisible();
  });

  it("shows why the changes could not be loaded and still lets the run be marked as reviewed", async () => {
    thread([run]);
    backend.on("worktree_diff", () => {
      throw { kind: "internal", message: "git diff failed" };
    });
    backend.on("mark_reviewed", () => {
      const reviewed = { ...run, reviewed_at: run.ended_at };
      backend.on("run_thread", () => [reviewed]);
      return reviewed;
    });
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("git diff failed");
    // Whether there is a worktree to discard is not known.
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Mark reviewed" }));
    expect(await screen.findByText("Marked as reviewed")).toBeVisible();
    expect(backend.callsTo("mark_reviewed")).toEqual([{ runId: run.id }]);
    // A reviewed run is not offered again, whatever the diff says.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull());
  });

  it("offers no review for a run still going whose changes could not be loaded", async () => {
    const live: Run = { ...run, status: "running", ended_at: null, summary: null };
    thread([live]);
    backend.on("worktree_diff", () => {
      throw { kind: "internal", message: "git diff failed" };
    });
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("git diff failed");
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
    backend.on("discard_worktree", () => {
      const reviewed = { ...run, reviewed_at: run.ended_at };
      backend.on("run_thread", () => [reviewed]);
      backend.on("worktree_diff", () => gone);
      return reviewed;
    });
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Discard this worktree?" });
    expect(dialog).toHaveTextContent(run.branch!);
    expect(backend.callsTo("discard_worktree")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText("Worktree discarded")).toBeVisible();
    expect(backend.callsTo("discard_worktree")).toEqual([{ runId: run.id }]);
    // The folder went with the worktree: nothing is left to open or to discard.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Open folder" })).toBeNull());
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
  });

  it("reloads the changes when a discard is refused", async () => {
    // The worktree was removed elsewhere: the core refuses, and the diff it answers next says so.
    thread([run]);
    backend.on("discard_worktree", () => {
      backend.on("worktree_diff", () => gone);
      throw { kind: "not_found", message: "the worktree is gone" };
    });
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    await user.click(await screen.findByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText("the worktree is gone")).toBeVisible();
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull(),
    );
  });

  it("reloads the run when a review is refused", async () => {
    // A follow-up run started and its event was missed: the core refuses, and the thread it
    // answers next holds the new run.
    const failed: Run = { ...folderRun, status: "failed", summary: null, error: "timeout" };
    const next: Run = {
      ...folderRun,
      id: "run-b",
      parent_run_id: run.id,
      trigger: "resume",
      status: "running",
      ended_at: null,
      summary: null,
    };
    thread([failed]);
    backend.on("mark_reviewed", () => {
      backend.on("run_thread", () => [failed, next]);
      throw { kind: "conflict", message: "the thread is still running" };
    });
    const { user, listening } = renderApp(`/runs/${run.id}`);
    const button = await screen.findByRole("button", { name: "Mark reviewed" });
    await listening();
    const loads = backend.callsTo("inbox").length;
    await user.click(button);
    expect(await screen.findByText("the thread is still running")).toBeVisible();
    expect(await screen.findByRole("button", { name: "Cancel run" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Mark reviewed" })).toBeNull();
    // The Inbox is asked again too, as after a refusal in the Inbox itself.
    expect(backend.callsTo("inbox").length).toBeGreaterThan(loads);
  });

  it("marks the run as reviewed once when the button is double-clicked", async () => {
    // The thread the core answers stays unreviewed here, so the button stays where it is.
    const failed: Run = { ...folderRun, status: "failed", summary: null, error: "timeout" };
    thread([failed]);
    backend.on("mark_reviewed", () => ({ ...failed, reviewed_at: run.ended_at }));
    renderApp(`/runs/${run.id}`);
    const button = await screen.findByRole("button", { name: "Mark reviewed" });
    fireEvent.click(button, { detail: 1 });
    expect(await screen.findByText("Marked as reviewed")).toBeVisible();
    await waitFor(() => expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Mark reviewed" }), { detail: 2 });
    await settle();
    expect(backend.callsTo("mark_reviewed")).toEqual([{ runId: run.id }]);
  });

  it("discards the worktree once when the confirmation is double-clicked", async () => {
    // The thread the core answers keeps its worktree here; only the calls are counted.
    thread([run]);
    backend.on("discard_worktree", () => ({ ...run, reviewed_at: run.ended_at }));
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    const confirm = await screen.findByRole("button", { name: "Discard the worktree" });
    // In the app the dialog stays on the screen while it closes, so the second click of a
    // double-click lands on the same button. Here the two clicks arrive before the dialog is
    // drawn again, which is that moment.
    act(() => {
      fireEvent.click(confirm, { detail: 1 });
      fireEvent.click(confirm, { detail: 2 });
    });
    await waitFor(() => expect(backend.callsTo("discard_worktree")).not.toEqual([]));
    await settle();
    expect(backend.callsTo("discard_worktree")).toEqual([{ runId: run.id }]);
  });

  it("holds both review actions while one is on its way", async () => {
    thread([run]);
    const answer = later<Run>();
    backend.on("mark_reviewed", () => answer.answer);
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Mark reviewed" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Discard worktree" })).toBeDisabled(),
    );
    expect(screen.getByRole("button", { name: "Mark reviewed" })).toBeDisabled();

    act(() => answer.give({ ...run, reviewed_at: run.ended_at }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Discard worktree" })).toBeEnabled(),
    );
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
        { id: sample.approval.id, approved: false, note: "not on main", allowRule: null },
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

describe("the calls Dots performed in a thread", () => {
  /// An approval that was decided late: `state` is where the call Dots performs stands, and
  /// null stands for an approval the model was asked to repeat.
  const decided = (id: string, state: ApplyState | null): Approval => ({
    ...sample.approval,
    id,
    input: { command: `step ${id}` },
    status: "approved",
    parked: true,
    resolved: true,
    self_apply: state !== null,
    apply_state: state,
  });
  const labels: [ApplyState, string][] = [
    ["applied", "Performed by Dots"],
    ["failed", "Performed; the tool reported an error"],
    ["unknown", "Outcome unknown: Dots stopped while this ran"],
    ["skipped", "Not performed"],
    ["queued", "Waiting to be performed"],
    ["applying", "Being performed"],
  ];

  it("says what became of each one, and lists no other approval", async () => {
    thread(
      [folderRun],
      [],
      [...labels.map(([state]) => decided(state, state)), decided("repeated", null)],
    );
    renderApp(`/runs/${run.id}`);
    const rows = within(await screen.findByRole("list", { name: "Approved calls" })).getAllByRole(
      "listitem",
    );
    expect(rows).toHaveLength(labels.length);
    labels.forEach(([state, label], i) => {
      expect(within(rows[i]).getByText("Bash")).toBeVisible();
      expect(within(rows[i]).getByText(`step ${state}`)).toBeVisible();
      expect(within(rows[i]).getByText(label)).toBeVisible();
    });
    expect(screen.queryByText("step repeated")).toBeNull();
    // They are decided: none of them is a card to answer.
    expect(screen.queryByRole("article", { name: "Approval for Bash" })).toBeNull();
  });

  it("names a memory call Dots performed by its memory tool", async () => {
    thread([folderRun], [], [{ ...decided("m", "applied"), tool: "Memory", input: { query: "pdf" } }]);
    renderApp(`/runs/${run.id}`);
    const rows = within(await screen.findByRole("list", { name: "Approved calls" })).getAllByRole(
      "listitem",
    );
    expect(within(rows[0]!).getByText("Memory › recall")).toBeVisible();
  });

  it("shows no such list for a thread without one", async () => {
    thread([folderRun], [], [decided("repeated", null)]);
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByRole("heading", { name: `${sample.dot.name} run` })).toBeVisible();
    await waitFor(() => expect(backend.callsTo("approvals_for_run").length).toBeGreaterThan(0));
    await settle();
    expect(screen.queryByRole("list", { name: "Approved calls" })).toBeNull();
  });

  it("follows a call as the core announces what became of it", async () => {
    thread([folderRun], [], [decided("a", "queued")]);
    renderApp(`/runs/${run.id}`);
    expect(await screen.findByText("Waiting to be performed")).toBeVisible();
    for (const [state, label] of labels.slice(0, 1)) {
      const now = decided("a", state);
      backend.on("approvals_for_run", () => [now]);
      act(() => backend.emit("runtime", { type: "approval_updated", approval: now }));
      expect(await screen.findByText(label)).toBeVisible();
    }
    expect(screen.queryByText("Waiting to be performed")).toBeNull();
  });

  it("names the dot in the rules a pending call offers", async () => {
    const waiting: Run = { ...folderRun, status: "awaiting_approval", ended_at: null };
    thread([waiting], [], [sample.approval]);
    backend.on("approval_rule_candidates", () => sample.rules);
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Approve and always allow" }));
    expect(
      screen.getByRole("group", { name: `Always allow for ${sample.dot.name}` }),
    ).toBeVisible();
  });
});

describe("the line under a scheduled run's trigger", () => {
  const slot = "2026-10-09T02:00:00+01:00";
  const scheduled: Run = { ...folderRun, payload: { scheduled_for: slot } };
  const section = () => screen.findByRole("region", { name: /^Scheduled run,/ });

  it("says which slot the run was for", async () => {
    thread([scheduled]);
    renderApp(`/runs/${scheduled.id}`);
    expect(
      within(await section()).getByText(`Scheduled for ${localDateTime(slot)}`),
    ).toBeVisible();
  });

  it("says that a missed slot was caught up", async () => {
    const caughtUp: Run = { ...scheduled, payload: { scheduled_for: slot, caught_up: true } };
    thread([caughtUp]);
    renderApp(`/runs/${caughtUp.id}`);
    expect(
      within(await section()).getByText(
        `Scheduled for ${localDateTime(slot)}, caught up after the PC was off`,
      ),
    ).toBeVisible();
  });

  it("says which attempt a retry is, and links the run that failed", async () => {
    const retry: Run = {
      ...scheduled,
      id: "run-retry",
      retry_of: "run-failed",
      payload: { scheduled_for: slot, attempt: 2 },
    };
    thread([retry]);
    renderApp(`/runs/${retry.id}`);
    const shown = within(await section());
    expect(shown.getByText("Retry 2 of 6 of")).toBeVisible();
    expect(shown.getByRole("link", { name: "the failed run" })).toHaveAttribute(
      "href",
      "/runs/run-failed",
    );
    expect(shown.queryByText(/^Scheduled for/)).toBeNull();
  });

  it("shows the slot and the attempt of a run that carries one but retries no run", async () => {
    const dropped: Run = {
      ...scheduled,
      id: "run-dropped",
      retry_of: null,
      payload: { scheduled_for: slot, attempt: 3 },
    };
    thread([dropped]);
    renderApp(`/runs/${dropped.id}`);
    const shown = within(await section());
    expect(shown.getByText(`Scheduled for ${localDateTime(slot)} · attempt 3 of 6`)).toBeVisible();
    expect(shown.queryByText(/Retry/)).toBeNull();
    expect(shown.queryByRole("link")).toBeNull();
  });

  it("says nothing for a run of another trigger, or a payload it cannot read", async () => {
    const manual: Run = { ...folderRun, id: "run-manual", trigger: "manual", payload: null };
    const odd: Run = { ...scheduled, id: "run-odd", payload: { scheduled_for: 7 } };
    thread([manual, odd]);
    renderApp(`/runs/${odd.id}`);
    await screen.findByRole("region", { name: /^Manual run,/ });
    expect(within(await section()).queryByText(/^Scheduled for|^Retry/)).toBeNull();
    expect(screen.queryByText(/^Scheduled for|^Retry/)).toBeNull();
  });
});

describe("the approval buttons for a Viewer", () => {
  it("are off with the core's reason", async () => {
    const waiting: Run = { ...folderRun, status: "awaiting_approval", ended_at: null };
    thread([waiting], [], [sample.approval]);
    backend.on("approval_rule_candidates", () => sample.rules);
    backend.on("get_run", () => waiting);
    backend.on("get_dot", () => ({ ...sample.dot, org_id: "acme-eng" }));
    const org = { ...sample.orgs[0], id: "acme-eng", membership: "active", role: "viewer" } as const;
    backend.on("org_list", () => [org]);
    renderApp(`/runs/${run.id}`);
    const title = "Acme Engineering: a Viewer can't answer approvals. Ask an Admin or Owner.";
    const approve = await screen.findByRole("button", { name: "Approve" });
    await waitFor(() => expect(approve).toBeDisabled());
    expect(approve).toHaveAttribute("title", title);
    const deny = screen.getByRole("button", { name: "Deny" });
    expect(deny).toBeDisabled();
    expect(deny).toHaveAttribute("title", title);
  });
});

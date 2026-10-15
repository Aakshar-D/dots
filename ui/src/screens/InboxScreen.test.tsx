import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval, Inbox, RuleCandidate, Run, WorktreeDiff } from "@/api/types";
import { keys } from "@/lib/query";
import { backend, later } from "@/test/backend";
import { installDefaults, sample } from "@/test/fixtures";
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
    inbox({ approvals: [approval], review_runs: [worktreeRun, failedRun], attention: [] });
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
      attention: [],
    });
    renderApp("/inbox");
    expect(await screen.findByRole("button", { name: "Mark reviewed" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull();
  });

  it("approves a tool call and drops it from the list", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.approval, status: "approved" };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("decide_approval")).toEqual([
      { id: approval.id, approved: true, note: null, allowRule: null },
    ]);
  });

  /// Lets what a command's answer set off reach the screen.
  const settle = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));

  it("keeps both buttons of a decided approval off until the list has reloaded", async () => {
    inbox({ approvals: [approval], review_runs: [], attention: [] });
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
      return { approvals: [], review_runs: [], attention: [] };
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
      const box = inbox({ approvals: [approval, next], review_runs: [], attention: [] });
      backend.on("decide_approval", (args) => {
        box.set({ approvals: [next], review_runs: [], attention: [] });
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
        { id: approval.id, approved: button === "Approve", note: null, allowRule: null },
      ]);

      // A click of its own decides the card that moved up.
      await user.click(moved.getByRole("button", { name: button }));
      await waitFor(() => expect(backend.callsTo("decide_approval")).toHaveLength(2));
      expect(backend.callsTo("decide_approval")[1]).toMatchObject({ id: next.id });
    },
  );

  it("marks a run as reviewed and drops it from the list", async () => {
    const box = inbox({ approvals: [], review_runs: [failedRun], attention: [] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
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
    const box = inbox({ approvals: [], review_runs: [failedRun, next], attention: [] });
    backend.on("mark_reviewed", (args) => {
      box.set({ approvals: [], review_runs: [next], attention: [] });
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
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
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
    inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
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
    const box = inbox({ approvals: [], review_runs: [failedRun], attention: [] });
    backend.on("mark_reviewed", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      throw { kind: "not_found", message: `run ${failedRun.id}` };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Mark reviewed" }));
    expect(await screen.findByText(`run ${failedRun.id}`)).toBeVisible();
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  it("drops a run whose discard was refused once the Inbox has reloaded without it", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      throw { kind: "not_found", message: `run ${worktreeRun.id}` };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    await user.click(await screen.findByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText(`run ${worktreeRun.id}`)).toBeVisible();
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  // A load that was on its way when an action changed what it reads answers with the state from
  // before. No screen shows the run by then, so nothing restarts that load: its answer has to
  // count as stale, or the run view would show it as it is.
  it("loads a run's changes again when its worktree was discarded while they were loading", async () => {
    const box = inbox({ approvals: [], review_runs: [worktreeRun], attention: [] });
    backend.on("run_thread", () => [sample.run]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    backend.on("approvals_for_run", () => []);
    const before = later<WorktreeDiff>();
    backend.on("worktree_diff", () => before.answer);
    backend.on("discard_worktree", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      backend.on("worktree_diff", () => ({ files: [], truncated: false, exists: false }));
      return { ...sample.run, reviewed_at: sample.run.ended_at };
    });
    const { user, router, listening } = renderApp("/inbox");
    await listening();
    await user.click(await screen.findByRole("link", { name: "Open" }));
    await waitFor(() => expect(backend.callsTo("worktree_diff")).toHaveLength(1));
    await user.click(screen.getByRole("link", { name: /Inbox/ }));
    await user.click(await screen.findByRole("button", { name: "Discard worktree" }));
    await user.click(await screen.findByRole("button", { name: "Discard the worktree" }));
    expect(await screen.findByText("Worktree discarded")).toBeVisible();
    before.give(sample.diff);
    await settle();

    await act(() => router.navigate(`/runs/${sample.run.id}`));
    await waitFor(() => expect(backend.callsTo("worktree_diff")).toHaveLength(2));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Discard worktree" })).toBeNull(),
    );
  });

  it("loads a run's approvals again when one was decided while they were loading", async () => {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    const waiting: Run = {
      ...sample.run,
      branch: null,
      status: "awaiting_approval",
      ended_at: null,
      summary: null,
    };
    backend.on("run_thread", () => [waiting]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    const before = later<Approval[]>();
    backend.on("approvals_for_run", () => before.answer);
    backend.on("decide_approval", () => {
      const approved: Approval = { ...sample.approval, status: "approved" };
      box.set({ approvals: [], review_runs: [], attention: [] });
      backend.on("approvals_for_run", () => [approved]);
      return approved;
    });
    const { user, router, listening } = renderApp("/inbox");
    await listening();
    await user.click(await screen.findByRole("link", { name: "Open the run" }));
    await waitFor(() => expect(backend.callsTo("approvals_for_run")).toHaveLength(1));
    await user.click(screen.getByRole("link", { name: /Inbox/ }));
    await user.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    before.give([sample.approval]);
    await settle();

    await act(() => router.navigate(`/runs/${waiting.id}`));
    await waitFor(() => expect(backend.callsTo("approvals_for_run")).toHaveLength(2));
    await waitFor(() =>
      expect(screen.queryByRole("article", { name: "Approval for Bash" })).toBeNull(),
    );
  });

  it("shows a new approval as soon as the dot asks", async () => {
    const box = inbox({ approvals: [], review_runs: [], attention: [] });
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    await waitFor(() => expect(backend.callsTo("inbox").length).toBeGreaterThan(0));
    box.set({ approvals: [approval], review_runs: [], attention: [] });
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await screen.findByRole("article", { name: "Approval for Bash" })).toBeVisible();
  });
});

describe("saving a rule with an approval", () => {
  const dotKey = keys.dot(approval.dot_id);
  /// An Inbox with the one approval, whose decision empties it, and the rules the core offers.
  function waiting(rules: RuleCandidate[] = sample.rules) {
    const box = inbox({ approvals: [approval], review_runs: [], attention: [] });
    backend.on("approval_rule_candidates", () => rules);
    backend.on("decide_approval", () => {
      box.set({ approvals: [], review_runs: [], attention: [] });
      return { ...sample.approval, status: "approved" };
    });
  }
  const opener = { name: "Approve and always allow" };
  const card = () => screen.findByRole("article", { name: "Approval for Bash" });

  it("offers the rules the core derived, the narrowest first, and sends the one picked", async () => {
    waiting();
    const { user, queryClient, listening } = renderApp("/inbox");
    // A loaded copy of the dot: the rule changes its policy, so it must be asked for again.
    // It is put there once the app listens, which is when it marks everything stale one time.
    await listening();
    queryClient.setQueryData(dotKey, sample.dot);
    const open = await within(await card()).findByRole("button", opener);
    expect(open).toHaveAttribute("aria-expanded", "false");
    await user.click(open);
    expect(open).toHaveAttribute("aria-expanded", "true");

    const group = screen.getByRole("group", { name: `Always allow for ${approval.dot_name}` });
    const radios = within(group).getAllByRole<HTMLInputElement>("radio");
    expect(radios.map((radio) => radio.value)).toEqual(sample.rules.map((rule) => rule.pattern));
    expect(radios.map((radio) => radio.checked)).toEqual([true, false, false]);
    for (const rule of sample.rules) {
      expect(within(group).getByText(rule.pattern)).toBeVisible();
      expect(within(group).getByText(rule.label)).toBeVisible();
    }
    expect(within(group).getByText("A deny rule still wins over this rule.")).toBeVisible();

    await user.click(radios[2]);
    await user.click(within(group).getByRole("button", { name: "Approve and save rule" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("decide_approval")).toEqual([
      { id: approval.id, approved: true, note: null, allowRule: sample.rules[2].pattern },
    ]);
    expect(backend.callsTo("approval_rule_candidates")[0]).toEqual({ id: approval.id });
    expect(queryClient.getQueryState(dotKey)?.isInvalidated).toBe(true);
  });

  it("saves the narrowest rule when no other is picked, with the note", async () => {
    waiting();
    const { user } = renderApp("/inbox");
    await user.type(within(await card()).getByLabelText("Note for the dot"), "fine by me");
    await user.click(await screen.findByRole("button", opener));
    await user.click(screen.getByRole("button", { name: "Approve and save rule" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        {
          id: approval.id,
          approved: true,
          note: "fine by me",
          allowRule: sample.rules[0].pattern,
        },
      ]),
    );
  });

  it("saves no rule with a plain approval or a denial, and leaves the dot alone", async () => {
    for (const button of ["Approve", "Deny"]) {
      backend.reset();
      installDefaults();
      waiting();
      const { user, queryClient, listening, unmount } = renderApp("/inbox");
      await listening();
      queryClient.setQueryData(dotKey, sample.dot);
      await within(await card()).findByRole("button", opener);
      await user.click(screen.getByRole("button", { name: button }));
      expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: approval.id, approved: button === "Approve", note: null, allowRule: null },
      ]);
      expect(queryClient.getQueryState(dotKey)?.isInvalidated).toBe(false);
      unmount();
    }
  });

  it("closes the list of rules without deciding anything", async () => {
    waiting();
    const { user } = renderApp("/inbox");
    const open = await within(await card()).findByRole("button", opener);
    await user.click(open);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("group", { name: /Always allow for/ })).toBeNull();
    expect(open).toHaveAttribute("aria-expanded", "false");
    expect(backend.callsTo("decide_approval")).toEqual([]);
    // The card is as it was: it can still be decided.
    expect(within(await card()).getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("offers no rule for a call the core has none for", async () => {
    waiting([]);
    const { listening } = renderApp("/inbox");
    const shown = await card();
    await listening();
    expect(backend.callsTo("approval_rule_candidates").length).toBeGreaterThan(0);
    expect(within(shown).queryByRole("button", opener)).toBeNull();
    expect(within(shown).getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("says that the rules could not be loaded, and the call can still be decided", async () => {
    waiting();
    backend.on("approval_rule_candidates", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    const { user } = renderApp("/inbox");
    const shown = await card();
    expect(await within(shown).findByText(/could not be loaded/)).toBeVisible();
    expect(within(shown).queryByRole("button", opener)).toBeNull();
    await user.click(within(shown).getByRole("button", { name: "Approve" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("decide_approval")).toEqual([
      { id: approval.id, approved: true, note: null, allowRule: null },
    ]);
  });
});

describe("the approvals of one run", () => {
  const second = { ...approval, id: "apr-2", tool: "Write", input: { file_path: "notes.md" } };
  const elsewhere = {
    ...approval,
    id: "apr-3",
    run_id: "run-other",
    dot_id: "dot-other",
    dot_name: "tidy-notes",
    tool: "Edit",
    input: { file_path: "b.txt" },
  };
  const mine = { name: `Approvals asked by ${approval.dot_name}` };
  const empty: Inbox = { approvals: [], review_runs: [], attention: [] };
  const answered = [approval, second].map((a) => ({ ...sample.approval, id: a.id }));

  it("stand together under their dot, and a single one has no batch buttons", async () => {
    // The core lists approvals in the order asked; a call of another run came in between.
    inbox({ approvals: [approval, elsewhere, second], review_runs: [], attention: [] });
    renderApp("/inbox");
    const group = await screen.findByRole("group", mine);
    const cards = within(group).getAllByRole("article");
    expect(cards.map((card) => card.getAttribute("aria-label"))).toEqual([
      "Approval for Bash",
      "Approval for Write",
    ]);
    expect(within(group).getByRole("button", { name: "Approve all (2)" })).toBeVisible();
    expect(within(group).getByRole("button", { name: "Deny all (2)" })).toBeVisible();
    // The run is named once for the group, not once per call.
    expect(within(group).getAllByRole("link", { name: "Open the run" })).toHaveLength(1);

    const other = screen.getByRole("group", { name: "Approvals asked by tidy-notes" });
    expect(within(other).getAllByRole("article")).toHaveLength(1);
    expect(within(other).queryByRole("button", { name: /all \(/ })).toBeNull();
    expect(within(other).getByRole("link", { name: "Open the run" })).toHaveAttribute(
      "href",
      "/runs/run-other",
    );
    // A group stands where its first call stood.
    const names = screen
      .getAllByRole("group", { name: /^Approvals asked by/ })
      .map((g) => g.getAttribute("aria-label"));
    expect(names).toEqual([mine.name, "Approvals asked by tidy-notes"]);
  });

  it.each([
    ["Approve all (2)", "Approve all", true],
    ["Deny all (2)", "Deny all", false],
  ])("are answered together by %s, after a confirmation that lists them", async (open, confirm, approved) => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_run_approvals", () => {
      box.set(empty);
      return answered.map((a) => ({ ...a, status: approved ? "approved" : "denied" }));
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: open }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(approval.dot_name);
    expect(dialog).toHaveTextContent("Bash git push origin main");
    expect(dialog).toHaveTextContent("Write notes.md");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("decide_run_approvals")).toEqual([]);

    await user.click(screen.getByRole("button", { name: open }));
    await user.click(await screen.findByRole("button", { name: confirm }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("decide_run_approvals")).toEqual([
      { runId: approval.run_id, approved, note: null },
    ]);
    // One command for the run, and none per call.
    expect(backend.callsTo("decide_approval")).toEqual([]);
  });

  it("stay, with their buttons, when the core refuses the batch", async () => {
    inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_run_approvals", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Approve all (2)" }));
    await user.click(await screen.findByRole("button", { name: "Approve all" }));
    expect(await screen.findByText("database is locked")).toBeVisible();
    const group = screen.getByRole("group", mine);
    expect(within(group).getAllByRole("article")).toHaveLength(2);
    expect(within(group).getByRole("button", { name: "Approve all (2)" })).toBeEnabled();
  });

  it("can be answered together again when the run asks for more after a batch", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    const later1 = { ...approval, id: "apr-4", tool: "Edit", input: { file_path: "c.txt" } };
    const later2 = { ...approval, id: "apr-5", tool: "Read", input: { file_path: "d.txt" } };
    backend.on("decide_run_approvals", () => {
      // The run went on and asked for two more calls before the Inbox reloaded.
      box.set({ approvals: [later1, later2], review_runs: [], attention: [] });
      return answered.map((a) => ({ ...a, status: "approved" }));
    });
    const { user } = renderApp("/inbox");
    await user.click(await screen.findByRole("button", { name: "Approve all (2)" }));
    await user.click(await screen.findByRole("button", { name: "Approve all" }));
    expect(await screen.findByRole("article", { name: "Approval for Edit" })).toBeVisible();
    const group = screen.getByRole("group", mine);
    expect(within(group).getAllByRole("article")).toHaveLength(2);
    expect(within(group).getByRole("button", { name: "Approve all (2)" })).toBeEnabled();
    expect(within(group).getByRole("button", { name: "Deny all (2)" })).toBeEnabled();
  });

  it("can still be answered one at a time", async () => {
    const box = inbox({ approvals: [approval, second], review_runs: [], attention: [] });
    backend.on("decide_approval", (args) => {
      box.set({ approvals: [second], review_runs: [], attention: [] });
      return { ...sample.approval, id: args.id, status: "approved" };
    });
    const { user } = renderApp("/inbox");
    const first = await screen.findByRole("article", { name: "Approval for Bash" });
    await user.click(within(first).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(screen.queryByRole("article", { name: "Approval for Bash" })).toBeNull(),
    );
    // One call is left: its group has nothing to batch.
    expect(screen.getByRole("article", { name: "Approval for Write" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /all \(/ })).toBeNull();
    expect(backend.callsTo("decide_run_approvals")).toEqual([]);
  });
});

describe("a parked approval", () => {
  it("says what approving does, by who will perform the call", async () => {
    const parked = { ...approval, parked: true };
    inbox({
      approvals: [
        { ...parked, id: "own", self_apply: true },
        { ...parked, id: "model", tool: "Write", input: { file_path: "a.txt" } },
        { ...approval, id: "live", tool: "Edit", input: { file_path: "b.txt" } },
      ],
      review_runs: [],
      attention: [],
    });
    renderApp("/inbox");
    const own = await screen.findByRole("article", { name: "Approval for Bash" });
    expect(
      within(own).getByText(
        "The run stopped waiting and is parked. If you approve, Dots performs exactly this call and then continues the run.",
      ),
    ).toBeVisible();
    const model = screen.getByRole("article", { name: "Approval for Write" });
    expect(
      within(model).getByText(
        "The run stopped waiting and is parked. Approving continues it in a new run.",
      ),
    ).toBeVisible();
    // A run that still waits is answered where it stands: nothing to explain.
    const live = screen.getByRole("article", { name: "Approval for Edit" });
    expect(within(live).queryByText(/parked/)).toBeNull();
  });
});

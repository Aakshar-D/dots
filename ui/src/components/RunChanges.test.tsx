import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import type { ChangesPreview, Run, RunChanges, UndoOutcome } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

import { ChangesLine } from "./RunChanges";

/// A finished chat run that worked in a folder of the person's.
const run: Run = {
  ...sample.run,
  branch: null,
  base_commit: null,
  trigger: "chat",
  payload: { message: "Tidy the notes." },
};

const changes: RunChanges = {
  run_id: run.id,
  state: "recorded",
  git: false,
  files: [
    { path: "notes.md", status: "modified", undoable: true, reason: null },
    { path: "draft.md", status: "added", undoable: true, reason: null },
    {
      path: "dump.sql",
      status: "modified",
      undoable: false,
      reason: "Larger than 20 MB: Sage did not keep a copy, so it can't be put back.",
    },
  ],
  note: null,
  undone_at: null,
};

const preview: ChangesPreview = {
  run_id: run.id,
  redo: false,
  files: [
    { path: "notes.md", status: "modified", action: "ready", reason: null },
    {
      path: "draft.md",
      status: "added",
      action: "conflict",
      reason: "The file changed after this run.",
    },
    {
      path: "dump.sql",
      status: "modified",
      action: "cannot",
      reason: "Larger than 20 MB: Sage did not keep a copy, so it can't be put back.",
    },
  ],
  diff: {
    files: [
      {
        path: "notes.md",
        status: "modified",
        patch: "diff --git a/notes.md b/notes.md\n@@ -1 +1 @@\n-old line\n+new line\n",
        binary: false,
      },
    ],
    truncated: false,
    exists: true,
  },
  later_runs: ["01J0LATER"],
};

function outcome(redo: boolean, restored: string[]): UndoOutcome {
  return {
    redo,
    restored,
    skipped: [],
    changes: { ...changes, undone_at: redo ? null : sample.run.queued_at },
  };
}

function answers() {
  backend.on("run_changes", () => changes);
  backend.on("run_changes_preview", () => preview);
  backend.on("list_run_events", () => []);
  backend.on("approvals_for_run", () => []);
}

function renderLine(r: Run) {
  const qc = createQueryClient();
  render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <ChangesLine run={r} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return userEvent.setup();
}

describe("a folder run's changes in the conversation", () => {
  it("says how many files the run changed and undoes them after a look at each", async () => {
    answers();
    backend.on("undo_run", () => outcome(false, ["notes.md"]));
    const user = renderLine(run);
    const line = await screen.findByRole("group", { name: "Changed files" });
    expect(within(line).getByRole("link", { name: "Changed 3 files" })).toHaveAttribute(
      "href",
      `/runs/${run.id}`,
    );
    await user.click(within(line).getByRole("button", { name: "Undo these changes" }));

    const dialog = await screen.findByRole("dialog", { name: "Undo these changes?" });
    const list = await within(dialog).findByRole("list", { name: "Files" });
    expect(within(list).getByText("notes.md")).toBeVisible();
    expect(within(list).getByText("Will be put back")).toBeVisible();
    expect(within(list).getByText("Changed since")).toBeVisible();
    expect(within(list).getByText("Can't be put back")).toBeVisible();
    expect(within(dialog).getByRole("alert")).toHaveTextContent(
      "Later runs in this folder changed some of the same files",
    );
    // Nothing is written before the person confirms; the conflict is left alone by default.
    expect(backend.callsTo("undo_run")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Undo 1 file" }));
    await waitFor(() =>
      expect(backend.callsTo("undo_run")).toEqual([{ runId: run.id, overwrite: false }]),
    );
  });

  it("overwrites files changed after the run only when the box is ticked", async () => {
    answers();
    backend.on("undo_run", () => outcome(false, ["notes.md", "draft.md"]));
    const user = renderLine(run);
    await user.click(await screen.findByRole("button", { name: "Undo these changes" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(
      await within(dialog).findByRole("checkbox", { name: /Also overwrite the 1 file/ }),
    );
    await user.click(within(dialog).getByRole("button", { name: "Undo 2 files" }));
    await waitFor(() =>
      expect(backend.callsTo("undo_run")).toEqual([{ runId: run.id, overwrite: true }]),
    );
  });

  it("offers Redo once the changes were undone", async () => {
    answers();
    backend.on("run_changes", () => ({ ...changes, undone_at: sample.run.queued_at }));
    backend.on("run_changes_preview", () => ({
      ...preview,
      redo: true,
      later_runs: [],
      files: preview.files.map((f) => (f.action === "conflict" ? { ...f, action: "ready" } : f)),
    }));
    backend.on("redo_run", () => outcome(true, ["notes.md", "draft.md"]));
    const user = renderLine(run);
    expect(await screen.findByText(/· undone/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Redo these changes" }));
    const dialog = await screen.findByRole("dialog", { name: "Redo these changes?" });
    await user.click(await within(dialog).findByRole("button", { name: "Redo 2 files" }));
    await waitFor(() =>
      expect(backend.callsTo("redo_run")).toEqual([{ runId: run.id, overwrite: false }]),
    );
  });

  it("shows nothing for a run with no checkpoint, one that changed nothing, or a worktree run", async () => {
    backend.on("run_changes", () => null);
    renderLine(run);
    await waitFor(() => expect(backend.callsTo("run_changes")).toHaveLength(1));
    expect(screen.queryByRole("group", { name: "Changed files" })).toBeNull();

    backend.reset();
    backend.on("run_changes", () => ({ ...changes, files: [] }));
    renderLine({ ...run, id: "01J0OTHER" });
    await waitFor(() => expect(backend.callsTo("run_changes")).toHaveLength(1));
    expect(screen.queryByRole("group", { name: "Changed files" })).toBeNull();

    backend.reset();
    renderLine(sample.run);
    expect(backend.callsTo("run_changes")).toEqual([]);
  });
});

describe("a folder run's page", () => {
  function thread(runs: Run[]) {
    answers();
    backend.on("run_thread", () => runs);
    backend.on("get_dot", () => sample.dot);
  }

  it("has a Changes tab with each file's diff and what can't be undone", async () => {
    thread([run]);
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    const panel = await screen.findByRole("region", { name: `Changes of run ${run.id}` });
    expect(within(panel).getByText("Changed 3 files")).toBeVisible();
    expect(await within(panel).findByText("+new line")).toBeVisible();
    const cannot = within(panel).getByRole("list", { name: "Can't be undone" });
    expect(within(cannot).getByText("dump.sql")).toBeVisible();
    expect(within(panel).getByRole("button", { name: "Undo these changes" })).toBeVisible();
  });

  it("says why a run has no change list", async () => {
    thread([run]);
    backend.on("run_changes", () => ({
      ...changes,
      state: "unavailable",
      files: [],
      note: "This folder has more than 20000 files to back up, so Sage did not back it up and this run can't be undone.",
    }));
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("tab", { name: "Changes" }));
    expect(await screen.findByText(/more than 20000 files/)).toBeVisible();
  });

  it("runs the same message again as a new conversation with Run again", async () => {
    thread([run]);
    const again: Run = { ...run, id: "01J0AGAIN", status: "queued" };
    backend.on("chat", () => again);
    const { user, router } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Run again" }));
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: run.dot_id, message: "Tidy the notes.", parentRunId: null },
      ]),
    );
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/01J0AGAIN"));
  });

  it("runs a manual run's sol again with Run now", async () => {
    const manual: Run = { ...run, trigger: "manual", payload: null };
    thread([manual]);
    backend.on("run_now", () => ({ ...manual, id: "01J0AGAIN", status: "queued" }));
    const { user } = renderApp(`/runs/${run.id}`);
    await user.click(await screen.findByRole("button", { name: "Run again" }));
    await waitFor(() => expect(backend.callsTo("run_now")).toEqual([{ dotId: run.dot_id }]));
  });
});

import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Dot, DotOverview, Run, Status, UpdateNote } from "@/api/types";
import { localTime } from "@/lib/format";
import { keys } from "@/lib/query";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const restartOffered: Status = { ...quietStatus, restart_required: true };
const idle: DotOverview = { ...sample.overview, active_run: null };
const triage: Dot = { ...sample.dot, id: "dot-triage", name: "triage" };
const active = (status: Run["status"]): Run => ({
  ...sample.run,
  id: "run-active",
  dot_id: triage.id,
  status,
  ended_at: null,
});
/// The banner's restart button, once it knows whether it has to ask first.
async function restartButton() {
  const button = () => screen.getByRole("button", { name: "Restart Dots" });
  await waitFor(() => expect(button()).toBeEnabled());
  return button();
}

describe("the app frame", () => {
  it("shows the navigation and how much waits in the inbox", async () => {
    backend.on("inbox", () => sample.inbox);
    renderApp("/dots");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    for (const name of ["Home", "Library", "Settings"]) {
      expect(within(nav).getByRole("link", { name })).toHaveAttribute("title", name);
    }
    // One pending approval, one run to review, one schedule that needs attention, one MCP
    // server whose tools changed, one paused watcher, the runs held for Claude usage, one
    // proposed instruction and one app sent for review.
    expect(await within(nav).findByRole("link", { name: "Inbox, 8 waiting" })).toBeVisible();
  });

  it("counts approvals of the dots of an account that is not the current one", async () => {
    backend.on("inbox", () => sample.inbox);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    backend.on("dot_overview", () => [{ ...idle, dot: { ...sample.dot, org_id: null } }]);
    renderApp("/dots");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    expect(await within(nav).findByRole("link", { name: "Inbox, 8 waiting" })).toBeVisible();
  });

  it("shows why Dots could not start instead of the app", async () => {
    backend.on("startup_error", () => sample.startupError);
    renderApp("/dots");
    expect(await screen.findByRole("heading", { name: "Dots could not start" })).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(sample.startupError.message);
    expect(screen.getByText(sample.startupError.log_dir)).toBeVisible();
    // The newest backup an update made, and where the previous installer is.
    expect(screen.getByText(sample.startupError.backup)).toBeVisible();
    expect(screen.getByText("https://github.com/Aakshar-D/dots/releases")).toBeVisible();
    expect(screen.queryByRole("navigation")).toBeNull();
  });

  it("warns about a taken port, a missing or untested CLI, a restart and a pause", async () => {
    backend.on("status", () => ({
      ...quietStatus,
      server_error: "could not listen on 127.0.0.1:47321: address in use",
      restart_required: true,
      schedules_paused: true,
      claude: { program: null, version: "2.1.300", tested_version: "2.1.295", mismatch: true },
    }));
    renderApp("/dots");
    expect(await screen.findByText(/The local server is not running: could not listen/)).toBeVisible();
    expect(screen.getByText(/The claude CLI was not found/)).toBeVisible();
    expect(screen.getByText(/Claude CLI 2\.1\.300 has not been tested.*2\.1\.295/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Restart Dots" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Resume schedules" })).toBeVisible();
    expect(screen.getAllByRole("link", { name: "Open Settings" })).toHaveLength(2);
  });

  it("shows no banner when nothing needs attention", async () => {
    const { listening } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await listening();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("marks the CLI as tested from its banner", async () => {
    const untested = {
      ...quietStatus,
      claude: { ...quietStatus.claude, version: "2.1.300", mismatch: true },
    };
    backend.on("status", () => untested);
    backend.on("mark_claude_tested", () => quietStatus);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("button", { name: "Mark as tested" }));
    await waitFor(() => expect(screen.queryByText(/has not been tested/)).toBeNull());
    expect(backend.callsTo("mark_claude_tested")).toHaveLength(1);
  });

  it("keeps the answer of 'Mark as tested' when an older status was on its way", async () => {
    const untested = {
      ...quietStatus,
      claude: { ...quietStatus.claude, version: "2.1.300", mismatch: true },
    };
    let now: Status = untested;
    backend.on("status", () => now);
    backend.on("mark_claude_tested", () => (now = quietStatus));
    const { user, listening, queryClient } = renderApp("/dots");
    const button = await screen.findByRole("button", { name: "Mark as tested" });
    await listening();

    // The status is asked for again, and the core reads it for that answer before the click.
    const read = later<Status>();
    const asked = backend.callsTo("status").length;
    backend.on("status", () =>
      backend.callsTo("status").length === asked + 1 ? read.answer : now,
    );
    act(() => backend.emit("runtime-lagged", null));
    await waitFor(() => expect(backend.callsTo("status")).toHaveLength(asked + 1));
    await user.click(button);
    await waitFor(() => expect(screen.queryByText(/has not been tested/)).toBeNull());
    await act(async () => read.give(untested));
    await waitFor(() => expect(queryClient.isFetching()).toBe(0));

    expect(screen.queryByText(/has not been tested/)).toBeNull();
    expect(backend.callsTo("status")).toHaveLength(asked + 2);
  });

  it("resumes schedules and restarts from their banners", async () => {
    backend.on("status", () => ({ ...quietStatus, schedules_paused: true, restart_required: true }));
    backend.on("set_schedules_paused", () => ({ ...quietStatus, restart_required: true }));
    backend.on("restart_app", () => null);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("button", { name: "Resume schedules" }));
    await waitFor(() => expect(screen.queryByText(/Schedules are paused/)).toBeNull());
    expect(backend.callsTo("set_schedules_paused")).toEqual([{ paused: false }]);
    // No run is active: the restart needs no second click.
    await user.click(await restartButton());
    await waitFor(() => expect(backend.callsTo("restart_app")).toHaveLength(1));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it.each(["queued", "running", "awaiting_approval"] as const)(
    "asks before a restart while a run is %s",
    async (status) => {
      backend.on("status", () => restartOffered);
      backend.on("dot_overview", () => [idle, { ...idle, dot: triage, active_run: active(status) }]);
      backend.on("restart_app", () => null);
      const { user } = renderApp("/settings");
      await user.click(await restartButton());
      const dialog = within(await screen.findByRole("alertdialog", { name: "Restart Dots now?" }));
      // The dot whose run has not ended is named, and what the restart does to a working run.
      expect(dialog.getByText(/triage has a run that has not ended/)).toBeVisible();
      expect(dialog.getByText(/ends as failed/)).toBeVisible();
      await user.click(dialog.getByRole("button", { name: "Cancel" }));
      expect(backend.callsTo("restart_app")).toEqual([]);

      await user.click(await restartButton());
      await user.click(await screen.findByRole("button", { name: "Restart now" }));
      await waitFor(() => expect(backend.callsTo("restart_app")).toHaveLength(1));
    },
  );

  it("asks before a restart once a run has started, and no more when it has ended", async () => {
    let overview = [idle];
    backend.on("status", () => restartOffered);
    backend.on("dot_overview", () => overview);
    backend.on("restart_app", () => null);
    // Settings does not show the dots: only the banner asks for them.
    const { user, listening, queryClient } = renderApp("/settings");
    await restartButton();
    await listening();
    /// Tells the app that the run changed and waits for the list of dots it then asks for.
    async function changed(run: Run) {
      const asked = backend.callsTo("dot_overview").length;
      act(() => backend.emit("runtime", { type: "run_updated", run }));
      await waitFor(() => {
        expect(backend.callsTo("dot_overview")).toHaveLength(asked + 1);
        expect(queryClient.isFetching()).toBe(0);
      });
    }

    const run = active("running");
    overview = [{ ...idle, active_run: run }];
    await changed(run);
    await user.click(await restartButton());
    const dialog = within(await screen.findByRole("alertdialog", { name: "Restart Dots now?" }));
    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("restart_app")).toEqual([]);

    overview = [idle];
    await changed({ ...run, status: "succeeded" });
    await user.click(await restartButton());
    await waitFor(() => expect(backend.callsTo("restart_app")).toHaveLength(1));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("asks before a restart when it cannot tell whether a run is active", async () => {
    backend.on("status", () => restartOffered);
    backend.on("dot_overview", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    backend.on("restart_app", () => null);
    const { user } = renderApp("/settings");
    await user.click(await restartButton());
    const dialog = within(await screen.findByRole("alertdialog", { name: "Restart Dots now?" }));
    expect(dialog.getByText(/could not check whether a run is active/)).toBeVisible();
    expect(backend.callsTo("restart_app")).toEqual([]);
    await user.click(dialog.getByRole("button", { name: "Restart now" }));
    await waitFor(() => expect(backend.callsTo("restart_app")).toHaveLength(1));
  });

  it("does not ask for the dots' runs while no restart is on offer", async () => {
    const { listening } = renderApp("/settings");
    await screen.findByRole("heading", { name: "Settings" });
    await listening();
    expect(backend.callsTo("dot_overview")).toEqual([]);
  });

  it("loads the dots column again when schedules were resumed during its first load", async () => {
    let paused = true;
    backend.on("status", () => ({ ...quietStatus, schedules_paused: paused }));
    backend.on("set_schedules_paused", () => {
      paused = false;
      return quietStatus;
    });
    const { user, listening } = renderApp("/settings");
    await screen.findByRole("heading", { name: "Settings" });
    await listening();

    // While paused the core holds the time the dot would have run; a resume moves it on.
    const next = "2026-10-20T03:30:00.000Z";
    const read = later<DotOverview[]>();
    const moved: DotOverview = {
      ...idle,
      schedule_state: { ...idle.schedule_state!, next_fire: next },
    };
    backend.on("dot_overview", () =>
      backend.callsTo("dot_overview").length === 1 ? read.answer : [moved],
    );
    await user.click(screen.getByRole("link", { name: "Home" }));
    await waitFor(() => expect(backend.callsTo("dot_overview")).toHaveLength(1));
    await user.click(screen.getByRole("button", { name: "Resume schedules" }));
    await waitFor(() => expect(screen.queryByText(/Schedules are paused/)).toBeNull());
    // The list was read before the resume.
    await act(async () => read.give([idle]));

    expect(await screen.findByText(`next ${localTime(next)}`)).toBeVisible();
    expect(backend.callsTo("dot_overview")).toHaveLength(2);
  });

  it("follows the tray to a route", async () => {
    renderApp("/dots");
    await screen.findByRole("heading", { name: "Dots" });
    act(() => backend.emit("navigate", "/inbox"));
    expect(await screen.findByRole("heading", { name: "Inbox" })).toBeVisible();
  });

  it("takes a status that changed outside the window", async () => {
    const { listening } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await listening();
    const asked = backend.callsTo("status").length;
    act(() => backend.emit("status", { ...quietStatus, schedules_paused: true }));
    expect(await screen.findByText(/Schedules are paused/)).toBeVisible();
    // No question was on its way, so the status that arrived is the newest: nothing is asked.
    expect(backend.callsTo("status")).toHaveLength(asked);
  });

  it("keeps the status the tray sent while an older answer was on its way", async () => {
    const paused = { ...quietStatus, schedules_paused: true };
    const attach = backend.holdListeners();
    // The first answer comes at once, the second when the test gives it, later ones say what
    // the core holds by then: paused.
    const read = later<Status>();
    backend.on("status", () => {
      const asked = backend.callsTo("status").length;
      return asked === 1 ? quietStatus : asked === 2 ? read.answer : paused;
    });
    const { queryClient } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await waitFor(() => expect(queryClient.isFetching()).toBe(0));

    // Once the app listens it asks for everything again. The core reads the status for that
    // answer, then the tray pauses the schedules and says so before the answer is here.
    await act(async () => attach());
    await waitFor(() => expect(backend.callsTo("status")).toHaveLength(2));
    act(() => backend.emit("status", paused));
    expect(await screen.findByText(/Schedules are paused/)).toBeVisible();
    await act(async () => read.give(quietStatus));
    await waitFor(() => expect(queryClient.isFetching()).toBe(0));

    expect(screen.getByText(/Schedules are paused/)).toBeVisible();
    expect(queryClient.getQueryData(keys.status)).toEqual(paused);
    // The answer on its way was dropped for one read after the change.
    expect(backend.callsTo("status")).toHaveLength(3);
  });

  it("refetches after a lagged stream and after an event it cannot read", async () => {
    const { listening } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await listening();
    const loads = backend.callsTo("inbox").length;
    act(() => backend.emit("runtime-lagged", null));
    await waitFor(() => expect(backend.callsTo("inbox")).toHaveLength(loads + 1));

    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    act(() => backend.emit("runtime", { type: "from_a_newer_core" }));
    await waitFor(() => expect(backend.callsTo("inbox")).toHaveLength(loads + 2));
    expect(logged).toHaveBeenCalledTimes(1);
    logged.mockRestore();
  });

  it("loads a screen again when the stream lagged during its first load", async () => {
    const { user, listening } = renderApp("/settings");
    await screen.findByRole("heading", { name: "Settings" });
    await listening();

    const idle: DotOverview = { ...sample.overview, active_run: null };
    const running = { ...idle, active_run: { ...sample.run, status: "running" as const } };
    const read = later<DotOverview[]>();
    backend.on("dot_overview", () =>
      backend.callsTo("dot_overview").length === 1 ? read.answer : [running],
    );
    await user.click(screen.getByRole("link", { name: "Home" }));
    await waitFor(() => expect(backend.callsTo("dot_overview")).toHaveLength(1));

    // The first load read an idle dot. Its run starts, and the event that says so is dropped.
    act(() => backend.emit("runtime-lagged", null));
    await act(async () => read.give([idle]));

    const row = await screen.findByRole("listitem", { name: idle.dot.name });
    expect(await within(row).findByRole("img", { name: "running" })).toBeVisible();
    expect(backend.callsTo("dot_overview")).toHaveLength(2);
  });

  it("asks again once it listens, for what the backend emitted before", async () => {
    const attach = backend.holdListeners();
    let paused = false;
    backend.on("status", () => ({ ...quietStatus, schedules_paused: paused }));
    const { listening } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await waitFor(() => expect(backend.callsTo("status")).toHaveLength(1));

    // The tray pauses the schedules while nothing listens yet: its event is lost.
    paused = true;
    act(() => backend.emit("status", { ...quietStatus, schedules_paused: true }));
    await act(async () => attach());

    expect(await screen.findByText(/Schedules are paused/)).toBeVisible();
    // Once: listening does not start a second round of questions.
    await listening();
    expect(backend.callsTo("status")).toHaveLength(2);
    expect(backend.callsTo("inbox")).toHaveLength(2);
  });

  it("updates the inbox count when an approval is requested", async () => {
    const { listening } = renderApp("/dots");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    await listening();
    backend.on("inbox", () => sample.inbox);
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: "Inbox, 8 waiting" })).toBeVisible();
  });
});

describe("the update strip", () => {
  const ready = (working: string[], error: string | null = null): UpdateNote => ({
    state: "ready",
    version: "0.2.0",
    working,
    error,
  });
  const waiting: UpdateNote = {
    state: "waiting",
    version: "0.2.0",
    waiting_for: ["triage"],
    purpose: "update",
  };
  const installing: UpdateNote = { state: "installing", version: "0.2.0", purpose: "update" };
  const strip = () => screen.findByText(/Dots 0\.2\.0 is ready/);

  it("shows nothing until an update is ready, then what the core announces", async () => {
    const { listening } = renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await listening();
    expect(screen.queryByText(/is ready/)).toBeNull();
    act(() => backend.emit("update", ready([], "The backup failed: disk full")));
    expect(await strip()).toBeVisible();
    expect(screen.getByText("The backup failed: disk full")).toBeVisible();
  });

  it("opens what's new and hides on Later", async () => {
    backend.on("update_note", () => ready([]));
    backend.on("update_open_notes", () => null);
    backend.on("update_later", () => ({ state: "none" }));
    const { user } = renderApp("/dots");
    await strip();
    await user.click(screen.getByRole("button", { name: "What's new" }));
    await waitFor(() => expect(backend.callsTo("update_open_notes")).toHaveLength(1));
    await user.click(screen.getByRole("button", { name: "Later" }));
    await waitFor(() => expect(screen.queryByText(/is ready/)).toBeNull());
  });

  it("restarts into the update at once when nothing is working", async () => {
    backend.on("update_note", () => ready([]));
    backend.on("update_restart", () => installing);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("button", { name: "Restart to update" }));
    expect(await screen.findByText(/Installing Dots 0\.2\.0/)).toBeVisible();
    expect(backend.callsTo("update_restart")).toHaveLength(1);
  });

  it("waits for the dots that are working, and Cancel stops waiting", async () => {
    backend.on("update_note", () => ready(["triage"]));
    backend.on("update_restart", () => waiting);
    backend.on("update_cancel_wait", () => ready(["triage"]));
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/triage is still running/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Restart to update" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Restart when it's done" }));
    const waitingFor = /Will restart into Dots 0\.2\.0 when triage finishes/;
    expect(await screen.findByText(waitingFor)).toBeVisible();
    expect(screen.getByText(/New runs wait until after the update/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText(/triage is still running/)).toBeVisible();
    expect(backend.callsTo("update_cancel_wait")).toHaveLength(1);
  });

  it("asks before it stops the runs that are working", async () => {
    backend.on("update_note", () => ready(["triage", "digest"]));
    backend.on("update_stop_and_restart", () => installing);
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/triage, digest are still running/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Stop them and restart" }));
    const asked = await screen.findByRole("alertdialog", { name: "Stop and restart now?" });
    const dialog = within(asked);
    expect(dialog.getByText(/triage, digest have runs that are working/)).toBeVisible();
    expect(dialog.getByText(/ends as failed/)).toBeVisible();
    await user.click(dialog.getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("update_stop_and_restart")).toEqual([]);

    await user.click(screen.getByRole("button", { name: "Stop them and restart" }));
    await user.click(await screen.findByRole("button", { name: "Stop and restart" }));
    expect(await screen.findByText(/Installing Dots 0\.2\.0/)).toBeVisible();
    // It names the dots the confirm showed, so the core stops no other.
    expect(backend.callsTo("update_stop_and_restart")).toEqual([
      { working: ["triage", "digest"] },
    ]);
  });

  it("shows the new names when another dot started before the stop", async () => {
    backend.on("update_note", () => ready(["triage"]));
    backend.on("update_stop_and_restart", () => ready(["digest", "triage"]));
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/triage is still running/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Stop it and restart" }));
    await user.click(await screen.findByRole("button", { name: "Stop and restart" }));
    expect(await screen.findByText(/digest, triage are still running/)).toBeVisible();
    expect(backend.callsTo("update_stop_and_restart")).toEqual([{ working: ["triage"] }]);
    expect(screen.getByRole("button", { name: "Stop them and restart" })).toBeVisible();
  });

  it("says why the last update was not installed, even with no update ready, until dismissed", async () => {
    backend.on("update_note", () => ({ state: "failed", error: "The backup failed: disk full" }));
    backend.on("update_later", () => ({ state: "none" }));
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/The update was not installed/)).toBeVisible();
    expect(screen.getByText(/The backup failed: disk full/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Restart to update" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByText(/was not installed/)).toBeNull());
    expect(backend.callsTo("update_later")).toHaveLength(1);
  });

  it("offers Later while a dot is working", async () => {
    backend.on("update_note", () => ready(["triage"]));
    backend.on("update_later", () => ({ state: "none" }));
    const { user } = renderApp("/dots");
    expect(await screen.findByText(/triage is still running/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Later" }));
    await waitFor(() => expect(screen.queryByText(/is ready/)).toBeNull());
    expect(backend.callsTo("update_later")).toHaveLength(1);
  });
});

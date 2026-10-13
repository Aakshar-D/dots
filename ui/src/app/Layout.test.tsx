import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DotOverview } from "@/api/types";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

describe("the app frame", () => {
  it("shows the navigation and how much waits in the inbox", async () => {
    backend.on("inbox", () => sample.inbox);
    renderApp("/dots");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    for (const name of ["Dots", "Chat", "Settings"]) {
      expect(within(nav).getByRole("link", { name })).toBeVisible();
    }
    // One pending approval and one run to review.
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*2$/ })).toBeVisible();
  });

  it("shows why Dots could not start instead of the app", async () => {
    backend.on("startup_error", () => sample.startupError);
    renderApp("/dots");
    expect(await screen.findByRole("heading", { name: "Dots could not start" })).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(sample.startupError.message);
    expect(screen.getByText(sample.startupError.log_dir)).toBeVisible();
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

  it("resumes schedules and restarts from their banners", async () => {
    backend.on("status", () => ({ ...quietStatus, schedules_paused: true, restart_required: true }));
    backend.on("set_schedules_paused", () => ({ ...quietStatus, restart_required: true }));
    backend.on("restart_app", () => null);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("button", { name: "Resume schedules" }));
    await waitFor(() => expect(screen.queryByText(/Schedules are paused/)).toBeNull());
    expect(backend.callsTo("set_schedules_paused")).toEqual([{ paused: false }]);
    await user.click(screen.getByRole("button", { name: "Restart Dots" }));
    await waitFor(() => expect(backend.callsTo("restart_app")).toHaveLength(1));
  });

  it("follows the tray to a route", async () => {
    renderApp("/dots");
    await screen.findByRole("heading", { name: "Dots" });
    act(() => backend.emit("navigate", "/inbox"));
    expect(await screen.findByRole("heading", { name: "Inbox" })).toBeVisible();
  });

  it("takes a status that changed outside the window", async () => {
    renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    act(() => backend.emit("status", { ...quietStatus, schedules_paused: true }));
    expect(await screen.findByText(/Schedules are paused/)).toBeVisible();
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
    await user.click(screen.getByRole("link", { name: "Dots" }));
    await waitFor(() => expect(backend.callsTo("dot_overview")).toHaveLength(1));

    // The first load read an idle dot. Its run starts, and the event that says so is dropped.
    act(() => backend.emit("runtime-lagged", null));
    await act(async () => read.give([idle]));

    const name = `Open the active run of ${idle.dot.name}`;
    expect(await screen.findByRole("link", { name })).toHaveTextContent("Running");
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
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*2$/ })).toBeVisible();
  });
});

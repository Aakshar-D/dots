import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { backend } from "@/test/backend";
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
    renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await waitFor(() => expect(backend.callsTo("status")).toHaveLength(1));
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
    renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    await waitFor(() => expect(backend.callsTo("inbox")).toHaveLength(1));
    act(() => backend.emit("runtime-lagged", null));
    await waitFor(() => expect(backend.callsTo("inbox")).toHaveLength(2));

    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    act(() => backend.emit("runtime", { type: "from_a_newer_core" }));
    await waitFor(() => expect(backend.callsTo("inbox")).toHaveLength(3));
    expect(logged).toHaveBeenCalledTimes(1);
    logged.mockRestore();
  });

  it("updates the inbox count when an approval is requested", async () => {
    renderApp("/dots");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    await waitFor(() => expect(backend.callsTo("inbox")).toHaveLength(1));
    backend.on("inbox", () => sample.inbox);
    act(() => backend.emit("runtime", { type: "approval_requested", approval: sample.approval }));
    expect(await within(nav).findByRole("link", { name: /^Inbox\s*2$/ })).toBeVisible();
  });
});

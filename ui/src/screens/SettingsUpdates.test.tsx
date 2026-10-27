import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Settings, UpdateStatus } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const CHECKED = "2026-10-27T10:42:00.000Z";

const status = (over: Partial<UpdateStatus> = {}): UpdateStatus => ({
  version: "0.3.0",
  checks_on: true,
  checked_at: CHECKED,
  ready: null,
  last_error: null,
  go_back: null,
  auto_install: false,
  ...over,
});

/// The Settings screen with the core keeping what it saves.
function settings(over: Partial<UpdateStatus> = {}) {
  let current: Settings = sample.settings;
  backend.on("get_settings", () => current);
  backend.on("update_settings", (args) => {
    current = args.settings as Settings;
    return { settings: current, restart_required: false };
  });
  backend.on("update_status", () => status(over));
  return renderApp("/settings");
}

const section = () => screen.findByRole("group", { name: "Updates" });

describe("the Updates settings", () => {
  it("show the version and when it last checked", async () => {
    settings();
    expect(
      await within(await section()).findByText(
        `Dots 0.3.0 · up to date · checked ${localTime(CHECKED)}`,
      ),
    ).toBeVisible();
  });

  it("say updates are off in a copy without checks, and show nothing else", async () => {
    settings({ checks_on: false, checked_at: null });
    const updates = await section();
    expect(await within(updates).findByText("Updates are off in this copy")).toBeVisible();
    expect(within(updates).queryByRole("button", { name: "Check for updates" })).toBeNull();
    expect(within(updates).queryByLabelText("Install updates by itself when idle")).toBeNull();
  });

  it("check now and say what it found", async () => {
    let answer: (v: unknown) => void = () => {};
    backend.on(
      "update_check_now",
      () => new Promise((resolve) => (answer = resolve)),
    );
    const { user } = settings();
    const updates = await section();
    const check = await within(updates).findByRole("button", { name: "Check for updates" });
    await user.click(check);
    await waitFor(() => expect(check).toBeDisabled());
    answer({ state: "ready", version: "0.3.1" });
    expect(await within(updates).findByText("0.3.1 is ready")).toBeVisible();
    backend.on("update_check_now", () => ({ state: "failed", error: "GitHub could not be reached" }));
    await user.click(check);
    expect(await within(updates).findByText("GitHub could not be reached")).toBeVisible();
  });

  it("save the idle switch with the other settings", async () => {
    const { user } = settings();
    const updates = await section();
    await user.click(await within(updates).findByLabelText("Install updates by itself when idle"));
    expect(
      within(updates).getByText("Off: a strip appears and you choose when to restart."),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_settings")).toHaveLength(1));
    const saved = backend.callsTo("update_settings")[0]!.settings as Settings;
    expect(saved.auto_install_updates).toBe(true);
  });

  it("repair asks first with the exact words", async () => {
    backend.on("update_repair", () => ({ state: "installing", version: "0.3.0", purpose: "repair" }));
    const { user } = settings();
    await user.click(await within(await section()).findByRole("button", { name: "Repair Dots…" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(
      "Dots will download version 0.3.0 again, back up your data and reinstall it. Runs that are working will be stopped.",
    );
    await user.click(within(dialog).getByRole("button", { name: "Repair" }));
    await waitFor(() => expect(backend.callsTo("update_repair")).toHaveLength(1));
  });

  it("repair says when there is no installer", async () => {
    backend.on("update_repair", () => {
      throw {
        kind: "conflict",
        message: "This version has no published installer to repair from.",
      };
    });
    const { user } = settings();
    await user.click(await within(await section()).findByRole("button", { name: "Repair Dots…" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Repair" }),
    );
    expect(
      await screen.findByText("This version has no published installer to repair from."),
    ).toBeVisible();
  });

  it("go back is offered only when status names a version, and asks first", async () => {
    const first = settings();
    await within(await section()).findByRole("button", { name: "Check for updates" });
    expect(screen.queryByRole("button", { name: /Go back to/ })).toBeNull();
    first.unmount();
    backend.on("update_go_back", () => ({ state: "installing", version: "0.2.0", purpose: "go_back" }));
    const { user } = settings({ go_back: "0.2.0" });
    await user.click(await within(await section()).findByRole("button", { name: "Go back to 0.2.0…" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(
      "Go back to 0.2.0? Your data from before the update comes back with it; anything done since the update is lost.",
    );
    await user.click(within(dialog).getByRole("button", { name: "Go back" }));
    await waitFor(() => expect(backend.callsTo("update_go_back")).toHaveLength(1));
  });

  it("the window tells the core it rendered, once", async () => {
    const { user } = settings();
    await section();
    await user.click(await screen.findByRole("link", { name: "Home" }));
    await user.click(await screen.findByRole("link", { name: "Settings" }));
    await section();
    expect(backend.callsTo("app_rendered")).toHaveLength(1);
  });
});

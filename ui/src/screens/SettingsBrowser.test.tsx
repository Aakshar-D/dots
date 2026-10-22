import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Settings } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

describe("browser use in Settings", () => {
  it("says what was found and saves the Edge path", async () => {
    let current: Settings = sample.settings;
    backend.on("get_settings", () => current);
    backend.on("browser_status", () => ({ edge: "C:/Edge/msedge.exe", npx: null }));
    backend.on("update_settings", (args) => {
      current = args.settings as Settings;
      return { settings: current, restart_required: false };
    });
    const { user } = renderApp("/settings");
    expect(await screen.findByText("Microsoft Edge found")).toBeVisible();
    expect(screen.getByText("C:/Edge/msedge.exe")).toBeVisible();
    expect(screen.getByText("Node.js not found")).toBeVisible();
    await user.type(screen.getByLabelText("Edge path"), "  D:/Edge/msedge.exe ");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...sample.settings, edge_path: "D:/Edge/msedge.exe" } },
      ]),
    );
  });

  it("says so under the field when the Edge program was declined", async () => {
    backend.on("update_settings", () => {
      throw { kind: "cancelled", message: "the Edge program was not changed" };
    });
    const { user } = renderApp("/settings");
    expect(await screen.findByText("Microsoft Edge not found")).toBeVisible();
    expect(screen.getByText("Node.js not found")).toBeVisible();
    await user.type(screen.getByLabelText("Edge path"), "D:/Edge/msedge.exe");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText("Not saved: you declined the change of the Edge program."),
    ).toBeVisible();
    expect(
      screen.queryByText("Not saved: you declined the change of the Claude program."),
    ).toBeNull();
  });
});

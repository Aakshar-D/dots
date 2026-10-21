import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Settings } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

describe("the Settings switch for dot notes", () => {
  it("is on by default and is saved with the other settings", async () => {
    let current: Settings = sample.settings;
    backend.on("get_settings", () => current);
    backend.on("update_settings", (args) => {
      current = args.settings as Settings;
      return { settings: current, restart_required: false };
    });
    const { user } = renderApp("/settings");
    const toggle = await screen.findByLabelText("Notify on dot notes");
    expect(toggle).toBeChecked();
    expect(screen.getByText("Questions from dots always notify.")).toBeVisible();
    await user.click(toggle);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...sample.settings, notify_on_notes: false } },
      ]),
    );
  });
});

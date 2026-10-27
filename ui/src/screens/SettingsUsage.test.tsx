import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Settings } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// The Settings screen with the core keeping what it saves.
function settings() {
  let current: Settings = sample.settings;
  backend.on("get_settings", () => current);
  backend.on("update_settings", (args) => {
    current = args.settings as Settings;
    return { settings: current, restart_required: false };
  });
  return renderApp("/settings");
}

describe("the Claude usage settings", () => {
  it("hold background dots at 60% of the 5-hour window and 70% of the week by default", async () => {
    settings();
    const section = await screen.findByRole("group", { name: "Claude usage" });
    expect(section).toBeVisible();
    expect(screen.getByLabelText("Hold background dots when Claude usage is high")).toBeChecked();
    expect(screen.getByLabelText("5-hour window share (%)")).toHaveValue(60);
    expect(screen.getByLabelText("Weekly window share (%)")).toHaveValue(70);
  });

  it("refuse a share that is not 10 to 100 in steps of 5", async () => {
    const { user } = settings();
    const field = await screen.findByLabelText("5-hour window share (%)");
    await user.clear(field);
    await user.type(field, "62");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("A share is 10 to 100%, in steps of 5.")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([]);
  });

  // Self-QA: the rule showed under the fields but neither field said it was wrong.
  it("mark both share fields with the rule and announce it", async () => {
    const { user } = settings();
    const field = await screen.findByLabelText("Weekly window share (%)");
    await user.clear(field);
    await user.type(field, "7");
    await user.click(screen.getByRole("button", { name: "Save" }));
    const rule = "A share is 10 to 100%, in steps of 5.";
    expect(await screen.findByRole("alert")).toHaveTextContent(rule);
    for (const name of ["5-hour window share (%)", "Weekly window share (%)"]) {
      expect(screen.getByLabelText(name)).toHaveAttribute("aria-invalid", "true");
      expect(screen.getByLabelText(name)).toHaveAccessibleDescription(rule);
    }
  });

  it("save the switch and both shares with the other settings", async () => {
    const { user } = settings();
    await user.click(await screen.findByLabelText("Hold background dots when Claude usage is high"));
    const weekly = screen.getByLabelText("Weekly window share (%)");
    await user.clear(weekly);
    await user.type(weekly, "85");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...sample.settings, hold_enabled: false, hold_share_weekly: 85 } },
      ]),
    );
  });
});

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Settings } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// The Runs tab of Settings with the core keeping what it saves, and today's spend.
function settings() {
  let current: Settings = sample.settings;
  backend.on("get_settings", () => current);
  backend.on("update_settings", (args) => {
    current = args.settings as Settings;
    return { settings: current, restart_required: false };
  });
  return renderApp("/settings/runs");
}

describe("the spending caps in Settings", () => {
  it("show today's and this month's spend against the caps, warning from 80%", async () => {
    backend.on("spend_today", () => sample.spend);
    settings();
    const section = await screen.findByRole("group", { name: "Spending caps" });
    // $8.50 of the $10.00 daily cap is 85%.
    const today = await within(section).findByText(/^Today: \$8\.50 of \$10\.00/);
    expect(today).toHaveTextContent("estimated · 1,250,000 tokens · Over 80% of the cap");
    expect(within(section).getByText(/^This month: \$61\.25 of \$200\.00/)).toBeVisible();
    expect(within(section).getByLabelText("Daily cost cap (USD)")).toHaveValue("10");
    expect(within(section).getByLabelText("Daily token cap")).toHaveValue("");
    expect(within(section).getByLabelText("Monthly cost cap (USD)")).toHaveValue("200");
  });

  it("say when a cap is reached", async () => {
    backend.on("spend_today", () => ({ ...sample.spend, today: { cost_usd: 10, tokens: 5 } }));
    settings();
    const section = await screen.findByRole("group", { name: "Spending caps" });
    expect(await within(section).findByText(/^Today: \$10\.00 of \$10\.00/)).toHaveTextContent(
      "Cap reached",
    );
  });

  it("refuse a cap out of bounds before saving", async () => {
    const { user } = settings();
    const field = await screen.findByLabelText("Daily token cap");
    await user.type(field, "0");
    expect(await screen.findByText("A token cap is 1 to 10,000,000,000 tokens.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await user.clear(field);
    await user.type(field, "abc");
    expect(screen.getByText("A token cap is 1 to 10,000,000,000 tokens.")).toBeVisible();
    const cost = screen.getByLabelText("Daily cost cap (USD)");
    await user.clear(cost);
    await user.type(cost, "0.001");
    expect(screen.getByText("A cost cap is $0.01 to $100,000.")).toBeVisible();
  });

  it("save caps with the other settings, an emptied one as no cap", async () => {
    const { user } = settings();
    const daily = await screen.findByLabelText("Daily cost cap (USD)");
    await user.clear(daily);
    await user.type(daily, "12.5");
    await user.type(screen.getByLabelText("Daily token cap"), "500,000");
    await user.clear(screen.getByLabelText("Monthly cost cap (USD)"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        {
          settings: {
            ...sample.settings,
            spend_daily_usd: 12.5,
            spend_daily_tokens: 500000,
            spend_monthly_usd: null,
          },
        },
      ]),
    );
  });
});

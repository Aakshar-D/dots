import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { LimitsState, UsageSummary } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [holding] = sample.limits as [LimitsState];
const week: UsageSummary = sample.usage;

/// The Usage page with the core answering `summary` for every period.
async function open(summary: (period: string) => UsageSummary = () => week) {
  backend.on("limits_get", () => holding);
  backend.on("usage_summary", (args) => summary(args.period as string));
  const view = renderApp("/usage");
  expect(await screen.findByRole("heading", { name: "Usage" })).toBeVisible();
  return view;
}

const table = () => screen.findByRole("table");
const rows = async () =>
  within(await table())
    .getAllByRole("row")
    .slice(1)
    .map((r) => within(r).getAllByRole("cell").map((c) => c.textContent));

describe("the Usage page", () => {
  it("opens on the last 5 hours and switches to the week and the month", async () => {
    const { user } = await open((period) => ({ ...week, period: period as UsageSummary["period"] }));
    expect(await table()).toHaveAccessibleName("Dots' runs in the last 5 hours");
    await user.click(screen.getByRole("tab", { name: "Week" }));
    expect(await screen.findByRole("table", { name: "Dots' runs in the last 7 days" })).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "Month" }));
    expect(
      await screen.findByRole("table", { name: "Dots' runs in the last 30 days" }),
    ).toBeVisible();
    // The app asks again once it listens to the backend; each period is asked in turn.
    const asked = backend.callsTo("usage_summary").map((a) => a.period);
    expect([...new Set(asked)]).toEqual(["five_hours", "week", "month"]);
  });

  it("lists each dot in the core's order with its runs, tokens, time and cost", async () => {
    await open();
    const headers = within(await table())
      .getAllByRole("columnheader")
      .map((h) => h.textContent);
    expect(headers).toEqual(["Dot", "Runs", "Tokens", "Run time", "API-eq."]);
    expect(await rows()).toEqual([
      ["nightly-tests", "14", "2.1M", "3h 12m", "$18.40"],
      ["local-summarizer", "22", "300k", "1h 05m", "–"],
    ]);
  });

  it("opens a dot's heaviest runs, each a link to its run", async () => {
    const { user } = await open();
    const toggle = await screen.findByRole("button", { name: "nightly-tests" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    const heaviest = screen.getByRole("list", { name: "Heaviest runs of nightly-tests" });
    const links = within(heaviest).getAllByRole("link");
    expect(links).toHaveLength(1);
    expect(links[0]).toHaveAttribute("href", "/runs/01J0RUN");
    expect(links[0]).toHaveTextContent("182k tokens");
    expect(links[0]).toHaveTextContent("$1.32");
  });

  it("shows both windows with their shares, and how many runs are held", async () => {
    await open();
    const top = await screen.findByRole("region", { name: "Claude usage" });
    await waitFor(() => expect(top).toHaveTextContent("5-hour64%holds at 60%"));
    expect(top).toHaveTextContent("weekly55%holds at 70%");
    expect(within(top).getByRole("link", { name: "Holding 2 runs" })).toHaveAttribute(
      "href",
      "/inbox",
    );
  });

  it("says a deleted dot's runs are not counted, and when a period has no runs", async () => {
    await open(() => ({ ...week, rows: [] }));
    expect(await screen.findByText("No runs in the last 5 hours.")).toBeVisible();
    expect(screen.getByText("A deleted dot's runs are deleted with it and not counted.")).toBeVisible();
  });
});

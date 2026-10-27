import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { LimitReading, LimitsState } from "@/api/types";
import { resetTime } from "@/lib/usage-format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const at = (minutesFromNow: number) => new Date(Date.now() + minutesFromNow * 60_000).toISOString();
const reading = (window: string, utilization: number, readMinutesAgo = 1): LimitReading => ({
  window,
  utilization,
  resets_at: at(120),
  status: "allowed",
  read_at: at(-readMinutesAgo),
});
const [holding] = sample.limits as [LimitsState];
const quiet: LimitsState = {
  ...holding,
  five_hour: reading("five_hour", 0.42),
  weekly: reading("seven_day", 0.55),
  holding: false,
  held_run_ids: [],
  hold_until: null,
};
// The time alone today, with the weekday once it falls tomorrow.
const resets = resetTime(at(120));

/// The rail's link to the Usage page, once the limits are in.
async function meter(state: LimitsState) {
  backend.on("limits_get", () => state);
  const view = renderApp("/home");
  const nav = await screen.findByRole("navigation", { name: "Main" });
  const link = await within(nav).findByRole("link", { name: /^Claude usage/ });
  return { ...view, link };
}

const bars = (link: HTMLElement) => [...link.querySelectorAll<HTMLElement>("[data-window]")];

describe("the rail's Claude usage bars", () => {
  it("fill to the 5-hour and weekly readings and read them out", async () => {
    const { link } = await meter(quiet);
    expect(link).toHaveAccessibleName(
      `Claude usage: 5-hour 42%, resets ${resets}; weekly 55%, resets ${resets}; holding 0 runs`,
    );
    expect(bars(link).map((b) => [b.dataset.window, b.dataset.level])).toEqual([
      ["five_hour", "42"],
      ["seven_day", "55"],
    ]);
    expect(bars(link).every((b) => b.dataset.holding === undefined)).toBe(true);
    // The smoke run: an empty track vanished into the rail. Every track has a solid outline.
    for (const bar of bars(link)) {
      expect(bar.className.split(" ")).toContain("border");
      expect(bar.className).not.toContain("border-dashed");
    }
  });

  it("turn to the holding colour and say how many runs wait", async () => {
    const { link } = await meter({
      ...quiet,
      holding: true,
      held_run_ids: ["a", "b", "c"],
      hold_until: at(120),
    });
    expect(link).toHaveAccessibleName(/holding 3 runs$/);
    expect(bars(link).every((b) => b.dataset.holding === "true")).toBe(true);
  });

  it("say one held run in the singular", async () => {
    const { link } = await meter({ ...quiet, holding: true, held_run_ids: ["a"], hold_until: at(120) });
    expect(link).toHaveAccessibleName(/holding 1 run$/);
  });

  it("show a window without a reading as an empty dashed bar", async () => {
    const { link } = await meter({ ...quiet, weekly: null });
    expect(link).toHaveAccessibleName(/; weekly unknown;/);
    const weekly = bars(link)[1]!;
    expect(weekly.dataset.level).toBe("unknown");
    expect(weekly.className).toContain("border-dashed");
  });

  it("say how old a reading is once it is older than 10 minutes", async () => {
    const { link } = await meter({ ...quiet, five_hour: reading("five_hour", 0.42, 25) });
    expect(link).toHaveAttribute("title", expect.stringMatching(/; read 25 min ago$/));
  });

  it("say nothing of age for a fresh reading", async () => {
    const { link } = await meter(quiet);
    expect(link.getAttribute("title")).not.toMatch(/read/);
  });

  it("shows at most 100%", async () => {
    const { link } = await meter({ ...quiet, five_hour: reading("five_hour", 1.4) });
    expect(link).toHaveAccessibleName(/^Claude usage: 5-hour 100%/);
    expect(bars(link)[0]!.dataset.level).toBe("100");
  });

  it("open the Usage page, and Tab reaches them", async () => {
    const { link, user } = await meter(quiet);
    link.focus();
    expect(link).toHaveFocus();
    await user.click(link);
    expect(await screen.findByRole("heading", { name: "Usage" })).toBeVisible();
  });
});

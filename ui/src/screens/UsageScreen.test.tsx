import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { ClaudeProject, ClaudeUsage, LimitsState, UsageSummary } from "@/api/types";
import { localTime } from "@/lib/format";
import { CLAUDE_REFRESH_MS } from "@/screens/UsageScreen";
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
      // Open Dot's requests, after the dots.
      ["Open Dot (voice)", "38 requests", "156k", "–", "$0.21"],
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
    await open(() => ({ ...week, rows: [], open_dot: null }));
    expect(await screen.findByText("No runs in the last 5 hours.")).toBeVisible();
    expect(screen.getByText("A deleted dot's runs are deleted with it and not counted.")).toBeVisible();
  });

  it("lists Open Dot's requests in a row of their own, also when no dot ran", async () => {
    const openDot = { requests: 7, tokens_in: 30_000, tokens_out: 1_153, cost_usd: 0.04 };
    const view = await open(() => ({ ...week, open_dot: openDot }));
    expect((await rows()).at(-1)).toEqual(["Open Dot (voice)", "7 requests", "31k", "–", "$0.04"]);
    expect(await rows()).toHaveLength(3);
    view.unmount();

    await open(() => ({ ...week, rows: [], open_dot: { ...openDot, requests: 1, cost_usd: null } }));
    expect(await rows()).toEqual([["Open Dot (voice)", "1 request", "31k", "–", "–"]]);
    expect(screen.queryByText("No runs in the last 5 hours.")).toBeNull();
  });
});

describe("all Claude use on the Usage page", () => {
  const [week, noFolder] = sample.claudeUsage as [ClaudeUsage, ClaudeUsage];

  /// The page with the core answering `usage` for every period.
  async function openAll(usage: (period: string) => ClaudeUsage = () => week) {
    backend.on("claude_usage", (args) => ({
      ...usage(args.period as string),
      period: args.period as ClaudeUsage["period"],
    }));
    return open();
  }

  it("shows the person's own sessions, Dots' and all of it, then each project", async () => {
    await openAll();
    const all = await screen.findByRole("region", { name: "All Claude use" });
    const totals = await within(all).findAllByRole("definition");
    expect(totals.map((t) => t.textContent)).toEqual([
      "3900 messages · 58 sessions · 3.1M tokens · cache reads 48M",
      "310 messages · 5 sessions · 461k tokens · cache reads 3.1M",
      "4210 messages · 63 sessions · 3.6M tokens · cache reads 51.1M",
    ]);
    const table = within(all).getByRole("table", {
      name: "Claude use by project in the last 5 hours",
    });
    const cells = within(table)
      .getAllByRole("row")
      .slice(1)
      .map((r) => within(r).getAllByRole("cell").map((c) => c.textContent));
    expect(cells).toEqual([
      ["dots", "3000", "40", "2.6M", "40M"],
      ["Dots (all dots)", "310", "5", "461k", "3.1M"],
      ["site", "900", "18", "530k", "8M"],
    ]);
    // The folder's full path on hover.
    expect(within(table).getByText("dots")).toHaveAttribute("title", "C:/Users/u/code/dots");
    expect(within(all).getByText(/^Read .* from C:\/Users\/u\/\.claude\/projects$/)).toBeVisible();
  });

  it("lets the keyboard reach each table's scroll box, named for its table", async () => {
    const { user } = await openAll();
    await screen.findByRole("table", { name: "Claude use by project in the last 5 hours" });
    for (const name of ["Dots' runs in the last 5 hours", "Claude use by project in the last 5 hours"]) {
      const box = screen.getByRole("region", { name });
      expect(box).toHaveAttribute("tabindex", "0");
      expect(within(box).getByRole("table")).toBeVisible();
    }
    const box = screen.getByRole("region", { name: "Claude use by project in the last 5 hours" });
    while (document.activeElement !== box) await user.tab();
    expect(box).toHaveFocus();
  });

  it("says one message, one session and one block message in the singular", async () => {
    const one = { ...week.own, messages: 1, sessions: 1 };
    await openAll(() => ({
      ...week,
      own: one,
      dots: { ...one, messages: 0, sessions: 0 },
      block: { ...week.block!, messages: 1 },
    }));
    const all = await screen.findByRole("region", { name: "All Claude use" });
    const totals = await within(all).findAllByRole("definition");
    expect(totals[0]).toHaveTextContent(/^1 message · 1 session · /);
    expect(totals[1]).toHaveTextContent(/^0 messages · 0 sessions · /);
    const top = await screen.findByRole("region", { name: "Claude usage" });
    await waitFor(() => expect(top).toHaveTextContent("This block: 1 message ·"));
  });

  it("names two projects in folders of one name by their parents too", async () => {
    const [first, , last] = week.projects as [ClaudeProject, ClaudeProject, ClaudeProject];
    await openAll(() => ({
      ...week,
      projects: [
        { ...first, cwd: "C:/Users/u/code/app/src", name: "src" },
        { ...last, cwd: "C:/Users/u/code/site/src", name: "src" },
      ],
    }));
    const table = await screen.findByRole("table", {
      name: "Claude use by project in the last 5 hours",
    });
    expect(within(table).getByText("app/src")).toHaveAttribute("title", "C:/Users/u/code/app/src");
    expect(within(table).getByText("site/src")).toBeVisible();
  });

  it("follows the period switch", async () => {
    const { user } = await openAll();
    await user.click(await screen.findByRole("tab", { name: "Month" }));
    expect(
      await screen.findByRole("table", { name: "Claude use by project in the last 30 days" }),
    ).toBeVisible();
    expect(backend.callsTo("claude_usage").map((a) => a.period)).toContain("month");
  });

  it("reads the transcripts when the page opens, and again while a scan stops at its cap", async () => {
    let scans = 0;
    backend.on("claude_usage_scan", () => {
      scans += 1;
      return { scanned_at: "2026-10-09T12:00:00.000Z", files: 3, bytes: 100, more: scans < 2 };
    });
    await openAll();
    await waitFor(() => expect(scans).toBe(2));
    // Each scan refreshes the section.
    await waitFor(() => expect(backend.callsTo("claude_usage").length).toBeGreaterThanOrEqual(3));
  });

  it("keeps an open page fresh: it asks again every minute and refreshes the section", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      await openAll();
      await screen.findByText(/^Read .* from /);
      // Let the opening asks settle, then count from there.
      await vi.advanceTimersByTimeAsync(1000);
      const scans = backend.callsTo("claude_usage_scan").length;
      const usage = backend.callsTo("claude_usage").length;
      await vi.advanceTimersByTimeAsync(CLAUDE_REFRESH_MS);
      await waitFor(() =>
        expect(backend.callsTo("claude_usage_scan").length).toBeGreaterThan(scans),
      );
      await waitFor(() => expect(backend.callsTo("claude_usage").length).toBeGreaterThan(usage));
    } finally {
      vi.useRealTimers();
    }
  });

  it("says so when this PC has no Claude Code folder", async () => {
    await openAll(() => noFolder);
    const all = await screen.findByRole("region", { name: "All Claude use" });
    expect(
      await within(all).findByText(
        "No Claude Code transcripts on this PC (looked in C:/Users/u/.claude/projects).",
      ),
    ).toBeVisible();
    expect(within(all).queryByRole("table")).toBeNull();
  });

  it("says so when the period had no Claude Code use", async () => {
    await openAll(() => ({ ...week, own: noFolder.own, dots: noFolder.dots, projects: [] }));
    expect(await screen.findByText("No Claude Code use in the last 5 hours.")).toBeVisible();
  });

  it("counts the current 5-hour block next to the window", async () => {
    await openAll();
    const top = await screen.findByRole("region", { name: "Claude usage" });
    await waitFor(() =>
      expect(top).toHaveTextContent(
        `This block: 212 messages · 1.4M tokens since ${localTime(week.block!.started_at)}`,
      ),
    );
  });

  it("leaves the block out when the 5-hour window has no reading", async () => {
    await openAll(() => ({ ...week, block: null }));
    await screen.findByRole("region", { name: "All Claude use" });
    const top = await screen.findByRole("region", { name: "Claude usage" });
    expect(top).not.toHaveTextContent("This block");
  });
});

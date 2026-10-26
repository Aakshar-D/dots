import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { PausedWatcher } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [paused] = sample.inbox.watchers as [PausedWatcher];
const inbox = (watchers: PausedWatcher[]) => ({
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers,
});

describe("paused watchers in the Inbox", () => {
  it("lists the watchers Dots paused, with Resume and a way to their dot", async () => {
    backend.on("inbox", () => inbox([paused]));
    backend.on("watcher_resume", () => sample.watchers[1]);
    const { user } = renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Paused watchers" });
    const item = within(section).getByRole("listitem", { name: "Billing mail paused" });
    expect(item).toHaveTextContent("nightly-tests");
    expect(item).toHaveTextContent(
      "paused: Gmail refused the app password for me@gmail.com, or IMAP is off for it.",
    );
    expect(within(item).getByRole("link", { name: "Open the dot's watchers" })).toHaveAttribute(
      "href",
      `/dots/${paused.dot_id}?tab=watchers`,
    );
    await user.click(within(item).getByRole("button", { name: "Resume" }));
    await waitFor(() =>
      expect(backend.callsTo("watcher_resume")).toEqual([{ id: paused.watcher_id }]),
    );
  });

  it("says a flood is too many runs, and the Inbox link counts paused watchers", async () => {
    const flood = { ...paused, name: "Repo events", reason: "flood" as const, error: null };
    backend.on("inbox", () => inbox([flood]));
    renderApp("/inbox");
    const item = await screen.findByRole("listitem", { name: "Repo events paused" });
    expect(item).toHaveTextContent("paused: too many runs");
    expect(screen.getByRole("link", { name: "Inbox, 1 waiting" })).toBeVisible();
  });
});

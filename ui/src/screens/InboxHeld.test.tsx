import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { HeldRun, Inbox } from "@/api/types";
import { resetTime } from "@/lib/usage-format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [scheduled] = sample.inbox.held as [HeldRun];
const draining: HeldRun = {
  ...scheduled,
  run_id: "01J0RUN5",
  trigger: "watcher",
  starts_after: null,
};
const only = (held: HeldRun[]): Inbox => ({
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held,
});

describe("runs held for Claude usage in the Inbox", () => {
  it("lists each with its dot, what started it and when it starts, and Run anyway", async () => {
    backend.on("inbox", () => only([scheduled, draining]));
    backend.on("run_release_hold", () => ({ ...sample.run, id: scheduled.run_id }));
    const { user } = renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Held for Claude usage" });
    const items = within(section).getAllByRole("listitem");
    expect(items[0]).toHaveTextContent(
      `${scheduled.dot_name} · Scheduled · starts after ${resetTime(scheduled.starts_after!)}`,
    );
    expect(items[1]).toHaveTextContent(
      `${draining.dot_name} · Watcher · waiting for the first run after the reset`,
    );
    await user.click(within(items[0]!).getByRole("button", { name: "Run anyway" }));
    await waitFor(() =>
      expect(backend.callsTo("run_release_hold")).toEqual([{ runId: scheduled.run_id }]),
    );
  });

  it("count as one in the rail's Inbox link, however many", async () => {
    backend.on("inbox", () => only([scheduled, draining]));
    renderApp("/home");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    expect(await within(nav).findByRole("link", { name: "Inbox, 1 waiting" })).toBeVisible();
  });
});

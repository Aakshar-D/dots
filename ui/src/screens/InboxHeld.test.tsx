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
  reason: { kind: "usage" },
};
const only = (held: HeldRun[]): Inbox => ({
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held,
  instructions: [],
  app_reviews: [],
});

describe("runs held for Claude usage in the Inbox", () => {
  it("lists each with its dot, what started it and when it starts, and Run anyway", async () => {
    backend.on("inbox", () => only([scheduled, draining]));
    backend.on("run_release_hold", () => ({ ...sample.run, id: scheduled.run_id }));
    const { user } = renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
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

  // Final review: Run anyway releases a background run from a share's hold; it cannot get
  // past Claude's limit, nor release a run of the person's own that waits for it.
  it("offer Run anyway only where it can release the run", async () => {
    const [holding] = sample.limits;
    const mine: HeldRun = { ...scheduled, run_id: "01J0RUN6", dot_name: "chatty", trigger: "chat" };
    backend.on("inbox", () => only([scheduled, mine]));
    backend.on("limits_get", () => holding);
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
    const [background, own] = within(section).getAllByRole("listitem");
    expect(within(background!).getByRole("button", { name: "Run anyway" })).toBeVisible();
    expect(within(own!).queryByRole("button", { name: "Run anyway" })).toBeNull();
  });

  it("offer no Run anyway while Claude's limit is reached", async () => {
    const [holding] = sample.limits;
    backend.on("inbox", () => only([scheduled]));
    backend.on("limits_get", () => ({ ...holding, rejected_until: scheduled.starts_after }));
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
    await within(section).findByRole("listitem");
    await waitFor(() =>
      expect(within(section).queryByRole("button", { name: "Run anyway" })).toBeNull(),
    );
  });

  it("shows a role hold's text and offers no Run anyway", async () => {
    const [holding] = sample.limits;
    const text = "Acme Engineering: your role no longer allows running dots";
    const role: HeldRun = {
      ...scheduled,
      run_id: "01J0RUN7",
      starts_after: null,
      reason: { kind: "role", account: "Acme Engineering", text },
    };
    backend.on("inbox", () => only([role]));
    backend.on("limits_get", () => holding);
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
    const [item] = await within(section).findAllByRole("listitem");
    expect(item).toHaveTextContent(`${role.dot_name} · Scheduled · ${text}`);
    expect(within(item!).queryByRole("button", { name: "Run anyway" })).toBeNull();
  });

  it("count as one in the rail's Inbox link, however many", async () => {
    backend.on("inbox", () => only([scheduled, draining]));
    renderApp("/home");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    expect(await within(nav).findByRole("link", { name: "Inbox, 1 waiting" })).toBeVisible();
  });
});

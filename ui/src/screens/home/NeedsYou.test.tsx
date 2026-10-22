import { act, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview, RuntimeEvent } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const at = (h: number) => `2026-10-09T${String(h).padStart(2, "0")}:00:00.000Z`;
/// An enabled dot named `name` whose last run was queued at `h` o'clock, with its counts.
const row = (name: string, h: number, needs_you: number, unread: number): DotOverview => ({
  dot: { ...sample.dot, id: `dot-${name}`, name, schedule: null, enabled: true },
  active_run: null,
  last_run: { ...sample.run, id: `run-${name}`, dot_id: `dot-${name}`, queued_at: at(h) },
  schedule_state: null,
  needs_you,
  unread,
});

const column = () => screen.getByRole("navigation", { name: "Dots" });
const item = (name: string) => within(column()).getByRole("listitem", { name });
const order = () =>
  within(column())
    .getAllByRole("listitem")
    .map((li) => li.getAttribute("aria-label"));

describe("what needs the person, on the home", () => {
  it("counts what is unread on each dot, in the waiting colour when it needs the person", async () => {
    backend.on("dot_overview", () => [
      row("asker", 9, 1, 3),
      row("reporter", 10, 0, 2),
      row("quiet", 11, 0, 0),
    ]);
    renderApp("/home/dot-quiet");
    await screen.findByRole("listitem", { name: "asker" });
    const asker = within(item("asker")).getByText("3 unread");
    expect(asker.closest("[data-tone]")).toHaveAttribute("data-tone", "waiting");
    const reporter = within(item("reporter")).getByText("2 unread");
    expect(reporter.closest("[data-tone]")).toHaveAttribute("data-tone", "muted");
    expect(within(item("quiet")).queryByText(/unread/)).toBeNull();
  });

  it("puts the dots that need the person first, then the newest activity", async () => {
    backend.on("dot_overview", () => [
      row("newest", 11, 0, 4),
      row("older", 9, 2, 2),
      row("oldest", 8, 1, 1),
      row("middle", 10, 0, 0),
    ]);
    renderApp("/home");
    await screen.findByRole("listitem", { name: "older" });
    expect(order()).toEqual(["older", "oldest", "newest", "middle"]);
  });

  it("shows the total unread on the Home link, and 99+ above 99", async () => {
    backend.on("unread_total", () => 4);
    const first = renderApp("/inbox");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const home = await within(nav).findByRole("link", { name: "Home, 4 unread" });
    expect(home).toHaveAttribute("title", "Home, 4 unread");
    expect(within(home).getByText("4")).toBeVisible();
    first.unmount();

    backend.on("unread_total", () => 120);
    renderApp("/inbox");
    const again = await screen.findByRole("navigation", { name: "Main" });
    const many = await within(again).findByRole("link", { name: "Home, 120 unread" });
    expect(within(many).getByText("99+")).toBeVisible();
  });

  it("counts again when a dot was seen", async () => {
    backend.on("dot_overview", () => [row("asker", 9, 1, 3)]);
    backend.on("unread_total", () => 3);
    const { listening } = renderApp("/home/dot-asker");
    expect(await screen.findByText("3 unread")).toBeInTheDocument();
    await listening();

    backend.on("dot_overview", () => [row("asker", 9, 1, 1)]);
    backend.on("unread_total", () => 1);
    act(() => backend.emit("runtime", { type: "dot_seen", dot_id: "dot-asker" }));
    expect(await screen.findByText("1 unread")).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "Home, 1 unread" })).toBeVisible();
  });

  it("marks the counts stale when a dot was seen, an approval moved or a message came", () => {
    const qc = createQueryClient();
    const counts = [keys.overview, keys.unreadTotal];
    const events: RuntimeEvent[] = [
      { type: "dot_seen", dot_id: sample.dot.id },
      { type: "approval_requested", approval: sample.approval },
      { type: "approval_decided", approval: sample.approval },
      { type: "approval_updated", approval: sample.approval },
      { type: "message_posted", message: sample.messages[0] },
      { type: "message_updated", message: sample.messages[1] },
    ];
    for (const event of events) {
      for (const key of counts) qc.setQueryData(key, []);
      applyRuntimeEvent(qc, event);
      for (const key of counts) {
        expect(qc.getQueryState(key)?.isInvalidated, `${event.type}: ${key.join("/")}`).toBe(true);
      }
    }
  });
});

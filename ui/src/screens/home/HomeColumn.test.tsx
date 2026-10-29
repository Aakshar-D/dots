import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview, Message, Run } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const at = (h: number) => `2026-10-09T${String(h).padStart(2, "0")}:00:00.000Z`;
/// An enabled dot named `name`, with no schedule and no run unless `over` gives them.
const row = (name: string, over: Partial<DotOverview> = {}): DotOverview => ({
  dot: { ...sample.dot, id: `dot-${name}`, name, schedule: null, enabled: true },
  active_run: null,
  last_run: null,
  schedule_state: null,
  needs_you: 0,
  unread: 0,
  ...over,
});
const ran = (h: number, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  queued_at: at(h),
  ...over,
});

const shopper = row("shopper", {
  active_run: ran(11, { id: "run-s", status: "awaiting_approval", ended_at: null }),
  last_run: ran(9, { summary: "Placed order 42\nand mailed the receipt" }),
});
const reader = row("reader", { last_run: ran(10, { summary: null, error: "timeout" }) });
const nightly = row("nightly", {
  last_run: ran(8),
  schedule_state: { ...sample.overview.schedule_state!, attention: null },
});
const idle = row("idle");

const column = () => screen.getByRole("navigation", { name: "Dots" });
const item = (name: string) => within(column()).getByRole("listitem", { name });

describe("the home's dots column", () => {
  it("lists the dots newest activity first, with status, last line and next run", async () => {
    backend.on("dot_overview", () => [idle, nightly, reader, shopper]);
    renderApp("/home");
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole(
      "listitem",
      { name: "shopper" },
    );
    expect(
      within(column())
        .getAllByRole("listitem")
        .map((li) => li.getAttribute("aria-label")),
    ).toEqual(["shopper", "reader", "nightly", "idle"]);
    const first = within(item("shopper"));
    expect(first.getByRole("img", { name: "waiting for you" })).toBeVisible();
    expect(first.getByText("Placed order 42")).toBeVisible();
    expect(within(item("reader")).getByText("timeout")).toBeVisible();
    expect(within(item("reader")).getByRole("img", { name: "idle" })).toBeVisible();
    const third = within(item("nightly"));
    expect(
      third.getByText(`next ${localTime(nightly.schedule_state!.next_fire)}`),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "New dot" })).toHaveAttribute("href", "/dots/new");
  });

  it("starts with New dot, then the heading over the list", async () => {
    backend.on("dot_overview", () => [reader, shopper]);
    renderApp(`/home/${reader.dot.id}`);
    const view = within(await screen.findByRole("navigation", { name: "Dots" }));
    const create = view.getByRole("link", { name: "New dot" });
    expect(create).toHaveTextContent("New dot");
    expect(create).toHaveAttribute("href", "/dots/new");
    const heading = view.getByRole("heading", { name: "Dots" });
    const list = view.getByRole("list");
    const before = (a: Node, b: Node) =>
      (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
    expect(before(create, heading)).toBe(true);
    expect(before(heading, list)).toBe(true);
  });

  it("marks running, attention and off, and a dot whose question is open as waiting", async () => {
    const question: Message = { ...sample.messages[0], dot_id: "dot-asker" };
    backend.on("messages_list", () => [question]);
    backend.on("dot_overview", () => [
      row("asker"),
      row("busy", { active_run: ran(11, { status: "running", ended_at: null }) }),
      row("failing", { schedule_state: sample.overview.schedule_state }),
      row("disabled", { dot: { ...sample.dot, id: "dot-disabled", name: "disabled", enabled: false } }),
    ]);
    renderApp("/home");
    await screen.findByRole("listitem", { name: "asker" });
    const mark = (name: string) => within(item(name)).getByRole("img");
    await waitFor(() => expect(mark("asker")).toHaveAccessibleName("waiting for you"));
    expect(mark("busy")).toHaveAccessibleName("running");
    expect(mark("failing")).toHaveAccessibleName("needs attention");
    expect(mark("disabled")).toHaveAccessibleName("off");
  });

  it("says when a dot's next follow-up fires", async () => {
    backend.on("dot_overview", () => [idle]);
    const soon = new Date(Date.now() + 3 * 3_600_000).toISOString();
    backend.on("follow_ups_list", () => [
      { ...sample.followUps[0], dot_id: idle.dot.id, due_at: soon },
    ]);
    renderApp("/home");
    expect(await screen.findByText("Next follow-up in 3 h")).toBeVisible();
  });

  it("opens the first dot of the column, then the dot last opened", async () => {
    backend.on("dot_overview", () => [reader, shopper]);
    const first = renderApp("/home");
    await waitFor(() =>
      expect(first.router.state.location.pathname).toBe(`/home/${shopper.dot.id}`),
    );
    await first.user.click(within(item("reader")).getByRole("link"));
    await waitFor(() =>
      expect(first.router.state.location.pathname).toBe(`/home/${reader.dot.id}`),
    );
    expect(within(item("reader")).getByRole("link")).toHaveAttribute("aria-current", "page");
    expect(within(item("shopper")).getByRole("link")).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("heading", { level: 1, name: "reader" })).toBeVisible();
    first.unmount();

    const again = renderApp("/home");
    await waitFor(() =>
      expect(again.router.state.location.pathname).toBe(`/home/${reader.dot.id}`),
    );
  });

  it("forgets a dot last opened that no longer exists", async () => {
    window.localStorage.setItem("dots.home.dot", "dot-gone");
    backend.on("dot_overview", () => [reader]);
    const { router } = renderApp("/home");
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${reader.dot.id}`));
  });

  it("moves the focus through the dots with the arrow keys, Home and End; Enter opens one", async () => {
    backend.on("dot_overview", () => [shopper, reader, nightly]);
    const { user, router } = renderApp(`/home/${reader.dot.id}`);
    const link = within(await screen.findByRole("listitem", { name: "reader" })).getByRole("link");
    act(() => link.focus());
    const before = router.state.location.key;
    await user.keyboard("{ArrowDown}");
    expect(within(item("nightly")).getByRole("link")).toHaveFocus();
    // Moving the focus opens nothing.
    expect(router.state.location.pathname).toBe(`/home/${reader.dot.id}`);
    expect(router.state.location.key).toBe(before);
    // The last stays the last.
    await user.keyboard("{ArrowDown}");
    expect(within(item("nightly")).getByRole("link")).toHaveFocus();
    await user.keyboard("{Home}");
    expect(within(item("shopper")).getByRole("link")).toHaveFocus();
    await user.keyboard("{End}");
    expect(within(item("nightly")).getByRole("link")).toHaveFocus();
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(within(item("shopper")).getByRole("link")).toHaveFocus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${shopper.dot.id}`));
  });

  it("offers to create the first dot when there is none", async () => {
    renderApp("/home");
    const create = await screen.findByRole("link", { name: "Create your first dot" });
    expect(create).toHaveAttribute("href", "/dots/new");
    expect(screen.getByRole("navigation", { name: "Main" })).toBeVisible();
  });

  it("goes back to the home when the open dot is deleted", async () => {
    let rows = [reader, shopper];
    backend.on("dot_overview", () => rows);
    const { router, queryClient } = renderApp(`/home/${reader.dot.id}`);
    await screen.findByRole("heading", { level: 1, name: "reader" });
    rows = [shopper];
    await act(() => queryClient.invalidateQueries({ queryKey: ["overview"] }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${shopper.dot.id}`));
  });

  it("shows the dot's name and its 'off' mark in the header of a disabled dot", async () => {
    const off = row("paused", {
      dot: { ...sample.dot, id: "dot-paused", name: "paused", enabled: false },
    });
    backend.on("dot_overview", () => [off]);
    renderApp(`/home/${off.dot.id}`);
    const header = (await screen.findByRole("heading", { level: 1, name: "paused" })).closest(
      "header",
    )!;
    expect(within(header).getByRole("img", { name: "off" })).toBeVisible();
    // The mark says it; the header has no status text of its own.
    expect(within(header).queryByText("off")).toBeNull();
  });

  it("shows a failed load of the dots", async () => {
    backend.on("dot_overview", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp("/home");
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
  });
});

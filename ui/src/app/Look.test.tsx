import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const nav = () => screen.findByRole("navigation", { name: "Main" });
/// The tone of the count on a rail link.
const tone = (link: HTMLElement, count: string) =>
  within(link).getByText(count).closest("[data-tone]")?.getAttribute("data-tone");

const row = (status: "running" | "idle"): DotOverview => ({
  dot: { ...sample.dot, schedule: null, enabled: true },
  active_run: status === "running" ? { ...sample.run, status: "running", ended_at: null } : null,
  last_run: null,
  schedule_state: null,
  needs_you: 0,
  unread: 0,
});

describe("the rail (deep-teal spec section 4)", () => {
  it("shows the Home count muted while nothing needs the person", async () => {
    backend.on("unread_total", () => 2);
    renderApp("/inbox");
    const home = await within(await nav()).findByRole("link", { name: "Home, 2 unread" });
    expect(tone(home, "2")).toBe("muted");
  });

  it("shows the Home and Inbox counts in the needs colour while an approval waits", async () => {
    backend.on("unread_total", () => 2);
    backend.on("inbox", () => ({
      approvals: sample.inbox.approvals,
      review_runs: [],
      attention: [],
      tools_changed: [],
      watchers: [], held: [], app_reviews: [],
    }));
    renderApp("/library");
    const rail = within(await nav());
    // The Inbox's count shows once the inbox has loaded, and the tone with it.
    expect(tone(await rail.findByRole("link", { name: "Inbox, 1 waiting" }), "1")).toBe("needs");
    expect(tone(await rail.findByRole("link", { name: "Home, 2 unread" }), "2")).toBe("needs");
  });

  it("counts an open question as something that needs the person", async () => {
    backend.on("messages_list", () => [sample.messages[0]]);
    renderApp("/library");
    const inbox = await within(await nav()).findByRole("link", { name: "Inbox, 1 waiting" });
    expect(tone(inbox, "1")).toBe("needs");
  });

  it("marks the screen in front", async () => {
    renderApp("/library");
    const rail = within(await nav());
    expect(await rail.findByRole("link", { name: "Library" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(rail.getByRole("link", { name: "Settings" })).not.toHaveAttribute("aria-current");
  });
});

describe("the dots column's marks", () => {
  it("names each status, and the running pulse stops under reduced motion", async () => {
    backend.on("dot_overview", () => [row("running")]);
    renderApp(`/home/${sample.dot.id}`);
    const column = within(await screen.findByRole("navigation", { name: "Dots" }));
    const mark = await column.findByRole("img", { name: "running" });
    expect(mark).toHaveAttribute("data-status", "running");
    expect(mark).toHaveClass("motion-reduce:animate-none");
  });
});

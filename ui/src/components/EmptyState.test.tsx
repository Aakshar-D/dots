import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Button } from "@/components/ui/button";
import { renderApp } from "@/test/render";

/// The empty state around `text`: its icon, hidden from assistive technology.
const icon = (text: HTMLElement) =>
  text.closest('[data-slot="empty-state"]')?.querySelector('svg[aria-hidden="true"]');

describe("polish (deep-teal spec section 4)", () => {
  it("shows an empty Inbox as an icon and one muted line", async () => {
    renderApp("/inbox");
    expect(icon(await screen.findByText(/^Nothing waits for you\./))).toBeTruthy();
  });

  it("shows an empty Library tab as an icon and one muted line", async () => {
    const { user } = renderApp("/library");
    expect(icon(await screen.findByText("No MCP servers yet."))).toBeTruthy();
    for (const [tab, line] of [
      ["Skills", "No skills yet."],
      ["Plugins", "No plugins yet."],
      ["Context", "No context collections yet."],
    ] as const) {
      await user.click(screen.getByRole("tab", { name: tab }));
      expect(icon(await screen.findByText(line))).toBeTruthy();
    }
  });

  it("gives a button the ring on keyboard focus, not a blurred halo", () => {
    render(<Button>Go</Button>);
    const button = screen.getByRole("button", { name: "Go" });
    expect(button).toHaveClass("focus-visible:outline-ring");
    expect(button).not.toHaveClass("focus-visible:ring-3");
  });
});

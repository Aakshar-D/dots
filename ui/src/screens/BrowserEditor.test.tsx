import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot, DotSpec } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot: Dot = {
  ...sample.dot,
  browser: true,
  policy: {
    ...sample.dot.policy,
    rules: [
      ...sample.dot.policy.rules,
      { tool: "Browser(shop.com)", action: "allow" },
      { tool: "BrowserRisky(shop.com)", action: "allow" },
      { tool: "Browser(news.example)", action: "allow" },
      { tool: "Browser(bank.com)", action: "deny" },
    ],
  },
};

describe("the dot editor's browser", () => {
  it("lists the approved sites, removes one, and opens the browser", async () => {
    // Assigned in the handler: declared so, TypeScript does not narrow it to null.
    let saved = null as DotSpec | null;
    backend.on("get_dot", () => dot);
    backend.on("update_dot", (args) => {
      saved = args.spec as DotSpec;
      return { ...dot, ...saved };
    });
    backend.on("browser_open", () => null);
    const { user } = renderApp(`/dots/${dot.id}`);
    expect(await screen.findByRole("switch", { name: "Browser" })).toBeChecked();
    expect(
      screen.getByText("Opens a visible Microsoft Edge window with this dot's own profile."),
    ).toBeVisible();
    const sites = screen.getByRole("list", { name: "Sites" });
    const shop = within(sites).getByRole("listitem", { name: "shop.com" });
    expect(shop).toHaveTextContent("risky actions allowed");
    expect(within(sites).getByRole("listitem", { name: "news.example" })).not.toHaveTextContent(
      "risky actions allowed",
    );
    // A denied site is not an approved one.
    expect(within(sites).queryByRole("listitem", { name: "bank.com" })).toBeNull();

    await user.click(within(shop).getByRole("button", { name: "Remove" }));
    expect(within(sites).queryByRole("listitem", { name: "shop.com" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Open the browser" }));
    await waitFor(() => expect(backend.callsTo("browser_open")).toEqual([{ dotId: dot.id }]));

    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved).not.toBeNull());
    const tools = saved!.policy.rules.map((r) => r.tool);
    expect(saved!.browser).toBe(true);
    expect(tools).not.toContain("Browser(shop.com)");
    expect(tools).not.toContain("BrowserRisky(shop.com)");
    expect(tools).toContain("Browser(news.example)");
    expect(tools).toContain("Browser(bank.com)");
    expect(saved!.policy.preset).toBe("custom");
  });

  it("turns the browser on for a new dot, whose browser opens once it is saved", async () => {
    const { user } = renderApp("/dots/new");
    const toggle = await screen.findByRole("switch", { name: "Browser" });
    expect(toggle).not.toBeChecked();
    expect(screen.queryByRole("heading", { name: "Sites" })).toBeNull();
    await user.click(toggle);
    expect(toggle).toBeChecked();
    expect(screen.getByRole("heading", { name: "Sites" })).toBeVisible();
    expect(screen.getByText(/^No site is approved yet/)).toBeVisible();
    expect(screen.getByText("Save the dot to open its browser.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Open the browser" })).toBeNull();
  });
});

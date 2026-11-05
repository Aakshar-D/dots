import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";

const none = {
  path: null,
  blocks_skipping_asks: false,
  forced_ask: [],
  forced_deny: [],
  managed_rules_only: false,
  error: null,
};

describe("Settings: the company's Claude Code policy", () => {
  it("says when none was found", async () => {
    renderApp("/settings/claude");
    expect(await screen.findByText("None found")).toBeVisible();
  });

  it("shows where a policy was found", async () => {
    backend.on("managed_policy", () => ({ ...none, path: "/etc/claude-code/managed-settings.json" }));
    renderApp("/settings/claude");
    await waitFor(() =>
      expect(screen.getByTestId("managed-policy")).toHaveTextContent(
        "Found at /etc/claude-code/managed-settings.json",
      ),
    );
  });

  it("shows why a policy could not be read", async () => {
    backend.on("managed_policy", () => ({
      ...none,
      path: "C:/p/managed-settings.json",
      error: "Couldn't read your company's Claude Code policy at C:/p/managed-settings.json: bad",
    }));
    renderApp("/settings/claude");
    expect(await screen.findByText(/Couldn't read your company's Claude Code policy/)).toBeVisible();
  });
});

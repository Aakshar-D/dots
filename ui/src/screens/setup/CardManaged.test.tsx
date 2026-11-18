import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Policy, SetupProposal } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { SetupCard } from "@/screens/setup/SetupCard";
import { backend } from "@/test/backend";

const proposal: SetupProposal = {
  name: "Digest",
  instructions: "Do it.",
  folder: null,
  schedule: null,
  preset: "trusted",
  extras: [],
  uses: { mcp_servers: [], skills: [], plugins: [], context: [], apps: [] },
  first_message: "",
  notes: "",
};
const trusted: Policy = {
  preset: "trusted",
  rules: [],
  default: "allow",
  mcp_default: "ask",
  ask_send_delete: true,
};
const managed = {
  path: "C:/Program Files/ClaudeCode/managed-settings.json",
  blocks_skipping_asks: false,
  forced_ask: [],
  forced_deny: ["Bash"],
  managed_rules_only: false,
  error: null,
};

function card() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <SetupCard proposal={proposal} newDotRoot="C:\Dots" onChange={() => {}} />
    </QueryClientProvider>,
  );
}

describe("the card and the company's Claude Code policy", () => {
  it("tells the person when the proposal conflicts", async () => {
    backend.on("policy_preset", () => trusted);
    backend.on("managed_policy_check", () => ({
      managed,
      conflicts: [{ kind: "deny", rule: "Bash", dot_rule: "everything else" }],
    }));
    card();
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("refuses these for every sol");
    expect(note).toHaveTextContent(managed.path);
    expect(backend.callsTo("managed_policy_check")[0]).toMatchObject({ engine: "claude" });
  });

  it("says nothing without a conflict", async () => {
    backend.on("policy_preset", () => trusted);
    card();
    await waitFor(() => expect(backend.callsTo("managed_policy_check").length).toBe(1));
    expect(screen.queryByRole("note")).toBeNull();
  });
});

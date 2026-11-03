import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { Policy } from "@/api/types";
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { backend } from "@/test/backend";

const sandboxed: Policy = {
  preset: "sandboxed",
  rules: [
    { tool: "Read", action: "allow" },
    { tool: "Write(./**)", action: "allow" },
  ],
  default: "ask",
  mcp_default: "ask",
};
const readOnly: Policy = {
  preset: "read-only",
  rules: [{ tool: "Read", action: "allow" }],
  default: "deny",
  mcp_default: "deny",
};

function setup(initial: Policy = sandboxed) {
  let latest = initial;
  function Host() {
    const [policy, setPolicy] = useState(initial);
    latest = policy;
    return <PolicyEditor policy={policy} onChange={setPolicy} />;
  }
  render(<Host />);
  return { user: userEvent.setup(), policy: () => latest };
}

describe("the policy editor", () => {
  it("replaces the rules with a chosen preset's", async () => {
    backend.on("policy_preset", () => readOnly);
    const { user, policy } = setup();
    await user.selectOptions(screen.getByLabelText("Preset"), "read-only");
    await waitFor(() => expect(policy()).toEqual(readOnly));
    await user.click(screen.getByText("Show rules"));
    expect(backend.callsTo("policy_preset")).toEqual([{ preset: "read-only" }]);
    expect(screen.getByLabelText("Anything else")).toHaveValue("deny");
  });

  it("becomes custom when a rule, the rule list or the default is edited", async () => {
    const { user, policy } = setup();
    await user.click(screen.getByText("Show rules"));
    await user.selectOptions(screen.getByLabelText("Rule 2 action"), "deny");
    expect(policy()).toEqual({
      ...sandboxed,
      preset: "custom",
      rules: [sandboxed.rules[0], { tool: "Write(./**)", action: "deny" }],
    });
    expect(screen.getByLabelText("Preset")).toHaveValue("custom");

    await user.click(screen.getByRole("button", { name: "Add rule" }));
    await user.type(screen.getByLabelText("Rule 3 tool"), "Bash(git push:*)");
    expect(policy().rules[2]).toEqual({ tool: "Bash(git push:*)", action: "ask" });

    await user.click(screen.getByRole("button", { name: "Remove rule 1" }));
    expect(policy().rules.map((r) => r.tool)).toEqual(["Write(./**)", "Bash(git push:*)"]);

    await user.selectOptions(screen.getByLabelText("Anything else"), "deny");
    expect(policy().default).toBe("deny");
  });

  it("keeps the rules when Custom is chosen, without asking the backend", async () => {
    const { user, policy } = setup();
    await user.selectOptions(screen.getByLabelText("Preset"), "custom");
    expect(policy()).toEqual({ ...sandboxed, preset: "custom" });
    expect(backend.callsTo("policy_preset")).toEqual([]);
  });

  it("folds the rules under a one-line sentence for a named preset", async () => {
    const { user } = setup();
    expect(screen.getByLabelText("Preset")).toHaveValue("sandboxed");
    expect(
      screen.getByText(
        "May edit files in its folder and run a few safe git commands. Asks before anything else.",
      ),
    ).toBeVisible();
    expect(screen.getByLabelText("Rule 1 tool")).not.toBeVisible();
    await user.click(screen.getByText("Show rules"));
    expect(screen.getByLabelText("Rule 1 tool")).toBeVisible();
  });

  it("shows the rules at once for a custom policy", () => {
    setup({ ...sandboxed, preset: "custom" });
    expect(screen.getByText("Your own rules.")).toBeVisible();
    expect(screen.getByLabelText("Rule 1 tool")).toBeVisible();
  });

  it("opens the rules when Custom is picked while they are folded", async () => {
    const { user } = setup();
    expect(screen.getByLabelText("Rule 1 tool")).not.toBeVisible();
    await user.selectOptions(screen.getByLabelText("Preset"), "custom");
    expect(screen.getByLabelText("Rule 1 tool")).toBeVisible();
  });
});

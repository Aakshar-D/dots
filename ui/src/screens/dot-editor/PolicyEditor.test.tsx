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
};
const readOnly: Policy = {
  preset: "read-only",
  rules: [{ tool: "Read", action: "allow" }],
  default: "deny",
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
    expect(backend.callsTo("policy_preset")).toEqual([{ preset: "read-only" }]);
    expect(screen.getByLabelText("Anything else")).toHaveValue("deny");
  });

  it("becomes custom when a rule, the rule list or the default is edited", async () => {
    const { user, policy } = setup();
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
});

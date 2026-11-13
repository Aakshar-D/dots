import { render, screen, waitFor, within } from "@testing-library/react";
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
  ask_send_delete: true,
};
const readOnly: Policy = {
  preset: "read-only",
  rules: [{ tool: "Read", action: "allow" }],
  default: "deny",
  mcp_default: "deny",
  ask_send_delete: true,
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

  it("opens the rules when they have an error", () => {
    const onChange = () => {};
    const { rerender } = render(<PolicyEditor policy={sandboxed} onChange={onChange} />);
    expect(screen.getByText("Show rules")).toBeVisible();
    rerender(
      <PolicyEditor policy={sandboxed} onChange={onChange} error="Rule 1 needs a tool." />,
    );
    expect(screen.getByText("Hide rules")).toBeVisible();
    expect(screen.getByText("Rule 1 needs a tool.")).toBeVisible();
  });

  it("starts with the rules open when they have an error", () => {
    render(<PolicyEditor policy={sandboxed} onChange={() => {}} error="Rule 1 needs a tool." />);
    expect(screen.getByText("Hide rules")).toBeVisible();
  });

  it("opens the rules when the policy becomes custom from outside", () => {
    const onChange = () => {};
    const { rerender } = render(<PolicyEditor policy={sandboxed} onChange={onChange} />);
    expect(screen.getByText("Show rules")).toBeVisible();
    rerender(<PolicyEditor policy={{ ...sandboxed, preset: "custom" }} onChange={onChange} />);
    expect(screen.getByText("Hide rules")).toBeVisible();
  });
});

describe("the always-allowed list", () => {
  const ruled: Policy = {
    preset: "sandboxed",
    rules: [
      { tool: "Read", action: "allow" },
      { tool: "Bash(git status:*)", action: "allow" },
      { tool: "Bash(git push:*)", action: "ask" },
      { tool: "Read(**/.git/**)", action: "deny" },
      { tool: "Notify", action: "allow" },
      { tool: "Memory", action: "allow" },
      { tool: "WebFetch(domain:example.com)", action: "allow" },
    ],
    default: "ask",
    mcp_default: "ask",
    ask_send_delete: true,
  };
  const list = () => screen.getByRole("list", { name: "Always allowed" });

  it("shows the allow rules in words, without Sage's own tools", () => {
    setup(ruled);
    const items = within(list()).getAllByRole("listitem");
    expect(items.map((item) => item.textContent)).toEqual([
      expect.stringContaining("Read: every call"),
      expect.stringContaining("Bash: commands that start with git status"),
      expect.stringContaining("WebFetch: example.com and its subdomains"),
    ]);
    expect(within(items[1]).getByText("Bash(git status:*)")).toBeVisible();
  });

  it("takes a rule back with Remove, and the policy becomes custom", async () => {
    const { user, policy } = setup(ruled);
    await user.click(
      screen.getByRole("button", { name: "Remove Bash: commands that start with git status" }),
    );
    expect(policy().preset).toBe("custom");
    expect(policy().rules.map((r) => r.tool)).toEqual([
      "Read",
      "Bash(git push:*)",
      "Read(**/.git/**)",
      "Notify",
      "Memory",
      "WebFetch(domain:example.com)",
    ]);
    expect(within(list()).getAllByRole("listitem")).toHaveLength(2);
  });

  it("says when nothing is always allowed", () => {
    setup({ ...ruled, rules: [{ tool: "Notify", action: "allow" }] });
    expect(screen.getByText("Nothing is always allowed.")).toBeVisible();
  });
});

describe("an always-allowed rule that covers a private folder", () => {
  it("says it always asks", async () => {
    backend.on("rules_touch_private", ({ patterns }) =>
      (patterns as string[]).map((p) => p === "Read(~/.ssh/**)"),
    );
    setup({
      preset: "custom",
      rules: [
        { tool: "Read(src/**)", action: "allow" },
        { tool: "Read(~/.ssh/**)", action: "allow" },
      ],
      default: "ask",
      mcp_default: "ask",
      ask_send_delete: true,
    });
    const items = within(screen.getByRole("list", { name: "Always allowed" })).getAllByRole(
      "listitem",
    );
    expect(await within(items[1]).findByText("Always asks: private folder")).toBeVisible();
    expect(within(items[0]).queryByText("Always asks: private folder")).toBeNull();
    expect(backend.callsTo("rules_touch_private")).toEqual([
      { patterns: ["Read(src/**)", "Read(~/.ssh/**)"] },
    ]);
  });

  it("shows the list as it is when the check fails", async () => {
    backend.on("rules_touch_private", () => {
      throw { kind: "internal", message: "no" };
    });
    setup({
      preset: "custom",
      rules: [{ tool: "Read(src/**)", action: "allow" }],
      default: "ask",
      mcp_default: "ask",
      ask_send_delete: true,
    });
    await waitFor(() => expect(backend.callsTo("rules_touch_private").length).toBe(1));
    expect(screen.getByText("Read: everything under src/")).toBeVisible();
    expect(screen.queryByText("Always asks: private folder")).toBeNull();
  });
});

describe("the company's Claude Code policy", () => {
  const managed = {
    path: "C:/Program Files/ClaudeCode/managed-settings.json",
    blocks_skipping_asks: false,
    forced_ask: ["Bash(git push:*)"],
    forced_deny: ["WebFetch"],
    managed_rules_only: false,
    error: null,
  };
  const conflict = { kind: "deny", rule: "WebFetch", dot_rule: "everything else" };

  it("says what is refused, with the policy file and the ask line", async () => {
    backend.on("managed_policy_check", () => ({ managed, conflicts: [conflict] }));
    setup();
    const note = await screen.findByRole("note");
    expect(note).toHaveTextContent("refuses these for every sol");
    expect(note).toHaveTextContent("WebFetch");
    expect(note).toHaveTextContent("Sage's own approval check");
    expect(note).toHaveTextContent(managed.path);
  });

  it("has no ask line when the policy has no ask rules", async () => {
    backend.on("managed_policy_check", () => ({
      managed: { ...managed, forced_ask: [] },
      conflicts: [conflict],
    }));
    setup();
    const note = await screen.findByRole("note");
    expect(note).not.toHaveTextContent("approval check");
  });

  it("says nothing when the policy does not conflict", async () => {
    backend.on("managed_policy_check", () => ({ managed, conflicts: [] }));
    setup();
    await waitFor(() => expect(backend.callsTo("managed_policy_check").length).toBe(1));
    expect(screen.queryByRole("note")).toBeNull();
  });

  it("is not asked about a sol that does not run on the Claude CLI", () => {
    function Host() {
      const [policy, setPolicy] = useState(sandboxed);
      return <PolicyEditor policy={policy} onChange={setPolicy} engine="local" />;
    }
    render(<Host />);
    expect(backend.callsTo("managed_policy_check")).toEqual([]);
  });
});

describe("always ask before sending or deleting", () => {
  const name = "Always ask before sending or deleting";

  it("is on for a preset and says what it does", () => {
    setup();
    const toggle = screen.getByRole("switch", { name });
    expect(toggle).toBeChecked();
    expect(screen.getByText(/always waits for your approval, even where a rule allows it/)).toBeVisible();
  });

  it("turns off and on without making the policy custom", async () => {
    const { user, policy } = setup();
    await user.click(screen.getByRole("switch", { name }));
    expect(policy()).toEqual({ ...sandboxed, ask_send_delete: false });
    expect(screen.getByRole("switch", { name })).not.toBeChecked();
    expect(screen.getByText(/follow the rules above like any other call/)).toBeVisible();
    await user.click(screen.getByRole("switch", { name }));
    expect(policy()).toEqual(sandboxed);
  });

  it("stays as the person set it when they choose another preset", async () => {
    backend.on("policy_preset", () => readOnly);
    const { user, policy } = setup({ ...sandboxed, ask_send_delete: false });
    await user.selectOptions(screen.getByLabelText("Preset"), "read-only");
    await waitFor(() => expect(policy()).toEqual({ ...readOnly, ask_send_delete: false }));
  });
});

import { describe, expect, it } from "vitest";

import { describeAllowRule, isDotsOwnRule } from "@/lib/allow-rules";

describe("an allow rule in words", () => {
  it("says what each kind of rule allows", () => {
    expect(describeAllowRule("Bash(git status:*)")).toBe("Bash: commands that start with git status");
    expect(describeAllowRule("PowerShell(npm test)")).toBe("PowerShell: the command npm test");
    expect(describeAllowRule("Read(src/**)")).toBe("Read: everything under src/");
    expect(describeAllowRule("Write(./**)")).toBe("Write: everything in its folder");
    expect(describeAllowRule("Edit(a.txt)")).toBe("Edit: a.txt");
    expect(describeAllowRule("WebFetch(domain:example.com)")).toBe(
      "WebFetch: example.com and its subdomains",
    );
    expect(describeAllowRule("mcp__github__*")).toBe("every tool of the server github");
    expect(describeAllowRule("mcp__github__create_issue")).toBe(
      "create_issue of the server github",
    );
    expect(describeAllowRule("Read")).toBe("Read: every call");
    expect(describeAllowRule("*")).toBe("every tool");
    expect(describeAllowRule("App(notes.add)")).toBe("App: notes.add");
  });

  it("knows Dots' own tools", () => {
    for (const tool of [
      "Skill",
      "Context",
      "Notify",
      "Ask",
      "FollowUp",
      "Memory",
      "Workflow",
      "App",
      "ProposeApp",
      "Instructions",
    ]) {
      expect(isDotsOwnRule(tool)).toBe(true);
    }
    for (const tool of ["Read", "Bash(git status:*)", "mcp__github__*", "WebFetch", "App(notes)"]) {
      expect(isDotsOwnRule(tool)).toBe(false);
    }
  });
});

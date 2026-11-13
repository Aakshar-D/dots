// An allow rule of a dot's policy, said in words for the "Always allowed" list (approval
// guards spec section 4). The rule's own text is shown beside these words.

/// Dots' own tools, which every preset allows and which never reach the person's machine: the
/// list leaves out their bare rules. A rule with an argument (`App(notes)`) is shown, and so is
/// `Workflow`, which starts real work.
const DOTS_OWN = new Set([
  "Skill",
  "Context",
  "Notify",
  "Ask",
  "FollowUp",
  "Memory",
  "App",
  "ProposeApp",
  "Instructions",
  "MakeDots",
]);

const COMMAND_TOOLS = new Set(["Bash", "PowerShell"]);

export function isDotsOwnRule(tool: string): boolean {
  return DOTS_OWN.has(tool.trim());
}

/// What an allow rule allows, in words: `Bash(git status:*)` is "Bash: commands that start
/// with git status".
export function describeAllowRule(rule: string): string {
  const text = rule.trim();
  if (text === "*") return "every tool";
  const open = text.indexOf("(");
  if (open > 0 && text.endsWith(")")) {
    const tool = text.slice(0, open);
    const spec = text.slice(open + 1, -1).trim();
    if (COMMAND_TOOLS.has(tool)) {
      return spec.endsWith(":*")
        ? `${tool}: commands that start with ${spec.slice(0, -2)}`
        : `${tool}: the command ${spec}`;
    }
    if (tool === "WebFetch" && spec.startsWith("domain:")) {
      return `${tool}: ${spec.slice("domain:".length)} and its subdomains`;
    }
    if (spec === "./**" || spec === "**") return `${tool}: everything in its folder`;
    if (spec.endsWith("/**")) return `${tool}: everything under ${spec.slice(0, -2)}`;
    return `${tool}: ${spec}`;
  }
  const mcp = /^mcp__(.+?)__(.+)$/.exec(text);
  if (mcp) {
    const [, server, name] = mcp;
    return name === "*" ? `every tool of the server ${server}` : `${name} of the server ${server}`;
  }
  return `${text}: every call`;
}

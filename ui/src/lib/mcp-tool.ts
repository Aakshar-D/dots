// MCP tool names as the engines report them: `mcp__<server>__<tool>`.

/// The server and tool of an MCP tool name, or null for any other name. A server name never
/// holds `__`, so the first `__` after the prefix ends it; the tool may hold more.
export function parseMcpTool(name: string): { server: string; tool: string } | null {
  const match = /^mcp__(.+?)__(.+)$/.exec(name);
  return match ? { server: match[1]!, tool: match[2]! } : null;
}

/// "salesforce › query" for `mcp__salesforce__query`. A server that comes with a plugin
/// (`owners`: server name to plugin name) shows as its plugin. Another name stays as it is.
export function mcpToolLabel(name: string, owners: ReadonlyMap<string, string>): string {
  const parsed = parseMcpTool(name);
  if (!parsed) return name;
  return `${owners.get(parsed.server) ?? parsed.server} › ${parsed.tool}`;
}

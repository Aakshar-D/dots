import { describe, expect, it } from "vitest";

import { mcpToolLabel, parseMcpTool } from "@/lib/mcp-tool";

describe("MCP tool names", () => {
  it("splits mcp__<server>__<tool>", () => {
    expect(parseMcpTool("mcp__salesforce__query")).toEqual({ server: "salesforce", tool: "query" });
    // a tool name may hold a double underscore
    expect(parseMcpTool("mcp__github__list__issues")).toEqual({ server: "github", tool: "list__issues" });
  });

  it("leaves other names alone", () => {
    for (const name of ["Bash", "mcp__", "mcp__x", "mcp____t", "mcp__x__", "skill"]) {
      expect(parseMcpTool(name)).toBeNull();
    }
  });

  it("shows server and tool, or the plugin that owns the server", () => {
    const owners = new Map([["plugin-pdf-tools-render", "pdf-tools"]]);
    expect(mcpToolLabel("mcp__salesforce__query", owners)).toBe("salesforce › query");
    expect(mcpToolLabel("mcp__plugin-pdf-tools-render__page", owners)).toBe("pdf-tools › page");
    expect(mcpToolLabel("Bash", owners)).toBe("Bash");
  });
});

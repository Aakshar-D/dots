import { McpTool } from "@/api/types";

/// The tools in a tool list the core keeps as JSON (`accepted_tools`, `seen_tools`). Anything
/// that is not a list of tools reads as an empty list.
export function parseTools(value: unknown): McpTool[] {
  const parsed = McpTool.array().safeParse(value);
  return parsed.success ? parsed.data : [];
}

/// Whether the server calls the tool read-only. Its own claim, and only a true hint counts.
export function isReadOnly(tool: McpTool): boolean {
  const annotations = tool.annotations;
  return (
    typeof annotations === "object" &&
    annotations !== null &&
    (annotations as Record<string, unknown>).readOnlyHint === true
  );
}

/// JSON with the keys of every object sorted: two values that mean the same text the same.
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object" && value !== null) {
    const entries = Object.entries(value)
      .filter(([, v]) => v !== undefined)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

export type ToolPart = "description" | "inputs" | "annotations";

export type ToolsDiff = {
  added: McpTool[];
  removed: McpTool[];
  changed: { tool: McpTool; parts: ToolPart[] }[];
};

/// What differs between the accepted tools and the ones last seen. Only what the fingerprint
/// covers counts (spec section 3): a tool's `title` is not compared.
export function diffTools(accepted: McpTool[], seen: McpTool[]): ToolsDiff {
  const before = new Map(accepted.map((t) => [t.name, t]));
  const after = new Map(seen.map((t) => [t.name, t]));
  const changed: ToolsDiff["changed"] = [];
  for (const tool of seen) {
    const old = before.get(tool.name);
    if (!old) continue;
    const parts: ToolPart[] = [];
    if ((old.description ?? "") !== (tool.description ?? "")) parts.push("description");
    if (canonical(old.inputSchema) !== canonical(tool.inputSchema)) parts.push("inputs");
    if (canonical(old.annotations ?? null) !== canonical(tool.annotations ?? null)) {
      parts.push("annotations");
    }
    if (parts.length > 0) changed.push({ tool, parts });
  }
  return {
    added: seen.filter((t) => !before.has(t.name)),
    removed: accepted.filter((t) => !after.has(t.name)),
    changed,
  };
}

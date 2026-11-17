// Reads a line the transcript keeps as raw, for the row that shows it.

/// A raw item of the transcript: `engine` when the line is the engine's own output, which may be
/// a JSON line of the CLI that the core does not read.
export type RawLine = { label: string; text: string; engine: boolean };

export type RawView =
  /// The engine's count of the tokens the model thought with since its last count.
  | { kind: "thinking"; tokens: number }
  /// A JSON object, as its fields.
  | { kind: "fields"; title: string; fields: { key: string; value: string }[] }
  | { kind: "text"; title: string; text: string };

/// The fields of an engine line that say nothing to a person: the ids of the session and of the
/// line.
const ENGINE_IDS = new Set(["session_id", "uuid"]);

/// `hook_started` as "hook started".
function words(name: string): string {
  return name.replace(/[_-]+/g, " ").trim();
}

function fieldValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "object" && value !== null) return JSON.stringify(value, null, 2);
  return String(value);
}

/// What a raw line shows as: an engine line of thinking progress as its token count, any other
/// JSON object as its fields (an engine line titled by its type, as "System · hook started"),
/// and anything else as the text it is.
export function rawView(line: RawLine): RawView {
  let value: unknown;
  try {
    value = JSON.parse(line.text);
  } catch {
    return { kind: "text", title: line.label, text: line.text };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { kind: "text", title: line.label, text: line.text };
  }
  const object = value as Record<string, unknown>;
  if (!line.engine) {
    const fields = Object.entries(object).map(([key, v]) => ({ key, value: fieldValue(v) }));
    return { kind: "fields", title: line.label, fields };
  }
  if (object.type === "system" && object.subtype === "thinking_tokens") {
    // Only the count since the last line adds up over a group: the running total would be
    // counted again with each line, so a line without the former counts for nothing.
    const delta = object.estimated_tokens_delta;
    return { kind: "thinking", tokens: typeof delta === "number" && delta > 0 ? delta : 0 };
  }
  // The type and the subtype go into the title when they are words; one that is not stays
  // among the fields, so that nothing of the line is lost.
  const named = (["type", "subtype"] as const).filter((key) => {
    const part = object[key];
    return typeof part === "string" && part.trim() !== "";
  });
  const kind = named.map((key) => words(object[key] as string)).join(" · ");
  const titled: ReadonlySet<string> = new Set(named);
  return {
    kind: "fields",
    title: kind === "" ? line.label : kind.charAt(0).toUpperCase() + kind.slice(1),
    fields: Object.entries(object)
      .filter(([key]) => !ENGINE_IDS.has(key) && !titled.has(key))
      .map(([key, v]) => ({ key: words(key), value: fieldValue(v) })),
  };
}

/// The most bars the thinking of a group draws.
const MAX_BARS = 24;

/// The thinking counts of a group as the heights of its bars: one per count, or, past
/// `MAX_BARS` counts, neighbours added up so that the bars stay as many.
export function thinkingBars(counts: number[]): number[] {
  if (counts.length <= MAX_BARS) return counts;
  const bars = new Array<number>(MAX_BARS).fill(0);
  counts.forEach((count, i) => {
    bars[Math.floor((i * MAX_BARS) / counts.length)]! += count;
  });
  return bars;
}

/// A token count as a person reads it: "383", "1.2k".
export function tokenCount(tokens: number): string {
  return tokens < 1000 ? String(tokens) : `${(tokens / 1000).toFixed(1).replace(/\.0$/, "")}k`;
}

/// The one line a group of consecutive raw lines shows as: the thinking with its tokens, then
/// the one other line by its title, or how many there are.
export function rawSummary(views: RawView[]): string {
  const parts: string[] = [];
  const thinking = views.filter((view) => view.kind === "thinking");
  if (thinking.length > 0) {
    const tokens = thinking.reduce((sum, view) => sum + view.tokens, 0);
    parts.push(
      tokens > 0 ? `Thought it through · about ${tokenCount(tokens)} tokens` : "Thought it through",
    );
  }
  const rest = views.filter((view) => view.kind !== "thinking");
  if (rest.length === 1) parts.push(rest[0]!.title);
  else if (rest.length > 1) {
    const same = rest.every((view) => view.title === rest[0]!.title);
    parts.push(same ? `${rest[0]!.title} × ${rest.length}` : `${rest.length} raw lines`);
  }
  return parts.join(" · ");
}

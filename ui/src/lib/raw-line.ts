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

/// The fields of an engine line that say nothing to a person: its kind is in the title, and the
/// ids name the session and the line.
const ENGINE_NOISE = new Set(["type", "subtype", "session_id", "uuid"]);

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
    const { estimated_tokens_delta: delta, estimated_tokens: total } = object;
    return {
      kind: "thinking",
      tokens: typeof delta === "number" ? delta : typeof total === "number" ? total : 0,
    };
  }
  const kind = [object.type, object.subtype]
    .filter((part): part is string => typeof part === "string" && part.trim() !== "")
    .map(words)
    .join(" · ");
  return {
    kind: "fields",
    title: kind === "" ? line.label : kind.charAt(0).toUpperCase() + kind.slice(1),
    fields: Object.entries(object)
      .filter(([key]) => !ENGINE_NOISE.has(key))
      .map(([key, v]) => ({ key: words(key), value: fieldValue(v) })),
  };
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
    parts.push(same ? `${rest[0]!.title} × ${rest.length}` : `${rest.length} engine lines`);
  }
  return parts.join(" · ");
}

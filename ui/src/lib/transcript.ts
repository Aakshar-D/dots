// Turns a run's stored events into what the transcript shows.
import { RunEventData, type RunEventRecord } from "@/api/types";
import { BROWSER_PREFIX } from "@/lib/browser";
import { mcpToolLabel } from "@/lib/mcp-tool";

export type TranscriptItem =
  | { type: "text"; seq: number; text: string }
  | {
      type: "tool";
      seq: number;
      tool: string;
      input: unknown;
      /// `null` until the result arrives.
      result: { output: string; isError: boolean } | null;
    }
  /// A call of the built-in browser (`mcp__browser__*`), shown as one line (browser spec
  /// section 7).
  | {
      type: "browser";
      seq: number;
      tool: string;
      input: unknown;
      /// `null` until the result arrives.
      result: { output: string; isError: boolean } | null;
    }
  /// A call of a check-in tool (`notify`, `ask`, `follow_up`), shown as one line.
  | {
      type: "checkin";
      seq: number;
      kind: CheckInKind;
      input: unknown;
      /// `null` until the result arrives.
      result: { output: string; isError: boolean } | null;
    }
  | { type: "error"; seq: number; text: string }
  /// What Dots says about the run itself: an item left out, a server held.
  | { type: "note"; seq: number; text: string }
  /// Something the transcript does not render as conversation; shown collapsed.
  | { type: "raw"; seq: number; label: string; text: string };

export type CheckInKind = "notify" | "ask" | "follow_up";

/// The check-in tools by the names their calls are stored under: the policy name on the local
/// engine, Dots' own MCP server's tool on the Claude engine.
const CHECK_INS = new Map<string, CheckInKind>([
  ["Notify", "notify"],
  ["mcp__dots__notify", "notify"],
  ["Ask", "ask"],
  ["mcp__dots__ask", "ask"],
  ["FollowUp", "follow_up"],
  ["mcp__dots__follow_up", "follow_up"],
]);

/// The trimmed string argument `key` of a call, or "".
function field(input: unknown, key: string): string {
  const value =
    typeof input === "object" && input !== null
      ? (input as Record<string, unknown>)[key]
      : undefined;
  return typeof value === "string" ? value.trim() : "";
}

/// The line a check-in call shows as (check-ins spec section 9). A follow-up names its time
/// once the core's answer has it: the core writes it in local time.
export function checkInLine(
  kind: CheckInKind,
  input: unknown,
  result: { output: string; isError: boolean } | null,
): string {
  switch (kind) {
    case "notify":
      return `Sent a note: ${field(input, "text")}`;
    case "ask":
      return `Asked: ${field(input, "question")}`;
    case "follow_up": {
      const when =
        result && !result.isError ? /scheduled for (.+)\.$/.exec(result.output)?.[1] : undefined;
      const note = field(input, "note");
      return when ? `Scheduled a follow-up for ${when}: ${note}` : `Scheduled a follow-up: ${note}`;
    }
  }
}

/// One line that says what a tool call is about.
export function toolPreview(input: unknown): string {
  if (typeof input === "object" && input !== null) {
    for (const key of ["command", "file_path", "path", "pattern", "url"]) {
      const value = (input as Record<string, unknown>)[key];
      if (typeof value === "string") return value;
    }
  }
  const text = JSON.stringify(input) ?? "";
  return text.length > 80 ? `${text.slice(0, 80)}…` : text;
}

/// Assistant text, tool calls with their results paired by id, failures, and raw lines.
/// Bookkeeping events (session, usage, the local engine's `message` records, the final
/// summary) are left out. An event from a newer core that does not parse is kept as raw.
export function transcriptItems(events: RunEventRecord[]): TranscriptItem[] {
  const items: TranscriptItem[] = [];
  const calls = new Map<
    string,
    Extract<TranscriptItem, { type: "tool" | "checkin" | "browser" }>
  >();
  for (const record of events) {
    const parsed = RunEventData.safeParse(record.data);
    if (!parsed.success) {
      items.push({
        type: "raw",
        seq: record.seq,
        label: `Unknown event (${record.kind})`,
        text: JSON.stringify(record.data, null, 2) ?? "",
      });
      continue;
    }
    const event = parsed.data;
    switch (event.kind) {
      case "assistant_text":
        items.push({ type: "text", seq: record.seq, text: event.text });
        break;
      case "tool_call": {
        const kind = CHECK_INS.get(event.tool);
        const call: Extract<TranscriptItem, { type: "tool" | "checkin" | "browser" }> =
          kind !== undefined
            ? { type: "checkin", seq: record.seq, kind, input: event.input, result: null }
            : event.tool.startsWith(BROWSER_PREFIX)
              ? { type: "browser", seq: record.seq, tool: event.tool, input: event.input, result: null }
              : { type: "tool", seq: record.seq, tool: event.tool, input: event.input, result: null };
        calls.set(event.id, call);
        items.push(call);
        break;
      }
      case "tool_result": {
        const result = { output: event.output, isError: event.is_error };
        const call = calls.get(event.id);
        if (call) call.result = result;
        else items.push({ type: "raw", seq: record.seq, label: "Tool result", text: event.output });
        break;
      }
      case "failed":
        items.push({ type: "error", seq: record.seq, text: event.error });
        break;
      case "raw":
        items.push({ type: "raw", seq: record.seq, label: "Unparsed output", text: event.line });
        break;
      case "notice":
        items.push({ type: "note", seq: record.seq, text: event.text });
        break;
      case "session_started":
      case "usage":
      case "message":
      case "finished":
        break;
    }
  }
  return items;
}

/// The memory tool a call ran: Dots' own server names it, and the local engine stores the three
/// calls under their policy name `Memory`, so its input tells them apart, as the core does
/// (`memory::Tool::for_input`). Null for any other tool.
export function memoryTool(tool: string, input: unknown): "remember" | "forget" | "recall" | null {
  const has = (key: string) => typeof input === "object" && input !== null && key in input;
  if (tool === "mcp__dots__remember" || (tool === "Memory" && has("text"))) return "remember";
  if (tool === "mcp__dots__forget" || (tool === "Memory" && has("id"))) return "forget";
  if (tool === "mcp__dots__recall" || tool === "Memory") return "recall";
  return null;
}

/// The name a person reads for a call of `tool`: a local memory call (policy name `Memory`) as
/// `Memory › remember`, `forget` or `recall`; any other tool as it is.
export function callLabel(tool: string, input: unknown): string {
  return tool === "Memory" ? `Memory › ${memoryTool(tool, input)}` : tool;
}

/// What a call of a memory tool did, for the line under its chip (dot memory spec section 5):
/// `remember` the text it saved, `forget` that it forgot one. Nothing for `recall`, for another
/// tool, or for a call that has no result yet, failed or waits for approval.
export function memoryNote(
  tool: string,
  input: unknown,
  result: { output: string; isError: boolean } | null,
): { kind: "remembered"; text: string } | { kind: "forgot" } | null {
  const which = memoryTool(tool, input);
  if (which === null || which === "recall" || result === null || result.isError) return null;
  return which === "remember"
    ? { kind: "remembered", text: field(input, "text") }
    : { kind: "forgot" };
}

/// How a group of tool steps reads, for one step and for `n` of them.
type StepPhrase = { one: string; many: (n: number) => string };

const READ_FILE: StepPhrase = { one: "Read a file", many: (n) => `Read ${n} files` };
const EDIT_FILE: StepPhrase = { one: "Edited a file", many: (n) => `Edited ${n} files` };
const COMMAND: StepPhrase = { one: "Ran a command", many: (n) => `Ran ${n} commands` };
const SEARCH_FILES: StepPhrase = {
  one: "Searched the files",
  many: (n) => `Searched the files ${n} times`,
};
const WEB_PAGE: StepPhrase = { one: "Read a web page", many: (n) => `Read ${n} web pages` };
const WEB_SEARCH: StepPhrase = {
  one: "Searched the web",
  many: (n) => `Searched the web ${n} times`,
};

/// The tools a phrase of its own says, by the names both engines store their calls under: the
/// Claude engine's built-in tools and the local engine's.
const STEP_PHRASES = new Map<string, StepPhrase>([
  ["Read", READ_FILE],
  ["read_file", READ_FILE],
  ["Write", EDIT_FILE],
  ["Edit", EDIT_FILE],
  ["MultiEdit", EDIT_FILE],
  ["NotebookEdit", EDIT_FILE],
  ["write_file", EDIT_FILE],
  ["edit_file", EDIT_FILE],
  ["Bash", COMMAND],
  ["shell", COMMAND],
  ["Glob", SEARCH_FILES],
  ["Grep", SEARCH_FILES],
  ["LS", SEARCH_FILES],
  ["glob", SEARCH_FILES],
  ["grep", SEARCH_FILES],
  ["list_dir", SEARCH_FILES],
  ["WebFetch", WEB_PAGE],
  ["WebSearch", WEB_SEARCH],
]);

/// The phrase of one step: its tool's own, else "Used" and the name its chip shows.
function stepPhrase(
  tool: string,
  input: unknown,
  owners: ReadonlyMap<string, string>,
): StepPhrase {
  const known = STEP_PHRASES.get(tool);
  if (known) return known;
  const label = mcpToolLabel(callLabel(tool, input), owners);
  return { one: `Used ${label}`, many: (n) => `Used ${label} ${n} times` };
}

/// The one line a group of consecutive tool steps shows as in the conversation (conversational
/// chat spec section 5): the steps counted by phrase, in the order each phrase first comes, as
/// "Read 2 files · Ran a command". `owners`: MCP server name to the plugin it comes with.
export function stepsLine(
  items: { tool: string; input: unknown }[],
  owners: ReadonlyMap<string, string>,
): string {
  // A phrase's one-step words tell it apart: two tools that share a phrase share them.
  const counts = new Map<string, { phrase: StepPhrase; n: number }>();
  for (const item of items) {
    const phrase = stepPhrase(item.tool, item.input, owners);
    const seen = counts.get(phrase.one);
    if (seen) seen.n += 1;
    else counts.set(phrase.one, { phrase, n: 1 });
  }
  return [...counts.values()]
    .map(({ phrase, n }) => (n === 1 ? phrase.one : phrase.many(n)))
    .join(" · ");
}

// Turns a run's stored events into what the transcript shows.
import { RunEventData, type RunEventRecord } from "@/api/types";

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
  | { type: "error"; seq: number; text: string }
  /// Something the transcript does not render as conversation; shown collapsed.
  | { type: "raw"; seq: number; label: string; text: string };

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
  const calls = new Map<string, Extract<TranscriptItem, { type: "tool" }>>();
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
        const call = {
          type: "tool" as const,
          seq: record.seq,
          tool: event.tool,
          input: event.input,
          result: null,
        };
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
      case "session_started":
      case "usage":
      case "message":
      case "finished":
        break;
    }
  }
  return items;
}

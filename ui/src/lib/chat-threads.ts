import type { Run } from "@/api/types";

/// A chat conversation with a dot: the lineage that starts at a chat run.
export type ChatThread = {
  /// The id of the lineage's first run; it names the thread in the address.
  rootId: string;
  /// The first message, cut to one line.
  title: string;
  /// The lineage's newest run: the thread's state, and what a follow-up continues from.
  latest: Run;
};

function firstLine(run: Run): string {
  const message =
    typeof run.payload === "object" && run.payload !== null
      ? (run.payload as Record<string, unknown>).message
      : null;
  const line = typeof message === "string" ? message.trim().split("\n")[0] : "";
  if (line === "") return "Chat";
  return line.length > 60 ? `${line.slice(0, 60)}…` : line;
}

/// The chat threads among a dot's runs, the most recently active first. Lineages that a
/// schedule, a webhook or "Run now" started are not chats.
export function chatThreads(runs: Run[]): ChatThread[] {
  const threads = new Map<string, ChatThread>();
  const byAge = [...runs].sort((a, b) => a.queued_at.localeCompare(b.queued_at));
  for (const run of byAge) {
    if (run.parent_run_id === null && run.trigger === "chat") {
      threads.set(run.id, { rootId: run.id, title: firstLine(run), latest: run });
    }
  }
  for (const run of byAge) {
    const thread = threads.get(run.root_run_id);
    if (thread) thread.latest = run;
  }
  return [...threads.values()].sort((a, b) =>
    b.latest.queued_at.localeCompare(a.latest.queued_at),
  );
}

/// Whether the run has not ended: it waits to start, runs, or waits for an approval. The one
/// definition for every screen (the core's is `RunStatus::is_terminal`).
export function isActive(run: Run): boolean {
  return run.status === "queued" || run.status === "running" || run.status === "awaiting_approval";
}

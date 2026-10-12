import { QueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { RunEventRecord } from "@/api/types";

/// Data stays fresh until a runtime event or a mutation marks it stale (lib/runtime-events.ts):
/// the backend tells the UI when something changed, so nothing polls.
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

export const keys = {
  startupError: ["startup-error"] as const,
  status: ["status"] as const,
  settings: ["settings"] as const,
  overview: ["overview"] as const,
  inbox: ["inbox"] as const,
  dot: (id: string) => ["dot", id] as const,
  runs: (dotId: string | null) => ["runs", dotId] as const,
  run: (id: string) => ["run", id] as const,
  thread: (runId: string) => ["thread", runId] as const,
  events: (runId: string) => ["events", runId] as const,
  approvals: (runId: string) => ["approvals", runId] as const,
  diff: (runId: string) => ["diff", runId] as const,
};

/// `have` plus the events of `more` it does not hold yet, ordered by `seq`.
export function mergeEvents(have: RunEventRecord[], more: RunEventRecord[]): RunEventRecord[] {
  const seen = new Set(have.map((e) => e.seq));
  const fresh = more.filter((e) => !seen.has(e.seq));
  if (fresh.length === 0) return have;
  return [...have, ...fresh].sort((a, b) => a.seq - b.seq);
}

/// A run's events. A refetch asks only for what is missing: the events after the last one
/// already loaded.
export function eventsQuery(qc: QueryClient, runId: string) {
  const queryKey = keys.events(runId);
  return {
    queryKey,
    queryFn: async () => {
      const have = qc.getQueryData<RunEventRecord[]>(queryKey) ?? [];
      const more = await commands.listRunEvents(runId, have.at(-1)?.seq ?? 0);
      return mergeEvents(have, more);
    },
  };
}

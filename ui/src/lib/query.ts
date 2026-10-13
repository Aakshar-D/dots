import { QueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { RunEventRecord } from "@/api/types";

/// Data stays fresh until a runtime event or a mutation marks it stale (lib/runtime-events.ts):
/// the backend tells the UI when something changed, so nothing polls.
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      // "always": every query and command is local IPC, so none waits for the network.
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnWindowFocus: false,
        networkMode: "always",
      },
      mutations: { retry: false, networkMode: "always" },
    },
  });
}

export const keys = {
  startupError: ["startup-error"] as const,
  status: ["status"] as const,
  settings: ["settings"] as const,
  overview: ["overview"] as const,
  inbox: ["inbox"] as const,
  dots: ["dots"] as const,
  dot: (id: string) => ["dot", id] as const,
  runs: (dotId: string | null) => ["runs", dotId] as const,
  run: (id: string) => ["run", id] as const,
  thread: (runId: string) => ["thread", runId] as const,
  events: (runId: string) => ["events", runId] as const,
  approvals: (runId: string) => ["approvals", runId] as const,
  diff: (runId: string) => ["diff", runId] as const,
};

/// Marks stale everything a change to a dot can show up in: the dot lists, every loaded dot,
/// the Inbox (a deleted dot takes its approvals and runs with it) and every list of runs. No
/// runtime event announces a dot change, so each command that changes a dot calls this.
export function invalidateDots(qc: QueryClient): void {
  for (const queryKey of [keys.dots, ["dot"], keys.overview, keys.inbox, ["runs"]]) {
    void qc.invalidateQueries({ queryKey });
  }
}

/// `have` plus the events of `more` it does not hold yet, ordered by `seq`.
export function mergeEvents(have: RunEventRecord[], more: RunEventRecord[]): RunEventRecord[] {
  const seen = new Set(have.map((e) => e.seq));
  const fresh = more.filter((e) => !seen.has(e.seq));
  if (fresh.length === 0) return have;
  return [...have, ...fresh].sort((a, b) => a.seq - b.seq);
}

/// A run's events. A refetch asks only for what is missing: the events after the last one
/// already loaded. The answer is merged into the list as it is when the answer arrives, so an
/// event a runtime event appended in the meantime stays.
export function eventsQuery(qc: QueryClient, runId: string) {
  const queryKey = keys.events(runId);
  return {
    queryKey,
    queryFn: async () => {
      const have = qc.getQueryData<RunEventRecord[]>(queryKey) ?? [];
      const more = await commands.listRunEvents(runId, have.at(-1)?.seq ?? 0);
      return mergeEvents(qc.getQueryData<RunEventRecord[]>(queryKey) ?? have, more);
    },
  };
}

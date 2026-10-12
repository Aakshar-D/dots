import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useNavigate } from "react-router";

import { onLagged, onNavigate, onRuntimeEvent, onStatus } from "@/api/events";
import type { RunEventRecord, RuntimeEvent } from "@/api/types";
import { keys, mergeEvents } from "@/lib/query";

/// Brings the query caches in step with one runtime event.
export function applyRuntimeEvent(qc: QueryClient, event: RuntimeEvent): void {
  switch (event.type) {
    case "run_updated":
      qc.setQueryData(keys.run(event.run.id), event.run);
      // Lists and views a run shows up in. The prefixes cover every dot's and thread's key.
      for (const queryKey of [["thread"], ["runs"], ["diff"], keys.overview, keys.inbox]) {
        void qc.invalidateQueries({ queryKey });
      }
      break;
    case "run_event": {
      const queryKey = keys.events(event.event.run_id);
      // Only a list that is loaded is extended; one that is not loads in full when asked for.
      if (qc.getQueryData(queryKey) !== undefined) {
        qc.setQueryData<RunEventRecord[]>(queryKey, (have = []) =>
          mergeEvents(have, [event.event]),
        );
      }
      break;
    }
    case "approval_requested":
    case "approval_decided":
      void qc.invalidateQueries({ queryKey: keys.inbox });
      void qc.invalidateQueries({ queryKey: keys.approvals(event.approval.run_id) });
      break;
  }
}

/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
/// stream refetches everything, the tray navigates, and a status changed elsewhere is taken.
export function useRuntimeEvents(): void {
  const qc = useQueryClient();
  const navigate = useNavigate();
  useEffect(() => {
    const refetch = () => void qc.invalidateQueries();
    const subscriptions = [
      onRuntimeEvent((event) => applyRuntimeEvent(qc, event), refetch),
      onLagged(refetch),
      onNavigate((path) => void navigate(path)),
      onStatus((status) => qc.setQueryData(keys.status, status)),
    ];
    return () => {
      for (const s of subscriptions) void s.then((unlisten) => unlisten());
    };
  }, [qc, navigate]);
}

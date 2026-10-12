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
      const have = qc.getQueryData<RunEventRecord[]>(queryKey);
      if (have === undefined) {
        // Not loaded. A first load in flight may have read the backend before this event was
        // stored, so the list is marked stale once that load has settled and then asks for
        // what is after its last event. Marking it stale during the load would be undone by
        // the load's answer. A fetch already running by then started after this event arrived,
        // so it is joined, not restarted: several events share one question. A list that is
        // not loading either is left alone: it loads in full when asked for.
        const load = qc.getQueryCache().find({ queryKey });
        if (load?.state.fetchStatus === "fetching") {
          const catchUp = () => void qc.invalidateQueries({ queryKey }, { cancelRefetch: false });
          void load.promise?.then(catchUp, catchUp);
        }
        break;
      }
      // `seq` counts up from 1 without gaps within a run.
      const last = have.at(-1)?.seq ?? 0;
      if (event.event.seq === last + 1) {
        qc.setQueryData<RunEventRecord[]>(queryKey, mergeEvents(have, [event.event]));
      } else if (event.event.seq > last + 1) {
        // An event in between is missing: ask for everything after the last one loaded.
        void qc.invalidateQueries({ queryKey });
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

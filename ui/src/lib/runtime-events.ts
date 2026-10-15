import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useNavigate } from "react-router";

import { onLagged, onNavigate, onRuntimeEvent, onStatus } from "@/api/events";
import type { RunEventRecord, RuntimeEvent } from "@/api/types";
import { invalidate, keys, mergeEvents, takeStatus } from "@/lib/query";

/// Brings the query caches in step with one runtime event.
export function applyRuntimeEvent(qc: QueryClient, event: RuntimeEvent): void {
  switch (event.type) {
    case "run_updated":
      // Lists and views a run shows up in. The prefixes cover every dot's and thread's key. The
      // run's approvals are among them: a run that ends takes its pending ones with it.
      for (const queryKey of [
        ["thread"],
        ["runs"],
        ["diff"],
        keys.overview,
        keys.inbox,
        keys.approvals(event.run.id),
      ]) {
        invalidate(qc, queryKey);
      }
      break;
    case "run_event": {
      const queryKey = keys.events(event.event.run_id);
      const have = qc.getQueryData<RunEventRecord[]>(queryKey);
      if (have === undefined) {
        // Not loaded. A first load in flight may have read the backend before this event was
        // stored, so the list is marked stale, which holds once that load has settled, and then
        // asks for what is after its last event. A list that is not loading either is left
        // alone: it loads in full when asked for.
        if (qc.getQueryState(queryKey)?.fetchStatus === "fetching") invalidate(qc, queryKey);
        break;
      }
      // `seq` counts up from 1 without gaps within a run.
      const last = have.at(-1)?.seq ?? 0;
      if (event.event.seq === last + 1) {
        qc.setQueryData<RunEventRecord[]>(queryKey, mergeEvents(have, [event.event]));
      } else if (event.event.seq > last + 1) {
        // An event in between is missing: ask for everything after the last one loaded.
        invalidate(qc, queryKey);
      }
      break;
    }
    case "approval_requested":
    case "approval_decided":
    // The approval's own run, which for a call Dots performs is the parent of the run at work.
    case "approval_updated":
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.approvals(event.approval.run_id));
      break;
    case "schedule_updated":
      // Where the dot's schedule stands: its next run, retry and "Needs attention", which the
      // Dots list and the Inbox show.
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
      break;
  }
}

/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
/// stream refetches everything, the tray navigates, and a status changed elsewhere is taken.
export function useRuntimeEvents(): void {
  const qc = useQueryClient();
  const navigate = useNavigate();
  useEffect(() => {
    let subscribed = true;
    const refetch = () => invalidate(qc);
    const subscriptions = [
      onRuntimeEvent((event) => applyRuntimeEvent(qc, event), refetch),
      onLagged(refetch),
      onNavigate((path) => void navigate(path)),
      onStatus((status) => {
        takeStatus(qc, status);
        // A pause or a resume moves every schedule's next run, which the overview holds.
        invalidate(qc, keys.overview);
      }),
    ];
    // The backend attaches a listener some time after it was asked to, and what it emits until
    // then is lost. Anything loaded so far may be older than such an event, so once every
    // listener is attached, everything is asked for again, one time.
    void Promise.all(subscriptions).then(() => {
      if (subscribed) refetch();
    });
    return () => {
      subscribed = false;
      for (const s of subscriptions) void s.then((unlisten) => unlisten());
    };
  }, [qc, navigate]);
}

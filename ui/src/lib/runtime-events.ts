import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useNavigate } from "react-router";

import { onLagged, onNavigate, onRuntimeEvent, onStatus, onUpdate } from "@/api/events";
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
      // A pending approval needs the person: the dots' counts.
      invalidate(qc, keys.overview);
      break;
    case "schedule_updated":
      // Where the dot's schedule stands: its next run, retry and "Needs attention", which the
      // Dots list and the Inbox show.
      for (const queryKey of [keys.dots, keys.overview, keys.inbox]) invalidate(qc, queryKey);
      break;
    case "library_updated":
      // The Library lists, the Inbox group of a changed tool list, and what shows the dots that
      // use the item: removing it takes it out of them, and no dot event says so. An app's row,
      // and the data a removed one kept, are on the Apps screen.
      for (const queryKey of [
        keys.library,
        keys.inbox,
        keys.dots,
        ["dot"],
        keys.overview,
        keys.apps,
      ]) {
        invalidate(qc, queryKey);
      }
      break;
    case "app_data_changed":
      // The app's open screen refreshes (`dots.onChange`).
      appDataChanges.dispatchEvent(new Event(event.app));
      break;
    case "org_updated":
      // The org list, and what lists a server per joined org: the Library's catalogs and the
      // Inbox's changed tool lists. What is read per org sits under the orgs' key.
      for (const queryKey of [keys.orgs, keys.library, keys.inbox]) invalidate(qc, queryKey);
      // Each dot's variables, read in its org. Only those: a dot loaded again would be one an
      // open editor might fill its form from.
      for (const query of qc.getQueryCache().findAll({ queryKey: ["dot"] })) {
        if (query.queryKey[2] === "variables") invalidate(qc, query.queryKey);
      }
      break;
    case "schedule_attention":
      // For the notification; the `schedule_updated` sent with it refreshes the screens.
      break;
    case "message_posted":
    case "message_updated":
      // The Inbox's Messages group and its count, and the chat the message belongs to. An
      // answer's run comes with its own `run_updated`.
      invalidate(qc, ["messages"]);
      // An open question needs the person, and a note is new: the dots' counts.
      invalidate(qc, keys.overview);
      break;
    case "dot_seen":
      // What is unread on the dot, and the Home link's total.
      invalidate(qc, keys.overview);
      break;
    case "follow_ups_updated":
      // The pending follow-ups in the Dots list and the dot's editor.
      invalidate(qc, ["follow-ups"]);
      break;
    case "memories_updated":
      // The dot's Memory tab.
      invalidate(qc, keys.memories(event.dot_id));
      break;
    case "instruction_proposal_changed":
      // The dot's proposal cards and the Inbox's group of them. Approving changes the dot,
      // which the card that decided reads again itself.
      invalidate(qc, keys.instructionProposals(event.dot_id));
      invalidate(qc, keys.inbox);
      break;
    case "watcher_updated":
      // The dot's Watchers tab, the Inbox's paused watchers and the dots' needs-you counts.
      invalidate(qc, keys.watchers(event.dot_id));
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.overview);
      break;
    case "watcher_paused":
      // The app's notification; a `watcher_updated` comes with it.
      break;
    case "app_proposal_changed":
      // A review card was posted or decided: the conversation's cards, the Inbox's reviews and
      // the dots' needs-you counts.
      invalidate(qc, ["app-proposals"]);
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.overview);
      break;
    case "limits_updated":
      // The rail's bars read the limits; the Inbox's held runs and the Usage page follow.
      qc.setQueryData(keys.limits, event.limits);
      invalidate(qc, keys.inbox);
      invalidate(qc, ["usage"]);
      break;
  }
}

/// Hears that an app's data changed: an event named by the app, for its open screen.
export const appDataChanges = new EventTarget();

/// Subscribes the app to the backend: runtime events patch the caches, a lagged or unreadable
/// stream refetches everything, the tray navigates, and a status or an update note changed
/// elsewhere is taken.
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
      onUpdate((note) => qc.setQueryData(keys.update, note)),
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

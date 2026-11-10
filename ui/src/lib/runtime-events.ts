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
      // The topic rows too: a run added to an archived topic brings it back.
      for (const queryKey of [
        ["thread"],
        ["runs"],
        ["topics"],
        ["diff"],
        keys.overview,
        keys.inbox,
        keys.approvals(event.run.id),
        keys.spend,
      ]) {
        invalidate(qc, queryKey);
      }
      // A run that works for a workflow step is linked from its workflow run's page. It names
      // its step run, not the workflow run, so every such page's links are marked.
      if (event.run.workflow_step_run_id !== null) invalidate(qc, ["workflow-dot-runs"]);
      break;
    case "run_quiet":
      // The quiet card of the run, and the dot's needs-you count. The run list comes with the
      // `run_updated` sent alongside.
      for (const queryKey of [["thread"], ["runs"], keys.overview]) {
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
    case "schedule_skipped":
      // The skip line in the conversation, and the header's count (the dots' schedule state).
      for (const queryKey of [keys.scheduleSkips(event.dot_id), keys.overview, keys.dots]) {
        invalidate(qc, queryKey);
      }
      break;
    case "topics_changed":
      // The dot's topic rows, and its runs and threads: a move or merge changes which topic a
      // run is in, and no `run_updated` says so.
      for (const queryKey of [keys.topics(event.dot_id), ["thread"], ["runs"]]) {
        invalidate(qc, queryKey);
      }
      break;
    case "library_updated":
      // The Library lists, the Inbox group of a changed tool list, and what shows the dots that
      // use the item: removing it takes it out of them, and no dot event says so. An app's row,
      // and the data a removed one kept, are on the Apps screen.
      for (const queryKey of [keys.library, keys.inbox, keys.dots, keys.overview, keys.apps]) {
        invalidate(qc, queryKey);
      }
      staleDots(qc);
      break;
    case "app_data_changed":
      // The app's open screen refreshes (`dots.onChange`).
      appDataChanges.dispatchEvent(new Event(event.app));
      break;
    case "org_updated":
      // The org list, and what lists a server per joined org: the Library's catalogs and the
      // Inbox's changed tool lists. What is read per org sits under the orgs' key.
      // A roster change can add or take away the workspaces a member sees, and with them the
      // scope in view (the core falls back to Default); a workspace that went away, or leaving
      // the account, moves dots to Default.
      for (const queryKey of [
        keys.orgs,
        keys.library,
        keys.inbox,
        ["workspaces"],
        // A role change or leaving the account changes which keys may be managed.
        ["apiKeys"],
        keys.scope,
        keys.dots,
        keys.overview,
      ]) {
        invalidate(qc, queryKey);
      }
      // What is read per dot under its key: its variables (read in its org) and its access
      // (from the member list).
      staleDots(qc);
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
    case "inbox_updated":
      // An item was snoozed, unsnoozed, marked done or undone, or came back: the Inbox, and
      // the counts under the overview (the dots' badges and the Home total).
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.overview);
      break;
    case "snooze_ended":
      // For the notification; the `inbox_updated` sent with it refreshes the screens.
      break;
    case "intents_updated":
      // The Intents screen of that workspace.
      invalidate(qc, keys.intents(event.org_id ?? "personal", event.workspace_id));
      break;
    case "graph_updated":
      // A build's progress refreshes the status lines; a build that ended, the graphs drawn.
      invalidate(qc, ["graph", "status"]);
      if (event.status.state !== "building") invalidate(qc, ["graph"]);
      break;
    case "scheduled_message_updated":
      // The dot's scheduled cards and its menu's count. Once sent, its card gives way to the
      // chat run it became: the runs and threads are read again too.
      invalidate(qc, keys.scheduledMessages(event.message.dot_id));
      if (event.message.status === "sent") {
        invalidate(qc, ["runs"]);
        invalidate(qc, ["thread"]);
      }
      break;
    case "limits_updated":
      // The rail's bars read the limits; the Inbox's held runs and the Usage page follow.
      qc.setQueryData(keys.limits, event.limits);
      invalidate(qc, keys.inbox);
      invalidate(qc, ["usage"]);
      break;
    case "team_updated":
      // The team lists and the team. Its members are what a builder's draft is checked
      // against, and the Inbox lists members outside the team's account or workspace.
      for (const queryKey of [keys.teams, ["workflow-problem", event.team_id], keys.inbox]) {
        invalidate(qc, queryKey);
      }
      break;
    case "workflow_updated":
      invalidate(qc, keys.workflows(event.team_id));
      invalidate(qc, keys.workflow(event.workflow_id));
      break;
    case "workflow_run_updated": {
      const { run } = event;
      // The run page with its check, diffs and dot runs, every list of workflow runs (the team
      // page's, a conversation's cards), and the workflow's runs in flight and last run, which no
      // workflow event announces. A waiting run is in the Inbox and needs the dot whose chat
      // started it.
      for (const queryKey of [
        keys.workflowRun(run.id),
        keys.workflowDotRuns(run.id),
        ["workflow-runs"],
        keys.workflows(run.team_id),
        keys.inbox,
        keys.overview,
      ]) {
        invalidate(qc, queryKey);
      }
      if (run.workflow_id !== null) invalidate(qc, keys.workflow(run.workflow_id));
      break;
    }
  }
}

/// Marks every loaded dot stale, and what is read per dot under its key. A dot an editor shows
/// is not loaded again now: the editor was filled from it once, and loading it again would put
/// the editor back to empty, losing what was typed. It is loaded when it is next opened. One
/// being loaded is asked again once that load settles, as `invalidate` does.
function staleDots(qc: QueryClient): void {
  for (const query of qc.getQueryCache().findAll({ queryKey: ["dot"] })) {
    if (query.queryKey.length > 2 || query.state.fetchStatus === "fetching") {
      invalidate(qc, query.queryKey);
    } else {
      void qc.invalidateQueries({ queryKey: query.queryKey, exact: true, refetchType: "none" });
    }
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

import type {
  HeldRun,
  Inbox,
  InboxKey,
  Message,
  NeedsAttention,
  PausedWatcher,
  PendingAppReview,
  PendingApproval,
  PendingInstructions,
  PendingMakeRequest,
  ReviewRun,
  TeamScopeIssue,
  ToolsChanged,
  UndeliveredMessage,
} from "@/api/types";
import { waits } from "@/lib/messages";

/// The key of each kind of Inbox item, as the core makes it (snooze and done spec section 4.1;
/// `inbox::entries` in the core). The contract fixture's `InboxKeys` checks them.
export const messageKey = (m: Message): InboxKey => ({ kind: "message", id: m.id });
export const approvalKey = (a: PendingApproval): InboxKey => ({ kind: "approval", id: a.id });
export const reviewKey = (r: ReviewRun): InboxKey => ({ kind: "review", id: r.id });
export const notDeliveredKey = (r: UndeliveredMessage): InboxKey => ({
  kind: "not_delivered",
  id: r.id,
});
export const attentionKey = (a: NeedsAttention): InboxKey => ({ kind: "attention", id: a.run_id });
export const watcherKey = (w: PausedWatcher): InboxKey => ({
  kind: "watcher",
  id: `${w.watcher_id}@${w.at}`,
});
export const heldKey = (h: HeldRun): InboxKey => ({ kind: "held", id: h.run_id });
export const toolsKey = (t: ToolsChanged): InboxKey => ({
  kind: "tools",
  id: `${t.server_id}/${t.scope}`,
});
export const instructionsKey = (i: PendingInstructions): InboxKey => ({
  kind: "instructions",
  id: i.proposal.id,
});
export const appReviewKey = (r: PendingAppReview): InboxKey => ({
  kind: "app_review",
  id: r.proposal.id,
});
export const teamScopeKey = (t: TeamScopeIssue): InboxKey => ({
  kind: "team_scope",
  id: `${t.team_id}/${t.dot_id}`,
});
export const makeRequestKey = (r: PendingMakeRequest): InboxKey => ({
  kind: "make_request",
  id: r.id,
});

/// A key as one string, for sets and React keys.
export function keyString(k: InboxKey): string {
  return `${k.kind}:${k.id}`;
}

/// Every item of the Inbox and the waiting messages, in the core's order: messages, approvals,
/// not delivered, runs to review, attention, tools changed, watchers, held runs, instructions,
/// apps to review, team scope problems, make requests. Only the tests call it: it is the contract check's
/// entry point (spec section 8), which compares these keys with the core's `InboxKeys`, so the
/// per-kind key functions the screen uses cannot drift from the core. Not dead code.
export function inboxKeys(inbox: Inbox, messages: Message[]): InboxKey[] {
  return [
    ...messages.filter(waits).map(messageKey),
    ...inbox.approvals.map(approvalKey),
    ...inbox.not_delivered.map(notDeliveredKey),
    ...inbox.review_runs.map(reviewKey),
    ...inbox.attention.map(attentionKey),
    ...inbox.tools_changed.map(toolsKey),
    ...inbox.watchers.map(watcherKey),
    ...inbox.held.map(heldKey),
    ...inbox.instructions.map(instructionsKey),
    ...inbox.app_reviews.map(appReviewKey),
    ...inbox.team_scope.map(teamScopeKey),
    ...(inbox.make_requests ?? []).map(makeRequestKey),
  ];
}

/// The items the person put away: snoozed or done. The core leaves them out of the sections
/// already; the messages come apart from the Inbox, so this is what leaves them out.
export function hiddenKeys(inbox: Inbox): Set<string> {
  return new Set([
    ...inbox.snoozed.map(keyString),
    ...inbox.done.map(keyString),
    // A message done more than 7 days ago is no longer in `done`, and stays hidden.
    ...inbox.hidden_messages.map((id) => keyString({ kind: "message", id })),
  ]);
}

/// The items back from snooze, shown first with their tag.
export function backKeys(inbox: Inbox): Set<string> {
  return new Set(inbox.back.map(keyString));
}

/// The messages the Inbox shows: those that wait, but not the hidden ones.
export function shownMessages(inbox: Inbox | undefined, messages: Message[]): Message[] {
  const hidden = inbox ? hiddenKeys(inbox) : new Set<string>();
  return messages.filter((m) => waits(m) && !hidden.has(keyString(messageKey(m))));
}

/// What waits in the Inbox, as the core's `inbox_count` counts it: each item of each section,
/// the held runs as one line however many, and the messages that wait and are not hidden.
export function inboxWaiting(inbox: Inbox | undefined, messages: Message[] | undefined): number {
  const sections = inbox
    ? inbox.approvals.length +
      inbox.not_delivered.length +
      inbox.review_runs.length +
      inbox.attention.length +
      inbox.tools_changed.length +
      inbox.watchers.length +
      Math.min(inbox.held.length, 1) +
      inbox.instructions.length +
      inbox.app_reviews.length +
      inbox.team_scope.length +
      // A dot asking to make, change or give a job to a dot beyond its reach.
      (inbox.make_requests?.length ?? 0) +
      // A workflow run waiting on the person: a check, a limit or a failed step.
      inbox.workflows.length
    : 0;
  return sections + shownMessages(inbox, messages ?? []).length;
}

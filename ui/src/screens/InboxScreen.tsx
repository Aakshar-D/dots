import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { Inbox as InboxIcon, Play } from "lucide-react";
import { useCallback, useEffect, useRef, type ReactNode } from "react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type {
  HeldRun,
  Inbox,
  InboxKey,
  Message,
  NeedsAttention,
  PausedWatcher,
  PendingAppReview,
  PendingInstructions,
  ReviewRun,
  TeamScopeIssue,
  ToolsChanged,
  UndeliveredMessage,
} from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { InboxActed, InboxActions } from "@/components/InboxActions";
import { InstructionsCard } from "@/components/InstructionsCard";
import { MessageCard } from "@/components/MessageCard";
import { Page } from "@/components/Page";
import { ReviewActions } from "@/components/ReviewActions";
import { CapActions } from "@/components/SpendCaps";
import { StatusBadge } from "@/components/StatusBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { byRun, type RunApprovals } from "@/lib/approvals";
import { relativeTime } from "@/lib/format";
import { triggerLabel } from "@/lib/home-topics";
import {
  appReviewKey,
  approvalKey,
  attentionKey,
  backKeys,
  heldKey,
  instructionsKey,
  keyString,
  messageKey,
  notDeliveredKey,
  reviewKey,
  shownMessages,
  teamScopeKey,
  toolsKey,
  watcherKey,
} from "@/lib/inbox";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { toolPreview } from "@/lib/transcript";
import { releasable } from "@/components/RunNumbers";
import { resetTime } from "@/lib/usage-format";
import { MarkedList } from "@/screens/inbox/MarkedList";
import { NotDeliveredRow } from "@/screens/inbox/NotDeliveredRow";
import { WaitingRunRow } from "@/screens/teams/WaitingRuns";

/// A finished run that waits for the user: it failed, or it left a worktree to look at.
function ReviewRow({ run, actions }: { run: ReviewRun; actions?: ReactNode }) {
  return (
    <li
      aria-label={`${run.dot_name} run to review`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <StatusBadge status={run.status} />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">{run.dot_name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {relativeTime(run.ended_at ?? run.queued_at)}
          {(run.error ?? run.summary) && ` · ${run.error ?? run.summary}`}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/runs/${run.id}`}>Open</Link>
      </Button>
      <ReviewActions
        runId={run.id}
        branch={run.branch}
        canReview
        canDiscard={run.worktree_exists}
      />
      {actions}
    </li>
  );
}

/// An app a dot sent for review (app library spec section 10). The card, with its preview, waits
/// in the dot's conversation; the row opens it there.
function AppReviewRow({ item, actions }: { item: PendingAppReview; actions?: ReactNode }) {
  const { proposal } = item;
  const { title, update_from: from } = proposal.summary;
  const what = from === null ? `the app ${title}` : `a change to ${title}`;
  return (
    <li
      aria-label={`${title} to review`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">
          {item.dot_name} sent {what} for review
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {proposal.note} · {relativeTime(proposal.created_at)}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/home/${item.dot_id}/${proposal.root_run_id}`}>Open</Link>
      </Button>
      {actions}
    </li>
  );
}

/// A watcher Dots paused (watchers spec section 7): why, since when, Resume, and its dot's tab.
function PausedWatcherRow({ item, actions }: { item: PausedWatcher; actions?: ReactNode }) {
  const qc = useQueryClient();
  const resume = useMutation({
    mutationFn: () => commands.watcherResume(item.watcher_id),
    onError: notifyError,
    onSettled: () => {
      for (const queryKey of [keys.inbox, keys.overview, keys.watchers(item.dot_id)]) {
        invalidate(qc, queryKey);
      }
    },
  });
  const why =
    item.reason === "flood" ? "paused: too many runs" : `paused: ${item.error ?? "an error"}`;
  return (
    <li aria-label={`${item.name} paused`} className="flex items-center gap-3 rounded-lg border p-3">
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">{item.name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {item.dot_name} · {why} · {relativeTime(item.at)}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/dots/${item.dot_id}?tab=watchers`}>Open the dot's watchers</Link>
      </Button>
      <Button
        size="sm"
        disabled={resume.isPending}
        onClick={onSingleClick(() => resume.mutate())}
      >
        Resume
      </Button>
      {actions}
    </li>
  );
}

/// A background run held for Claude usage (usage and limits spec section 5): when it starts,
/// and **Run anyway**. One held for a role says why and has no button: it goes when the role
/// returns (members spec section 6.3). One held by a spending cap names the cap, with **Raise
/// cap** and, for the person's own runs, **Run once anyway** (spend caps design).
function HeldRunRow({ item, actions }: { item: HeldRun; actions?: ReactNode }) {
  if (item.reason.kind === "spend_cap") {
    return (
      <li
        aria-label={`${item.dot_name} run held`}
        className="flex items-center gap-3 rounded-lg border p-3"
      >
        <Link to={`/runs/${item.run_id}`} className="min-w-0 flex-1 truncate hover:underline">
          {item.dot_name} · {triggerLabel(item)} · {item.reason.text}
        </Link>
        <CapActions
          runId={item.run_id}
          dotId={item.dot_id}
          trigger={item.trigger}
          text={item.reason.text}
        />
        {actions}
      </li>
    );
  }
  return <UsageHeldRow item={item} actions={actions} />;
}

function UsageHeldRow({ item, actions }: { item: HeldRun; actions?: ReactNode }) {
  const qc = useQueryClient();
  const limits = useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data;
  const release = useMutation({
    mutationFn: () => commands.runReleaseHold(item.run_id),
    onError: notifyError,
    onSettled: () => {
      for (const queryKey of [keys.inbox, keys.limits]) invalidate(qc, queryKey);
    },
  });
  const { reason } = item;
  const role = reason.kind === "role";
  const when = role
    ? reason.text
    : item.starts_after
      ? `starts after ${resetTime(item.starts_after)}`
      : "waiting for the first run after the reset";
  return (
    <li
      aria-label={`${item.dot_name} run held`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <Link to={`/runs/${item.run_id}`} className="min-w-0 flex-1 truncate hover:underline">
        {item.dot_name} · {triggerLabel(item)} · {when}
      </Link>
      {!role && releasable(item.trigger, limits) && (
        <Button
          size="sm"
          variant="outline"
          disabled={release.isPending}
          onClick={onSingleClick(() => release.mutate())}
        >
          Run anyway
        </Button>
      )}
      {actions}
    </li>
  );
}

/// A schedule whose last occurrence failed in the end: its retries were used up, or its run
/// had called a tool. The schedule keeps firing; the flag goes when a later run of the dot
/// succeeds or when it is dismissed here.
function AttentionRow({ item, actions }: { item: NeedsAttention; actions?: ReactNode }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  // The flag shows in the Inbox and in the Dots list. The core also announces the change
  // (`ScheduleUpdated`); this reload does not wait for it.
  const reload = () => {
    for (const queryKey of [keys.inbox, keys.overview]) invalidate(qc, queryKey);
  };
  const runNow = useMutation({
    mutationFn: () => commands.runNow(item.dot_id),
    onSuccess: (run) => void navigate(`/runs/${run.id}`),
    onError: notifyError,
    onSettled: reload,
  });
  const dismiss = useMutation({
    mutationFn: () => commands.dismissScheduleAttention(item.dot_id),
    onError: notifyError,
    onSettled: reload,
  });
  return (
    <li
      aria-label={`${item.dot_name} needs attention`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <Link to={`/dots/${item.dot_id}`} className="font-medium hover:underline">
          {item.dot_name}
        </Link>
        <span className="truncate text-xs text-muted-foreground">
          Scheduled run failed {relativeTime(item.failed_at)}
          {item.error && ` · ${item.error}`}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={`/runs/${item.run_id}`}>Open the failed run</Link>
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={runNow.isPending}
        onClick={onSingleClick(() => runNow.mutate())}
      >
        <Play aria-hidden /> Run now
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={dismiss.isPending}
        onClick={onSingleClick(() => dismiss.mutate())}
      >
        Dismiss
      </Button>
      {actions}
    </li>
  );
}

/// An MCP server whose tool list changed since it was accepted in one scope, Personal or an
/// org's: its tools ask there until a review. An org's row names the org, and its review
/// accepts the org's list.
function ToolsChangedRow({ item, actions }: { item: ToolsChanged; actions?: ReactNode }) {
  const inOrg = item.scope !== "personal";
  const orgs = useQuery({ queryKey: keys.orgs, queryFn: commands.orgList, enabled: inOrg });
  const orgName = orgs.data?.find((org) => org.id === item.scope)?.name ?? item.scope;
  const name = inOrg ? `${item.name} (${orgName})` : item.name;
  const review =
    `/library?review=${encodeURIComponent(item.server_id)}` +
    (inOrg ? `&scope=${encodeURIComponent(item.scope)}` : "");
  return (
    <li
      aria-label={`${name} tools changed`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="font-medium">{name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {item.dots.length === 0
            ? "No dot uses it yet."
            : `Used by ${item.dots.map((dot) => dot.name).join(", ")}`}
        </span>
      </div>
      <Button asChild size="sm" variant="outline">
        <Link to={review}>Review</Link>
      </Button>
      {actions}
    </li>
  );
}

/// "Approve all" and "Deny all" for the calls of one run that the Inbox lists now. Mounted per
/// set of calls (its key): one answer per set, and a new set asked after a batch can be
/// answered again.
function RunBatch({ group }: { group: RunApprovals }) {
  const qc = useQueryClient();
  const decideAll = useMutation({
    // Only the calls listed here: the run may also wait for a call the person snoozed.
    mutationFn: (approved: boolean) =>
      commands.decideRunApprovals(
        group.runId,
        group.approvals.map((approval) => approval.id),
        approved,
        null,
      ),
    onSuccess: () => {
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.approvals(group.runId));
    },
    onError: notifyError,
  });
  // One answer per set: after a success the buttons stay off until the Inbox has reloaded.
  const decided = decideAll.isPending || decideAll.isSuccess;
  // A call the guardrails keep for an Admin can't be approved from this PC, so neither can the
  // set (the core refuses it too); Deny all stays. Each card says which call it is.
  const guardrails = useQueries({
    queries: group.approvals.map((approval) => ({
      queryKey: keys.approvalGuardrail(approval.id),
      queryFn: () => commands.approvalGuardrail(approval.id),
      retry: false,
    })),
  });
  const needsAdmin = guardrails.find((q) => q.data?.needs_admin)?.data?.text;
  const count = group.approvals.length;
  const calls = group.approvals.map((approval) => (
    <span key={approval.id} className="block font-mono text-xs">
      {approval.tool} {toolPreview(approval.input)}
    </span>
  ));
  return (
    <>
      <ConfirmButton
        title={`Approve all ${count} calls?`}
        description={
          <>
            {group.dotName} may perform these calls, in the order it asked:
            {calls}
          </>
        }
        confirmLabel="Approve all"
        disabled={decided || needsAdmin !== undefined}
        reason={needsAdmin}
        onConfirm={() => decideAll.mutate(true)}
      >
        Approve all ({count})
      </ConfirmButton>
      {needsAdmin && <Badge variant="outline">Needs an Admin</Badge>}
      <ConfirmButton
        title={`Deny all ${count} calls?`}
        description={
          <>
            {group.dotName} may perform none of these calls, and gets no note:
            {calls}
          </>
        }
        confirmLabel="Deny all"
        disabled={decided}
        onConfirm={() => decideAll.mutate(false)}
      >
        Deny all ({count})
      </ConfirmButton>
    </>
  );
}

/// The tool calls one run waits for. Two or more can be answered together: the core decides
/// them in the order they were asked and resumes the run once. A rule is saved only with a
/// single approval, on its card.
function RunApprovalsGroup({ group, actions }: { group: RunApprovals; actions?: ReactNode }) {
  // The batch, not the group, is keyed by the calls: the cards keep what was typed into them
  // when the run asks for another call.
  const batchKey = group.approvals.map((approval) => approval.id).join(" ");
  return (
    <div
      role="group"
      aria-label={`Approvals asked by ${group.dotName}`}
      className="flex flex-col gap-2"
    >
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-xs text-muted-foreground">
          <Link to={`/dots/${group.dotId}`} className="text-foreground hover:underline">
            {group.dotName}
          </Link>{" "}
          asked {relativeTime(group.approvals[0].created_at)} ·{" "}
          <Link to={`/runs/${group.runId}`} className="hover:underline">
            Open the run
          </Link>{" "}
          ·{" "}
          {/* The old chat address of the run opens its topic, in the conversation or alone. */}
          <Link to={`/chat/${group.dotId}/${group.runId}`} className="hover:underline">
            Open conversation
          </Link>
        </p>
        {group.approvals.length > 1 && <RunBatch key={batchKey} group={group} />}
        {actions}
      </div>
      {group.approvals.map((approval) => (
        <ApprovalCard key={approval.id} approval={approval} dotName={group.dotName} />
      ))}
    </div>
  );
}

/// A team's member dot that is outside the team's account or workspace, and why.
function TeamScopeRow({ item, actions }: { item: TeamScopeIssue; actions?: ReactNode }) {
  return (
    <li
      aria-label={`${item.dot_name} in team ${item.team_name} needs attention`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <Link to={`/dots/${item.dot_id}`} className="font-medium hover:underline">
          {item.dot_name}
        </Link>
        <span className="text-xs text-muted-foreground">
          Team {item.team_name} · {item.reason}
        </span>
      </div>
      {actions}
    </li>
  );
}

/// Instructions a dot proposed, with links to its dot and its conversation.
function InstructionsRow({ item, actions }: { item: PendingInstructions; actions?: ReactNode }) {
  const { proposal, dot_name: dotName } = item;
  return (
    <li className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-xs text-muted-foreground">
          <Link to={`/dots/${proposal.dot_id}`} className="text-foreground hover:underline">
            {dotName}
          </Link>{" "}
          proposed {relativeTime(proposal.created_at)} ·{" "}
          {/* The old chat address of the run opens its topic, in the conversation or alone. */}
          <Link to={`/chat/${proposal.dot_id}/${proposal.run_id}`} className="hover:underline">
            Open conversation
          </Link>
        </p>
        {actions}
      </div>
      <InstructionsCard proposal={proposal} dotName={dotName} />
    </li>
  );
}

/// A section of the Inbox under its heading; nothing when it has no rows.
function Section(props: { id: string; title: string; count: number; children: ReactNode }) {
  if (props.count === 0) return null;
  return (
    <section aria-labelledby={props.id} className="flex flex-col gap-2">
      {/* Focusable from code: focus lands here when an item of the section was put away. */}
      <h2 id={props.id} tabIndex={-1} className="font-semibold outline-none">
        {props.title}
      </h2>
      {props.children}
    </section>
  );
}

/// The items of `items` back from snooze, and the others.
function split<T>(items: T[], key: (item: T) => InboxKey, back: Set<string>): [T[], T[]] {
  const isBack = (item: T) => back.has(keyString(key(item)));
  return [items.filter((item) => !isBack(item)), items.filter(isBack)];
}

/// Everything that waits for the user (snooze and done spec section 6): first what came back
/// from snooze, with its tag; then messages, approvals, messages that did not reach their dot,
/// and the rest; last the Snoozed and Done lists. Each row has **Snooze**, and **Done** but on
/// approvals. Hidden items are not listed: the core leaves them out of its sections, and the
/// messages are left out here.
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
  // The dots' open questions and unread notes come first (check-ins spec section 9).
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  });
  // After Snooze or Done the item's row goes, and focus with it: once the Inbox has reloaded,
  // focus moves to the heading of the section it was in, or to the page's when that is gone.
  const focusTo = useRef<string | null | undefined>(undefined);
  const acted = useCallback((headingId: string | null) => {
    focusTo.current = headingId;
  }, []);
  useEffect(() => {
    if (focusTo.current === undefined) return;
    const active = document.activeElement;
    if (active !== null && active !== document.body) return;
    const heading =
      (focusTo.current !== null ? document.getElementById(focusTo.current) : null) ??
      document.querySelector<HTMLElement>("h1");
    focusTo.current = undefined;
    if (heading === null) return;
    if (!heading.hasAttribute("tabindex")) heading.setAttribute("tabindex", "-1");
    heading.focus();
  }, [inbox.data, messages.data]);
  if (inbox.isPending || messages.isPending) return <Page title="Inbox" />;
  if (inbox.isError) {
    return (
      <Page title="Inbox">
        <ErrorNote error={inbox.error} />
      </Page>
    );
  }
  const data: Inbox = inbox.data;
  const back = backKeys(data);
  const [waiting, backMessages] = split(
    shownMessages(data, messages.data ?? []),
    messageKey,
    back,
  );
  const [approvals, backApprovals] = split(data.approvals, approvalKey, back);
  const [undelivered, backUndelivered] = split(data.not_delivered, notDeliveredKey, back);
  const [reviews, backReviews] = split(data.app_reviews, appReviewKey, back);
  const [instructions, backInstructions] = split(data.instructions, instructionsKey, back);
  const [attention, backAttention] = split(data.attention, attentionKey, back);
  const [teamScope, backTeamScope] = split(data.team_scope, teamScopeKey, back);
  const [watchers, backWatchers] = split(data.watchers, watcherKey, back);
  const [held, backHeld] = split(data.held, heldKey, back);
  const [tools, backTools] = split(data.tools_changed, toolsKey, back);
  const [runs, backRuns] = split(data.review_runs, reviewKey, back);
  // A waiting workflow run has no Snooze or Done: the core gives it no Inbox key, and it
  // leaves the Inbox once the person acts on it.
  const workflows = data.workflows;

  const act = (key: InboxKey, what: string) => <InboxActions items={[key]} done what={what} />;
  const row = {
    message: (m: Message) => (
      <MessageCard
        key={m.id}
        message={m}
        actions={act(messageKey(m), `${m.dot_name}'s ${m.kind === "question" ? "question" : "note"}`)}
      />
    ),
    approvals: (group: RunApprovals) => (
      <RunApprovalsGroup
        key={group.runId}
        group={group}
        // An approval is decided, not marked done: the group snoozes each call it shows.
        actions={
          <InboxActions
            items={group.approvals.map(approvalKey)}
            done={false}
            what={`approvals asked by ${group.dotName}`}
          />
        }
      />
    ),
    undelivered: (item: UndeliveredMessage) => (
      <NotDeliveredRow key={item.id} item={item} actions={act(notDeliveredKey(item), `${item.dot_name}'s message that was not delivered`)} />
    ),
    review: (item: PendingAppReview) => (
      <AppReviewRow key={item.proposal.id} item={item} actions={act(appReviewKey(item), `${item.dot_name}'s app to review`)} />
    ),
    instructions: (item: PendingInstructions) => (
      <InstructionsRow
        key={item.proposal.id}
        item={item}
        actions={act(instructionsKey(item), `${item.dot_name}'s new instructions`)}
      />
    ),
    attention: (item: NeedsAttention) => (
      <AttentionRow key={item.dot_id} item={item} actions={act(attentionKey(item), `${item.dot_name}'s schedule`)} />
    ),
    teamScope: (item: TeamScopeIssue) => (
      <TeamScopeRow
        key={`${item.team_id}/${item.dot_id}`}
        item={item}
        actions={act(teamScopeKey(item), `${item.dot_name} in team ${item.team_name}`)}
      />
    ),
    watcher: (item: PausedWatcher) => (
      <PausedWatcherRow key={item.watcher_id} item={item} actions={act(watcherKey(item), `${item.dot_name}'s watcher ${item.name}`)} />
    ),
    held: (item: HeldRun) => (
      <HeldRunRow key={item.run_id} item={item} actions={act(heldKey(item), `${item.dot_name}'s run waiting to run`)} />
    ),
    tools: (item: ToolsChanged) => (
      <ToolsChangedRow
        key={`${item.server_id}/${item.scope}`}
        item={item}
        actions={act(toolsKey(item), `${item.name} tools`)}
      />
    ),
    run: (run: ReviewRun) => <ReviewRow key={run.id} run={run} actions={act(reviewKey(run), `${run.dot_name}'s run to review`)} />,
  };

  // Back from snooze: each item's own card or row, in the order they came back (newest
  // first). Approvals come back as their run's group, where the first of them came back.
  const backCards = new Map<string, { node: ReactNode; row: boolean }>();
  const add = <T,>(items: T[], key: (item: T) => InboxKey, render: (item: T) => ReactNode) => {
    for (const item of items) backCards.set(keyString(key(item)), { node: render(item), row: true });
  };
  add(backMessages, messageKey, row.message);
  for (const group of byRun(backApprovals)) {
    const card = { node: row.approvals(group), row: false };
    for (const approval of group.approvals) backCards.set(keyString(approvalKey(approval)), card);
  }
  add(backUndelivered, notDeliveredKey, row.undelivered);
  add(backReviews, appReviewKey, row.review);
  add(backInstructions, instructionsKey, row.instructions);
  add(backAttention, attentionKey, row.attention);
  add(backTeamScope, teamScopeKey, row.teamScope);
  add(backWatchers, watcherKey, row.watcher);
  add(backHeld, heldKey, row.held);
  add(backTools, toolsKey, row.tools);
  add(backRuns, reviewKey, row.run);
  const shownBack = new Set<{ node: ReactNode; row: boolean }>();
  const backList: ReactNode[] = [];
  for (const mark of data.back) {
    const card = backCards.get(keyString(mark));
    if (!card || shownBack.has(card)) continue;
    shownBack.add(card);
    backList.push(
      <div key={keyString(mark)} className="flex flex-col items-start gap-1">
        <Badge variant="secondary">Back from snooze</Badge>
        <div className="w-full">{card.row ? <ul>{card.node}</ul> : card.node}</div>
      </div>,
    );
  }

  const nothing =
    !messages.isError &&
    backList.length === 0 &&
    waiting.length === 0 &&
    approvals.length === 0 &&
    undelivered.length === 0 &&
    runs.length === 0 &&
    attention.length === 0 &&
    teamScope.length === 0 &&
    tools.length === 0 &&
    watchers.length === 0 &&
    held.length === 0 &&
    instructions.length === 0 &&
    reviews.length === 0 &&
    workflows.length === 0;
  return (
    <InboxActed.Provider value={acted}>
      <Page title="Inbox">
        {messages.isError && <ErrorNote error={messages.error} />}
        {nothing && (
          <EmptyState icon={InboxIcon}>
            Nothing waits for you. Questions and notes from dots, approvals a dot asks for,
            messages that did not reach a dot, apps sent for review, workflow runs that wait for
            you, schedules that need attention, paused watchers and finished runs to review show
            up here.
          </EmptyState>
        )}
        <Section id="inbox-back" title="Back from snooze" count={backList.length}>
          <div className="flex flex-col gap-2">{backList}</div>
        </Section>
        <Section id="inbox-messages" title="Messages" count={waiting.length}>
          <ul className="flex flex-col gap-2">{waiting.map(row.message)}</ul>
        </Section>
        <Section id="inbox-approvals" title="Approvals" count={approvals.length}>
          {byRun(approvals).map(row.approvals)}
        </Section>
        <Section id="inbox-not-delivered" title="Not delivered" count={undelivered.length}>
          <ul className="flex flex-col gap-2">{undelivered.map(row.undelivered)}</ul>
        </Section>
        <Section id="inbox-app-reviews" title="Apps to review" count={reviews.length}>
          <ul className="flex flex-col gap-2">{reviews.map(row.review)}</ul>
        </Section>
        <Section id="inbox-workflows" title="Workflows" count={workflows.length}>
          <ul className="flex flex-col gap-2">
            {workflows.map((item) => (
              <WaitingRunRow key={item.id} item={item} />
            ))}
          </ul>
        </Section>
        <Section id="inbox-instructions" title="New instructions" count={instructions.length}>
          <ul className="flex flex-col gap-2">{instructions.map(row.instructions)}</ul>
        </Section>
        <Section
          id="inbox-attention"
          title="Needs attention"
          count={attention.length + teamScope.length}
        >
          <ul className="flex flex-col gap-2">
            {attention.map(row.attention)}
            {teamScope.map(row.teamScope)}
          </ul>
        </Section>
        <Section id="inbox-watchers" title="Paused watchers" count={watchers.length}>
          <ul className="flex flex-col gap-2">{watchers.map(row.watcher)}</ul>
        </Section>
        <Section id="inbox-held" title="Waiting to run" count={held.length}>
          <ul className="flex flex-col gap-2">{held.map(row.held)}</ul>
        </Section>
        <Section id="inbox-tools" title="Tools changed" count={tools.length}>
          <ul className="flex flex-col gap-2">{tools.map(row.tools)}</ul>
        </Section>
        <Section id="inbox-runs" title="Runs to review" count={runs.length}>
          <ul className="flex flex-col gap-2">{runs.map(row.run)}</ul>
        </Section>
        <MarkedList key="snoozed" title="Snoozed" marks={data.snoozed} />
        <MarkedList key="done" title="Done" marks={data.done} />
      </Page>
    </InboxActed.Provider>
  );
}

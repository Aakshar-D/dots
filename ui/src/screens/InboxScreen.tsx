import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Inbox as InboxIcon, Play } from "lucide-react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type {
  HeldRun,
  NeedsAttention,
  PausedWatcher,
  PendingInstructions,
  ReviewRun,
  ToolsChanged,
} from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { InstructionsCard } from "@/components/InstructionsCard";
import { MessageCard } from "@/components/MessageCard";
import { Page } from "@/components/Page";
import { ReviewActions } from "@/components/ReviewActions";
import { StatusBadge } from "@/components/StatusBadge";
import { Button } from "@/components/ui/button";
import { byRun, type RunApprovals } from "@/lib/approvals";
import { relativeTime } from "@/lib/format";
import { triggerLabel } from "@/lib/home-topics";
import { waits } from "@/lib/messages";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { toolPreview } from "@/lib/transcript";
import { releasable } from "@/components/RunNumbers";
import { resetTime } from "@/lib/usage-format";

/// A finished run that waits for the user: it failed, or it left a worktree to look at.
function ReviewRow({ run }: { run: ReviewRun }) {
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
    </li>
  );
}

/// A watcher Dots paused (watchers spec section 7): why, since when, Resume, and its dot's tab.
function PausedWatcherRow({ item }: { item: PausedWatcher }) {
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
    </li>
  );
}

/// A background run held for Claude usage (usage and limits spec section 5): when it starts,
/// and **Run anyway**.
function HeldRunRow({ item }: { item: HeldRun }) {
  const qc = useQueryClient();
  const limits = useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data;
  const release = useMutation({
    mutationFn: () => commands.runReleaseHold(item.run_id),
    onError: notifyError,
    onSettled: () => {
      for (const queryKey of [keys.inbox, keys.limits]) invalidate(qc, queryKey);
    },
  });
  const when = item.starts_after
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
      {releasable(item.trigger, limits) && (
        <Button
          size="sm"
          variant="outline"
          disabled={release.isPending}
          onClick={onSingleClick(() => release.mutate())}
        >
          Run anyway
        </Button>
      )}
    </li>
  );
}

/// A schedule whose last occurrence failed in the end: its retries were used up, or its run
/// had called a tool. The schedule keeps firing; the flag goes when a later run of the dot
/// succeeds or when it is dismissed here.
function AttentionRow({ item }: { item: NeedsAttention }) {
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
    </li>
  );
}

/// An MCP server whose tool list changed since it was accepted in one scope, Personal or an
/// org's: its tools ask there until a review. An org's row names the org, and its review
/// accepts the org's list.
function ToolsChangedRow({ item }: { item: ToolsChanged }) {
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
    </li>
  );
}

/// "Approve all" and "Deny all" for the calls of one run that the Inbox lists now. Mounted per
/// set of calls (its key): one answer per set, and a new set asked after a batch can be
/// answered again.
function RunBatch({ group }: { group: RunApprovals }) {
  const qc = useQueryClient();
  const decideAll = useMutation({
    mutationFn: (approved: boolean) => commands.decideRunApprovals(group.runId, approved, null),
    onSuccess: () => {
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.approvals(group.runId));
    },
    onError: notifyError,
  });
  // One answer per set: after a success the buttons stay off until the Inbox has reloaded.
  const decided = decideAll.isPending || decideAll.isSuccess;
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
            {group.dotName} may perform every call this run waits for, in the order it asked:
            {calls}
          </>
        }
        confirmLabel="Approve all"
        disabled={decided}
        onConfirm={() => decideAll.mutate(true)}
      >
        Approve all ({count})
      </ConfirmButton>
      <ConfirmButton
        title={`Deny all ${count} calls?`}
        description={
          <>
            {group.dotName} may perform none of the calls this run waits for, and gets no note:
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
function RunApprovalsGroup({ group }: { group: RunApprovals }) {
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
      </div>
      {group.approvals.map((approval) => (
        <ApprovalCard key={approval.id} approval={approval} dotName={group.dotName} />
      ))}
    </div>
  );
}

/// Instructions a dot proposed, with links to its dot and its conversation.
function InstructionsRow({ item }: { item: PendingInstructions }) {
  const { proposal, dot_name: dotName } = item;
  return (
    <li className="flex flex-col gap-2">
      <p className="text-xs text-muted-foreground">
        <Link to={`/dots/${proposal.dot_id}`} className="text-foreground hover:underline">
          {dotName}
        </Link>{" "}
        proposed {relativeTime(proposal.created_at)} ·{" "}
        {/* The old chat address of the run opens its topic, in the conversation or alone. */}
        <Link to={`/chat/${proposal.dot_id}/${proposal.run_id}`} className="hover:underline">
          Open conversation
        </Link>
      </p>
      <InstructionsCard proposal={proposal} dotName={dotName} />
    </li>
  );
}

/// Everything that waits for the user: tool calls to approve, schedules that need attention,
/// and finished runs to review.
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
  // The dots' open questions and unread notes come first (check-ins spec section 9).
  const messages = useQuery({
    queryKey: keys.messages(null),
    queryFn: () => commands.messagesList(null),
  });
  if (inbox.isPending || messages.isPending) return <Page title="Inbox" />;
  if (inbox.isError) {
    return (
      <Page title="Inbox">
        <ErrorNote error={inbox.error} />
      </Page>
    );
  }
  const {
    approvals,
    review_runs: runs,
    attention,
    tools_changed: tools,
    watchers,
    held,
    instructions,
  } = inbox.data;
  const waiting = (messages.data ?? []).filter(waits);
  if (
    !messages.isError &&
    waiting.length === 0 &&
    approvals.length === 0 &&
    runs.length === 0 &&
    attention.length === 0 &&
    tools.length === 0 &&
    watchers.length === 0 &&
    held.length === 0 &&
    instructions.length === 0
  ) {
    return (
      <Page title="Inbox">
        <EmptyState icon={InboxIcon}>
          Nothing waits for you. Questions and notes from dots, approvals a dot asks for,
          schedules that need attention, paused watchers and finished runs to review show up
          here.
        </EmptyState>
      </Page>
    );
  }
  return (
    <Page title="Inbox">
      {messages.isError && <ErrorNote error={messages.error} />}
      {waiting.length > 0 && (
        <section aria-labelledby="inbox-messages" className="flex flex-col gap-2">
          <h2 id="inbox-messages" className="font-semibold">
            Messages
          </h2>
          <ul className="flex flex-col gap-2">
            {waiting.map((message) => (
              <MessageCard key={message.id} message={message} />
            ))}
          </ul>
        </section>
      )}
      {approvals.length > 0 && (
        <section aria-labelledby="inbox-approvals" className="flex flex-col gap-2">
          <h2 id="inbox-approvals" className="font-semibold">
            Approvals
          </h2>
          {byRun(approvals).map((group) => (
            <RunApprovalsGroup key={group.runId} group={group} />
          ))}
        </section>
      )}
      {instructions.length > 0 && (
        <section aria-labelledby="inbox-instructions" className="flex flex-col gap-2">
          <h2 id="inbox-instructions" className="font-semibold">
            New instructions
          </h2>
          <ul className="flex flex-col gap-2">
            {instructions.map((item) => (
              <InstructionsRow key={item.proposal.id} item={item} />
            ))}
          </ul>
        </section>
      )}
      {attention.length > 0 && (
        <section aria-labelledby="inbox-attention" className="flex flex-col gap-2">
          <h2 id="inbox-attention" className="font-semibold">
            Needs attention
          </h2>
          <ul className="flex flex-col gap-2">
            {attention.map((item) => (
              <AttentionRow key={item.dot_id} item={item} />
            ))}
          </ul>
        </section>
      )}
      {watchers.length > 0 && (
        <section aria-labelledby="inbox-watchers" className="flex flex-col gap-2">
          <h2 id="inbox-watchers" className="font-semibold">
            Paused watchers
          </h2>
          <ul className="flex flex-col gap-2">
            {watchers.map((item) => (
              <PausedWatcherRow key={item.watcher_id} item={item} />
            ))}
          </ul>
        </section>
      )}
      {held.length > 0 && (
        <section aria-labelledby="inbox-held" className="flex flex-col gap-2">
          <h2 id="inbox-held" className="font-semibold">
            Held for Claude usage
          </h2>
          <ul className="flex flex-col gap-2">
            {held.map((item) => (
              <HeldRunRow key={item.run_id} item={item} />
            ))}
          </ul>
        </section>
      )}
      {tools.length > 0 && (
        <section aria-labelledby="inbox-tools" className="flex flex-col gap-2">
          <h2 id="inbox-tools" className="font-semibold">
            Tools changed
          </h2>
          <ul className="flex flex-col gap-2">
            {tools.map((item) => (
              <ToolsChangedRow key={`${item.server_id}/${item.scope}`} item={item} />
            ))}
          </ul>
        </section>
      )}
      {runs.length > 0 && (
        <section aria-labelledby="inbox-runs" className="flex flex-col gap-2">
          <h2 id="inbox-runs" className="font-semibold">
            Runs to review
          </h2>
          <ul className="flex flex-col gap-2">
            {runs.map((run) => (
              <ReviewRow key={run.id} run={run} />
            ))}
          </ul>
        </section>
      )}
    </Page>
  );
}

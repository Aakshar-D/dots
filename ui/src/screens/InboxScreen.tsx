import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { ReviewRun } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { ReviewActions } from "@/components/ReviewActions";
import { StatusBadge } from "@/components/StatusBadge";
import { Button } from "@/components/ui/button";
import { byRun, type RunApprovals } from "@/lib/approvals";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { toolPreview } from "@/lib/transcript";

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

/// The tool calls one run waits for. Two or more can be answered together: the core decides
/// them in the order they were asked and resumes the run once. A rule is saved only with a
/// single approval, on its card.
function RunApprovalsGroup({ group }: { group: RunApprovals }) {
  const qc = useQueryClient();
  const decideAll = useMutation({
    mutationFn: (approved: boolean) => commands.decideRunApprovals(group.runId, approved, null),
    onSuccess: () => {
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.approvals(group.runId));
    },
    onError: notifyError,
  });
  // One answer per group: after a success the group stays until the Inbox has reloaded.
  const decided = decideAll.isPending || decideAll.isSuccess;
  const count = group.approvals.length;
  const calls = group.approvals.map((approval) => (
    <span key={approval.id} className="block font-mono text-xs">
      {approval.tool} {toolPreview(approval.input)}
    </span>
  ));
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
          </Link>
        </p>
        {count > 1 && (
          <>
            <ConfirmButton
              title={`Approve all ${count} calls?`}
              description={
                <>
                  {group.dotName} may perform every call this run waits for, in the order it
                  asked:
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
                  {group.dotName} may perform none of the calls this run waits for, and gets no
                  note:
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
        )}
      </div>
      {group.approvals.map((approval) => (
        <ApprovalCard key={approval.id} approval={approval} dotName={group.dotName} />
      ))}
    </div>
  );
}

/// Everything that waits for the user: tool calls to approve, and finished runs to review.
export function InboxScreen() {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
  if (inbox.isPending) return <Page title="Inbox" />;
  if (inbox.isError) {
    return (
      <Page title="Inbox">
        <ErrorNote error={inbox.error} />
      </Page>
    );
  }
  const { approvals, review_runs: runs } = inbox.data;
  if (approvals.length === 0 && runs.length === 0) {
    return (
      <Page title="Inbox">
        <p className="text-muted-foreground">
          Nothing waits for you. Approvals a dot asks for and finished runs to review show up
          here.
        </p>
      </Page>
    );
  }
  return (
    <Page title="Inbox">
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

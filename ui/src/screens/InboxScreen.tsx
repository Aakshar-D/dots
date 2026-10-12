import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { ReviewRun, Run } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { StatusBadge } from "@/components/StatusBadge";
import { Button } from "@/components/ui/button";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";

/// A finished run that waits for the user: it failed, or it left a worktree to look at.
function ReviewRow({ run }: { run: ReviewRun }) {
  const qc = useQueryClient();
  const done = (message: string) => (changed: Run) => {
    qc.setQueryData(keys.run(changed.id), changed);
    for (const queryKey of [["thread"], ["diff"]]) {
      void qc.invalidateQueries({ queryKey });
    }
    toast.success(message);
  };
  // Also after a refusal: a run that is gone or was reviewed elsewhere has to leave the list.
  const reload = () => void qc.invalidateQueries({ queryKey: keys.inbox });
  const markReviewed = useMutation({
    mutationFn: () => commands.markReviewed(run.id),
    onSuccess: done("Marked as reviewed"),
    onError: notifyError,
    onSettled: reload,
  });
  const discard = useMutation({
    mutationFn: () => commands.discardWorktree(run.id),
    onSuccess: done("Worktree discarded"),
    onError: notifyError,
    onSettled: reload,
  });
  const busy = markReviewed.isPending || discard.isPending;
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
      <Button size="sm" variant="outline" disabled={busy} onClick={() => markReviewed.mutate()}>
        Mark reviewed
      </Button>
      {run.worktree_exists && (
        <ConfirmButton
          title="Discard this worktree?"
          description={
            <>
              The worktree folder and the branch <code>{run.branch}</code> are deleted, with
              every change the dot made there. This cannot be undone.
            </>
          }
          confirmLabel="Discard the worktree"
          onConfirm={() => discard.mutate()}
          disabled={busy}
        >
          Discard worktree
        </ConfirmButton>
      )}
    </li>
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
          {approvals.map((approval) => (
            <div key={approval.id} className="flex flex-col gap-1">
              <p className="text-xs text-muted-foreground">
                <Link to={`/dots/${approval.dot_id}`} className="text-foreground hover:underline">
                  {approval.dot_name}
                </Link>{" "}
                asked {relativeTime(approval.created_at)} ·{" "}
                <Link to={`/runs/${approval.run_id}`} className="hover:underline">
                  Open the run
                </Link>
              </p>
              <ApprovalCard approval={approval} />
            </div>
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

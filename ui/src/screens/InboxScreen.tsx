import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { ReviewRun } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { ReviewActions } from "@/components/ReviewActions";
import { StatusBadge } from "@/components/StatusBadge";
import { Button } from "@/components/ui/button";
import { relativeTime } from "@/lib/format";
import { keys } from "@/lib/query";

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

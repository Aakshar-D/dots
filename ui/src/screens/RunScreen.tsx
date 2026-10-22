import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpen } from "lucide-react";
import { Link, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { Approval, Run, RunEventRecord } from "@/api/types";
import { AppliedCalls } from "@/components/AppliedCalls";
import { ApprovalCard } from "@/components/ApprovalCard";
import { DiffView } from "@/components/DiffView";
import { ErrorNote, LoadErrors } from "@/components/ErrorNote";
import { Markdown } from "@/components/Markdown";
import { Page } from "@/components/Page";
import { ReviewActions } from "@/components/ReviewActions";
import { StatusBadge } from "@/components/StatusBadge";
import { Transcript } from "@/components/Transcript";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { isActive } from "@/lib/home-topics";
import { duration, localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { eventsQuery, keys } from "@/lib/query";
import { useNow } from "@/lib/use-now";

/// One thread: the run in the address and every other run of its lineage, as one transcript,
/// with what the thread changed and the actions on it.
function Thread({ runId, runs }: { runId: string; runs: Run[] }) {
  const qc = useQueryClient();
  const latest = runs.at(-1)!;
  const active = runs.find(isActive);
  const now = useNow(active !== undefined);
  const dot = useQuery({
    queryKey: keys.dot(latest.dot_id),
    queryFn: () => commands.getDot(latest.dot_id),
  });
  const events = useQueries({ queries: runs.map((run) => eventsQuery(qc, run.id)) });
  const approvals = useQueries({
    queries: runs.map((run) => ({
      queryKey: keys.approvals(run.id),
      queryFn: () => commands.approvalsForRun(run.id),
    })),
  });
  const hasWorktree = latest.branch !== null;
  const diff = useQuery({
    queryKey: keys.diff(runId),
    queryFn: () => commands.worktreeDiff(runId),
    enabled: hasWorktree,
  });

  // The folder is gone once an unchanged worktree was cleaned up or a discard ran; the run keeps
  // its branch name and its path, so only the diff says whether there is still something to
  // discard or to open. Until the diff has answered, neither is known.
  const worktreeKept = diff.data?.exists === true;
  const worktreeGone = diff.data?.exists === false;

  const eventsByRun = new Map<string, RunEventRecord[]>(
    runs.map((run, i) => [run.id, events[i]?.data ?? []]),
  );
  const asked: Approval[] = approvals.flatMap((q) => q.data ?? []);
  const pending = asked.filter((a) => a.status === "pending");

  const cancel = useMutation({ mutationFn: commands.cancelRun, onError: notifyError });
  const openFolder = useMutation({
    mutationFn: () => commands.openRunFolder(runId),
    onError: notifyError,
  });

  // A diff that failed to load does not say whether a worktree waits to be looked at. The run
  // can be marked all the same, so that a diff that cannot be read does not keep it unreviewed.
  const needsReview =
    !active &&
    latest.reviewed_at === null &&
    (latest.status === "failed" || worktreeKept || diff.isError);
  const tokensIn = runs.reduce((n, r) => n + r.tokens_in, 0);
  const tokensOut = runs.reduce((n, r) => n + r.tokens_out, 0);
  const first = runs[0];
  const took = duration(first.started_at, active ? null : latest.ended_at, now);

  const actions = (
    <>
      <StatusBadge status={latest.status} />
      {active && (
        <Button
          size="sm"
          variant="outline"
          disabled={cancel.isPending}
          onClick={() => cancel.mutate(active.id)}
        >
          Cancel run
        </Button>
      )}
      {latest.workspace_path !== null && !worktreeGone && (
        <Button
          size="sm"
          variant="outline"
          disabled={openFolder.isPending}
          onClick={() => openFolder.mutate()}
        >
          <FolderOpen aria-hidden /> Open folder
        </Button>
      )}
      <ReviewActions
        runId={runId}
        branch={latest.branch}
        canReview={needsReview}
        canDiscard={worktreeKept && !active}
      />
    </>
  );

  const transcript = (
    <div className="flex flex-col gap-4">
      <LoadErrors queries={[...events, ...approvals]} />
      {pending.map((approval) => (
        <ApprovalCard key={approval.id} approval={approval} dotName={dot.data?.name} />
      ))}
      <AppliedCalls approvals={asked} />
      {/* The run's error stands above, so the transcript does not repeat it. */}
      <Transcript runs={runs} events={eventsByRun} shownError={latest.error} />
    </div>
  );

  return (
    <Page title={dot.data ? `${dot.data.name} run` : "Run"} actions={actions}>
      <dl className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-muted-foreground">
        {dot.data && (
          <div className="flex gap-1">
            <dt>Dot</dt>
            <dd>
              <Link to={`/dots/${dot.data.id}`} className="text-foreground hover:underline">
                {dot.data.name}
              </Link>
            </dd>
          </div>
        )}
        <div className="flex gap-1">
          <dt>Queued</dt>
          <dd className="text-foreground">{localDateTime(first.queued_at)}</dd>
        </div>
        <div className="flex gap-1">
          {/* A thread that waits to start runs for no time yet: it keeps "not started". */}
          <dt>{active && took !== "" ? "Running for" : "Took"}</dt>
          <dd className="text-foreground">{took || "not started"}</dd>
        </div>
        <div className="flex gap-1">
          <dt>Tokens</dt>
          <dd className="text-foreground">
            {tokensIn} in, {tokensOut} out
          </dd>
        </div>
        {latest.branch && (
          <div className="flex gap-1">
            <dt>Branch</dt>
            <dd className="font-mono text-foreground">{latest.branch}</dd>
          </div>
        )}
      </dl>
      {latest.error && (
        <p role="alert" className="rounded-md border border-destructive/50 p-3 text-destructive">
          {latest.error}
        </p>
      )}
      {latest.summary && (
        <section aria-label="Summary" className="rounded-md border p-3">
          <Markdown text={latest.summary} />
        </section>
      )}
      {hasWorktree ? (
        <Tabs defaultValue="transcript">
          <TabsList>
            <TabsTrigger value="transcript">Transcript</TabsTrigger>
            <TabsTrigger value="changes">Changes</TabsTrigger>
          </TabsList>
          <TabsContent value="transcript">{transcript}</TabsContent>
          <TabsContent value="changes">
            {diff.isError ? (
              <ErrorNote error={diff.error} />
            ) : diff.data && !diff.data.exists ? (
              <p className="text-muted-foreground">
                This run's worktree was removed: it had no changes, or it was discarded.
              </p>
            ) : (
              diff.data && <DiffView diff={diff.data} />
            )}
          </TabsContent>
        </Tabs>
      ) : (
        transcript
      )}
    </Page>
  );
}

export function RunScreen() {
  const { runId = "" } = useParams();
  const thread = useQuery({
    queryKey: keys.thread(runId),
    queryFn: () => commands.runThread(runId),
  });
  if (thread.isPending) return <Page title="Run" />;
  if (thread.isError) {
    return (
      <Page title="Run">
        <ErrorNote error={thread.error} />
      </Page>
    );
  }
  if (thread.data.length === 0) {
    return (
      <Page title="Run">
        <ErrorNote error="This run has no thread." />
      </Page>
    );
  }
  return <Thread key={runId} runId={runId} runs={thread.data} />;
}

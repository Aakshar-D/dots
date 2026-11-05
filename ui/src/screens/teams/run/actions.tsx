import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { WorkflowRun } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { DiffView } from "@/components/DiffView";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// After the core answered an action on a workflow run with the run as it is now: the run page
/// shows it at once, and the lists of runs (the team page's, a conversation's cards) and what
/// needs the person are read again. The run's own event follows and changes nothing more.
export function settle(qc: QueryClient, next: WorkflowRun): void {
  qc.setQueryData(keys.workflowRun(next.id), next);
  for (const queryKey of [["workflow-runs"], keys.workflows(next.team_id), keys.inbox]) {
    invalidate(qc, queryKey);
  }
}

/// Whether a workflow run can still be acted on: it is running or waiting.
export function inFlight(run: Pick<WorkflowRun, "status">): boolean {
  return run.status === "running" || run.status === "waiting";
}

/// "Stop" (section 5.4): asks first, then cancels the step at work and every step after it.
export function StopRun({ run }: { run: WorkflowRun }) {
  const qc = useQueryClient();
  const stop = useMutation({
    mutationFn: () => commands.workflowStop(run.id),
    onSuccess: (next) => settle(qc, next),
    onError: notifyError,
  });
  return (
    <ConfirmButton
      title="Stop this workflow run?"
      description="The step at work stops and no step after it runs. Its branches stay, and nothing already pushed is undone."
      confirmLabel="Stop run"
      disabled={stop.isPending || !inFlight(run)}
      onConfirm={() => stop.mutate()}
    >
      Stop
    </ConfirmButton>
  );
}

/// "Retry step" on a failed step (section 10.3): a new attempt, counted against the budget.
export function RetryStep({ run }: { run: WorkflowRun }) {
  const qc = useQueryClient();
  const retry = useMutation({
    mutationFn: () => commands.workflowRetryStep(run.id),
    onSuccess: (next) => settle(qc, next),
    onError: notifyError,
  });
  return (
    <Button size="sm" disabled={retry.isPending} onClick={() => retry.mutate()}>
      Retry step
    </Button>
  );
}

/// "View diff" of a branch a check shows or a deploy names, at `commit`, in a dialog: what its
/// commits change from the project's default branch. A branch that is gone (no commit) has none.
export function ViewDiffButton(props: { runId: string; branch: string; commit: string | null }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        aria-label={`View diff of ${props.branch}`}
        title={props.commit ? undefined : "The branch is gone."}
        disabled={!props.commit}
        onClick={() => setOpen(true)}
      >
        View diff
      </Button>
      {open && props.commit && (
        <BranchDiffDialog
          runId={props.runId}
          branch={props.branch}
          commit={props.commit}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

function BranchDiffDialog(props: {
  runId: string;
  branch: string;
  commit: string;
  onClose: () => void;
}) {
  const diff = useQuery({
    queryKey: keys.workflowBranchDiff(props.runId, props.branch, props.commit),
    queryFn: () => commands.workflowBranchDiff(props.runId, props.branch, props.commit),
  });
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="break-all">Diff of {props.branch}</DialogTitle>
          <DialogDescription>
            What its commits change, from the project's default branch, at{" "}
            <span className="font-mono">{props.commit.slice(0, 7)}</span>.
          </DialogDescription>
        </DialogHeader>
        {diff.isError ? (
          <ErrorNote error={diff.error} />
        ) : diff.data ? (
          <DiffView diff={diff.data} />
        ) : (
          <p className="text-muted-foreground">Reading the diff…</p>
        )}
      </DialogContent>
    </Dialog>
  );
}

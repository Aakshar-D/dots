import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { LoaderCircle } from "lucide-react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { WorkflowRun } from "@/api/types";
import { Button } from "@/components/ui/button";
import { keys } from "@/lib/query";

import { formatSpend, taskLine } from "../format";
import { TestBadge, WorkflowRunBadge } from "../WorkflowRunBadge";
import { inFlight, RetryStep, StopRun } from "./actions";
import { CheckPanel } from "./CheckPanel";
import { LimitPanel } from "./LimitPanel";
import { cardSteps, type CardStep, latestStepRuns } from "./steps";

/// The workflow runs that the runs `runIds` started (teams spec section 10.4), by the run that
/// started each, oldest first: one query for every run a conversation shows. Every workflow
/// run's event marks it stale (`lib/runtime-events.ts`), so a new run's card shows and each
/// card follows its run.
export function useStartedWorkflows(runIds: string[]): {
  byRun: ReadonlyMap<string, WorkflowRun[]>;
  error: Error | null;
} {
  const started = useQuery({
    queryKey: keys.workflowRunsForOrigins(runIds),
    queryFn: () => commands.workflowRunsForOrigins(runIds),
    enabled: runIds.length > 0,
  });
  const byRun = new Map<string, WorkflowRun[]>();
  for (const run of started.data ?? []) {
    if (run.origin_run_id === null) continue;
    byRun.set(run.origin_run_id, [...(byRun.get(run.origin_run_id) ?? []), run]);
  }
  return { byRun, error: started.isError ? started.error : null };
}

/// The look of a step's mark: done, waiting on the person, running (spinning), failed, stopped
/// or skipped, not reached.
const MARK: Record<CardStep["status"], { glyph: string; className: string }> = {
  succeeded: { glyph: "✓", className: "text-success" },
  waiting: { glyph: "●", className: "text-needs" },
  running: { glyph: "", className: "text-primary" },
  failed: { glyph: "✕", className: "text-destructive" },
  cancelled: { glyph: "–", className: "text-muted-foreground" },
  skipped: { glyph: "–", className: "text-muted-foreground" },
  not_reached: { glyph: "○", className: "text-muted-foreground" },
};

/// One step on the card's line: its mark, then its name. The item is named by the step and its
/// state.
function StepMark({ step }: { step: CardStep }) {
  const mark = MARK[step.status];
  return (
    <li
      aria-label={`${step.name}: ${step.label}`}
      data-status={step.status}
      title={step.label}
      className={cn("flex items-center gap-1", step.status === "waiting" && "text-needs")}
    >
      {step.status === "running" ? (
        <LoaderCircle
          aria-hidden
          className={cn("size-3 animate-spin motion-reduce:animate-none", mark.className)}
        />
      ) : (
        <span aria-hidden className={mark.className}>
          {mark.glyph}
        </span>
      )}
      {step.name}
    </li>
  );
}

/// A workflow run as a card in the conversation of the chat that started it (section 10.4,
/// option B), live: the team and the workflow, the task's first line, the status, the spend, the
/// steps as one line of marks, and what the person can do now: the check (View diff, Send back,
/// Approve), a limit (Add budget or one more send-back), a failed step (Retry step), Stop while
/// it is in flight, and always Open run. The actions call the workflow commands only: none of
/// them wakes the chat's dot.
export function WorkflowCard({ run }: { run: WorkflowRun }) {
  const team = useQuery({
    queryKey: keys.team(run.team_id),
    queryFn: () => commands.teamGet(run.team_id),
  });
  const workflowId = run.workflow_id;
  const workflow = useQuery({
    queryKey: keys.workflow(workflowId ?? ""),
    queryFn: () => commands.workflowGet(workflowId!),
    enabled: workflowId !== null,
  });
  const workflowName =
    workflowId === null || workflow.isError ? "A deleted workflow" : (workflow.data?.name ?? "");
  const task = taskLine(run.task);
  const stepFailed = run.status === "waiting" && run.waiting_on === "step_failed";
  // A failed step's error, or the run's when it failed; a step that failed says it once.
  const failedStep =
    stepFailed && run.current_step ? latestStepRuns(run).get(run.current_step) : undefined;
  const error =
    run.status === "failed" || stepFailed ? (run.error ?? failedStep?.error ?? null) : null;
  return (
    <article
      aria-label={`Workflow run: ${task || "No task."}`}
      data-status={run.status}
      className={cn(
        "flex flex-col gap-2 rounded-xl border bg-card px-4 py-3",
        run.status === "waiting" && "border-needs-border",
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-semibold">{team.data?.name ?? "Team"}</span>
        {workflowName && <span className="text-muted-foreground">· {workflowName}</span>}
        <span className="flex-1" />
        {run.test && <TestBadge />}
        <WorkflowRunBadge status={run.status} waitingOn={run.waiting_on} />
        <span className="text-xs text-muted-foreground">{formatSpend(run)}</span>
      </div>
      <p className="truncate">{task || "No task."}</p>
      <ol aria-label="Steps" className="flex flex-wrap gap-x-3 gap-y-1 text-xs">
        {cardSteps(run).map((step) => (
          <StepMark key={step.id} step={step} />
        ))}
      </ol>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <CheckPanel run={run} stop={false} />
      <LimitPanel run={run} stop={false} />
      <div className="flex flex-wrap items-center gap-2">
        <Button asChild size="sm" variant="outline">
          <Link to={`/workflow-runs/${run.id}`}>Open run</Link>
        </Button>
        {stepFailed && <RetryStep run={run} />}
        {inFlight(run) && <StopRun run={run} />}
      </div>
    </article>
  );
}

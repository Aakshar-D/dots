import { cn } from "cn";
import { ArrowDown } from "lucide-react";
import { Link } from "react-router";

import type { Step, StepDotRun, StepRun, WorkflowRun } from "@/api/types";
import { StatusBadge } from "@/components/StatusBadge";
import { Badge } from "@/components/ui/badge";

import { delegateLine, KIND_LABEL, type NameOf, stepLine } from "../builder/labels";
import { taskLine } from "../format";
import { RetryStep } from "./actions";
import { CheckPanel } from "./CheckPanel";
import { DeployResult } from "./DeployResult";
import { pathTaken, STEP_STATUS_LABEL, stepName, stepRunLinks } from "./steps";

/// What every card of the run page reads: the run, the newest step run of each step, the team's
/// names and the run's dot runs (to link each step to its own).
export type RunView = {
  run: WorkflowRun;
  latest: Map<string, StepRun>;
  nameOf: NameOf;
  dotRuns: StepDotRun[];
};

const MARK: Record<keyof typeof STEP_STATUS_LABEL, string> = {
  running: "animate-pulse bg-primary motion-reduce:animate-none",
  waiting: "bg-needs",
  succeeded: "bg-success",
  failed: "bg-destructive",
  cancelled: "bg-muted-foreground",
  skipped: "border border-muted-foreground bg-transparent",
  not_reached: "border border-dashed border-muted-foreground bg-transparent",
};

/// A step's state as a coloured dot, running with the pulse of a running dot.
function StepMark({ status }: { status: keyof typeof STEP_STATUS_LABEL }) {
  return (
    <span
      aria-hidden
      data-status={status}
      className={cn("inline-block size-2.5 shrink-0 rounded-full", MARK[status])}
    />
  );
}

/// The run's steps as the builder lays them out (section 10.3), numbered, joined by arrows, a
/// branch's paths as labelled columns; each card shows its newest step run.
export function RunSteps(props: { steps: Step[]; prefix: string; name: string; view: RunView }) {
  return (
    <ol aria-label={props.name} className="flex flex-col">
      {props.steps.map((step, i) => (
        <li key={step.id} className="flex flex-col">
          {i > 0 && (
            <div className="flex justify-center py-1 text-muted-foreground">
              <ArrowDown aria-hidden className="size-4" />
            </div>
          )}
          <StepRunCard step={step} number={`${props.prefix}${i + 1}`} view={props.view} />
        </li>
      ))}
    </ol>
  );
}

/// How a card looks: a waiting check in the needs colours, a delegate dashed in the accent, a
/// running step in the accent, a failed one in red.
function look(step: Step, stepRun: StepRun | undefined): string {
  if (step.kind === "check" && stepRun?.status === "waiting") {
    return "border-needs-border bg-needs-surface";
  }
  if (stepRun?.status === "failed") return "border-destructive";
  if (step.kind === "agent" && step.delegate) return "border-dashed border-primary";
  if (stepRun?.status === "running") return "border-primary";
  return "";
}

/// One step and its newest step run: its state and attempt, what the run page says about the
/// attempt, its error, outcome or decision, a delegate's members, a deploy's result, links to
/// its dot runs, and what the person can do here: the waiting check, or Retry step.
function StepRunCard({ step, number, view }: { step: Step; number: string; view: RunView }) {
  const { run, nameOf } = view;
  const stepRun = view.latest.get(step.id);
  const status = stepRun?.status ?? "not_reached";
  const name = step.name.trim() || "(no name)";
  const delegate = step.kind === "agent" ? delegateLine(step, nameOf) : undefined;
  const current = run.status === "waiting" && run.current_step === step.id;
  const links = stepRun ? stepRunLinks(stepRun, view.dotRuns) : null;
  const decision = stepRun?.decision;
  const deployError = stepRun?.deploy_result?.error;
  const label =
    step.kind === "check" && status === "waiting" ? "Waiting for you" : STEP_STATUS_LABEL[status];
  return (
    <div
      role="group"
      aria-label={`Step ${number}: ${name}`}
      data-step-id={step.id}
      data-status={status}
      className={cn(
        "flex w-full flex-col gap-1.5 rounded-xl border bg-card p-3",
        look(step, stepRun),
      )}
    >
      <div className="flex items-start gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className={cn("font-semibold", step.kind === "check" && "text-needs")}>
            {number} · {name}
          </span>
          <span className="text-xs text-muted-foreground">
            {stepLine(step, run.definition.steps, nameOf)}
          </span>
          {delegate && <span className="text-xs text-primary">{delegate}</span>}
        </div>
        <Badge variant="outline">{delegate ? "Delegate" : KIND_LABEL[step.kind]}</Badge>
        <span className="flex shrink-0 items-center gap-1.5 text-xs">
          <StepMark status={status} />
          {label}
          {stepRun && stepRun.attempt > 1 && (
            <span className="text-muted-foreground">· attempt {stepRun.attempt}</span>
          )}
        </span>
      </div>
      {stepRun?.notice && <p className="text-xs text-muted-foreground">{stepRun.notice}</p>}
      {stepRun?.error && stepRun.error !== deployError && (
        <p className="text-sm text-destructive">{stepRun.error}</p>
      )}
      {stepRun?.outcome && (
        <p className="text-xs">
          Outcome: <span className="font-medium">{stepRun.outcome}</span>
        </p>
      )}
      {decision && "decision" in decision && (
        <p className="text-xs">
          {decision.decision === "approved" ? "Approved" : "Sent back"}
          {decision.note ? `: ${decision.note}` : ""}
        </p>
      )}
      {stepRun?.send_back && (
        <p className="text-xs">
          Sends the work back to {stepName(run, stepRun.send_back.to)}: “{stepRun.send_back.note}”
        </p>
      )}
      {stepRun?.handoff && stepRun.handoff.members.length > 0 && (
        <ul aria-label={`Members ${name} started`} className="mt-1 flex flex-col gap-1 pl-4">
          {stepRun.handoff.members.map((member, i) => {
            const runId = member.branch ? links?.members.get(member.branch) : undefined;
            return (
              <li
                key={i}
                aria-label={taskLine(member.task)}
                className="flex flex-wrap items-center gap-2 rounded-md border bg-background p-2 text-xs"
              >
                <span className="font-medium">{nameOf(member.dot_id)}</span>
                <span className="min-w-0 flex-1">{taskLine(member.task)}</span>
                <StatusBadge status={member.status} />
                <span className="font-mono break-all text-muted-foreground">
                  {member.branch ?? "No branch"}
                </span>
                {runId && (
                  <Link to={`/runs/${runId}`} className="underline">
                    Open run
                  </Link>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {stepRun?.deploy_result && <DeployResult run={run} result={stepRun.deploy_result} />}
      {links && links.step.length > 0 && (
        <ul aria-label={`Sol runs of ${name}`} className="flex flex-wrap gap-2 text-xs">
          {links.step.map((link) => (
            <li key={link.id} className="flex items-center gap-1">
              <Link to={`/runs/${link.id}`} className="underline">
                Run of {link.label}
              </Link>
              <StatusBadge status={link.status} />
            </li>
          ))}
        </ul>
      )}
      {step.kind === "check" && current && <CheckPanel run={run} stop={false} />}
      {current && run.waiting_on === "step_failed" && stepRun?.status === "failed" && (
        <div className="flex flex-wrap items-center gap-2">
          <RetryStep run={run} />
          <span className="text-xs text-muted-foreground">
            A new attempt of the step, counted against the budget.
          </span>
        </div>
      )}
      {step.kind === "branch" && (
        <div className="mt-2 flex gap-3 overflow-x-auto pl-4">
          {[
            ...step.paths.map((path) => ({ label: path.label, steps: path.steps })),
            { label: null, steps: step.otherwise },
          ].map((path) => {
            const taken = pathTaken(stepRun);
            const title = path.label === null ? "Otherwise" : path.label.trim() || "(no label)";
            const region = path.label === null ? "Otherwise" : `Path ${path.label.trim()}`;
            const state = taken === undefined ? null : taken === path.label ? "Taken" : "Not taken";
            return (
              <section
                key={region}
                aria-label={region}
                data-taken={state === null ? undefined : state === "Taken"}
                className={cn(
                  "flex min-w-56 flex-1 flex-col gap-1 rounded-lg border border-dashed p-2",
                  state === "Taken" && "border-primary",
                )}
              >
                <h3 className="text-xs font-medium text-muted-foreground">
                  {title}
                  {state && ` · ${state}`}
                </h3>
                {path.steps.length === 0 ? (
                  <p className="text-xs text-muted-foreground">No steps: it goes on after.</p>
                ) : (
                  <RunSteps
                    steps={path.steps}
                    prefix={`${number}.`}
                    name={
                      path.label === null
                        ? "Steps of Otherwise"
                        : `Steps of the path ${path.label.trim()}`
                    }
                    view={view}
                  />
                )}
              </section>
            );
          })}
        </div>
      )}
    </div>
  );
}

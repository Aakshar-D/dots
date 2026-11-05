import type {
  AgentStep,
  Budget,
  Run,
  RunStatus,
  Step,
  StepRun,
  StepRunStatus,
  WorkflowRun,
} from "@/api/types";
import { formatTokens } from "@/lib/usage-format";

import { agentsBefore, findStep } from "../builder/draft";
import { DEFAULT_BUDGET } from "../format";

/// The newest step run of each step, by step id: what the run page shows on the step's card. A
/// run's step runs come oldest first, so a later attempt replaces an earlier one.
export function latestStepRuns(run: Pick<WorkflowRun, "steps">): Map<string, StepRun> {
  const latest = new Map<string, StepRun>();
  for (const stepRun of run.steps) latest.set(stepRun.step_id, stepRun);
  return latest;
}

/// A step's state as its card says it (section 10.3); a step with no step run is not reached.
export const STEP_STATUS_LABEL: Record<StepRunStatus | "not_reached", string> = {
  running: "Running",
  waiting: "Waiting",
  succeeded: "Done",
  failed: "Failed",
  cancelled: "Stopped",
  skipped: "Skipped",
  not_reached: "Not reached",
};

/// The path a branch step took, from its step run's decision: the label, null for Otherwise,
/// undefined while it has not decided.
export function pathTaken(stepRun: StepRun | undefined): string | null | undefined {
  const decision = stepRun?.decision;
  if (!decision || !("path" in decision)) return undefined;
  return decision.path;
}

/// The agent step a check sends the work back to: its own `send_back_to`, else the nearest agent
/// step before it on its path, as the core picks it.
export function sendBackTarget(steps: Step[], checkId: string): AgentStep | undefined {
  const check = findStep(steps, checkId);
  if (check?.kind === "check" && check.send_back_to) {
    const to = findStep(steps, check.send_back_to);
    return to?.kind === "agent" ? to : undefined;
  }
  return agentsBefore(steps, checkId).at(-1);
}

/// The name of the step `id` in the run's definition, or the id when it has none.
export function stepName(run: Pick<WorkflowRun, "definition">, id: string): string {
  return findStep(run.definition.steps, id)?.name.trim() || id;
}

/// One of the four limits of a workflow run (section 5.7): what it spent and its limit, and
/// what Add budget adds to it once it is reached (the core's `raised`: the default amount).
export type Limit = {
  key: keyof Budget;
  label: string;
  spent: number;
  limit: number;
  reached: boolean;
  /// "3 / 10", "120k / 1.5M".
  text: string;
  /// "10 runs", "1.5M tokens": what Add budget adds to it.
  more: string;
};

/// The run's four limits in the order the meters show them. Minutes are the run's active
/// minutes, whole ones; send-backs have no Add budget (Allow one more send-back raises them).
export function limits(run: WorkflowRun): Limit[] {
  const minutes = Math.floor(run.active_ms / 60_000);
  const b = run.budget;
  return [
    {
      key: "max_runs",
      label: "Runs",
      spent: run.spent_runs,
      limit: b.max_runs,
      reached: run.spent_runs >= b.max_runs,
      text: `${run.spent_runs} / ${b.max_runs}`,
      more: `${DEFAULT_BUDGET.max_runs} runs`,
    },
    {
      key: "max_tokens",
      label: "Tokens",
      spent: run.spent_tokens,
      limit: b.max_tokens,
      reached: run.spent_tokens >= b.max_tokens,
      text: `${formatTokens(run.spent_tokens)} / ${formatTokens(b.max_tokens)}`,
      more: `${formatTokens(DEFAULT_BUDGET.max_tokens)} tokens`,
    },
    {
      key: "max_minutes",
      label: "Minutes",
      spent: minutes,
      limit: b.max_minutes,
      // As the core counts it: the active time in milliseconds against the limit's.
      reached: run.active_ms >= b.max_minutes * 60_000,
      text: `${minutes} / ${b.max_minutes}`,
      more: `${DEFAULT_BUDGET.max_minutes} minutes`,
    },
    {
      key: "max_send_backs",
      label: "Send-backs",
      spent: run.send_backs,
      limit: b.max_send_backs,
      reached: run.send_backs >= b.max_send_backs,
      text: `${run.send_backs} / ${b.max_send_backs}`,
      more: "1 send-back",
    },
  ];
}

/// The limits Add budget raises: the reached ones among runs, tokens and minutes.
export function budgetReached(run: WorkflowRun): Limit[] {
  return limits(run).filter((l) => l.key !== "max_send_backs" && l.reached);
}

/// "a, b and c".
export function joinAnd(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}

/// A link from a step's card to one of its dot runs' run screens.
export type DotRunLink = { id: string; label: string; status: RunStatus | null };

/// The dot runs of a step run and the run each delegate member's branch is from. `runs` are the
/// newest dot runs (newest first): those of the step run are grouped by thread, one link per
/// thread, to its root. A hand-off branch whose run is not among them still gets its link, to
/// the run the branch names. Members' runs are linked from the members, not again from the step.
export function stepRunLinks(
  stepRun: StepRun,
  runs: Run[],
  nameOf: (dotId: string) => string,
): { step: DotRunLink[]; members: Map<string, string> } {
  const byId = new Map(runs.map((r) => [r.id, r]));
  const threads = new Map<string, DotRunLink>();
  for (const r of runs) {
    if (r.workflow_step_run_id !== stepRun.id || threads.has(r.root_run_id)) continue;
    threads.set(r.root_run_id, { id: r.root_run_id, label: nameOf(r.dot_id), status: r.status });
  }
  const linkOf = (runId: string) => byId.get(runId)?.root_run_id ?? runId;
  const handoff = stepRun.handoff;
  for (const ref of handoff?.branches ?? []) {
    const id = linkOf(ref.run_id);
    const known = byId.get(ref.run_id);
    if (threads.has(id)) continue;
    threads.set(id, {
      id,
      label: known ? nameOf(known.dot_id) : ref.branch,
      status: known?.status ?? null,
    });
  }
  const members = new Map<string, string>();
  for (const member of handoff?.members ?? []) {
    const ref = member.branch ? handoff?.branches.find((b) => b.branch === member.branch) : null;
    if (ref) members.set(member.branch!, linkOf(ref.run_id));
  }
  const memberRuns = new Set(members.values());
  return { step: [...threads.values()].filter((l) => !memberRuns.has(l.id)), members };
}

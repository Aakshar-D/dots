import type {
  Budget,
  Team,
  TeamSpec,
  Trigger,
  WaitingOn,
  WorkflowRun,
  WorkflowRunStatus,
  WorkflowSpec,
} from "@/api/types";
import { formatTokens } from "@/lib/usage-format";

/// What starts a workflow, by its trigger's kind (teams spec section 5.1).
export const TRIGGER_LABEL: Record<Trigger["kind"], string> = {
  manual: "Run by hand",
  chat: "Started from a chat",
  schedule: "On a schedule",
  webhook: "On a webhook",
};

/// What a waiting workflow run waits for, in the needs-you wording (section 10.5).
export const WAITING_LABEL: Record<WaitingOn, string> = {
  check: "Needs your check",
  budget: "Hit its budget",
  send_backs: "Hit its send-back limit",
  step_failed: "A step failed",
};

const STATUS_LABEL: Record<WorkflowRunStatus, string> = {
  running: "Running",
  waiting: "Waiting",
  succeeded: "Succeeded",
  failed: "Failed",
  cancelled: "Cancelled",
};

/// A workflow run's status as a person reads it: a waiting run says what it waits for when
/// that is known (a workflow's `last_run` does not say).
export function workflowRunStatusLabel(run: {
  status: WorkflowRunStatus;
  waiting_on?: WaitingOn | null;
}): string {
  if (run.status === "waiting" && run.waiting_on) return WAITING_LABEL[run.waiting_on];
  return STATUS_LABEL[run.status];
}

/// The first line of a task, trimmed: how lists, cards and titles name a workflow run.
export function taskLine(task: string): string {
  return (task.trim().split(/\r?\n/, 1)[0] ?? "").trim();
}

/// "3 runs · 120k tokens": what a workflow run spent so far (section 10.4).
export function formatSpend(run: Pick<WorkflowRun, "spent_runs" | "spent_tokens">): string {
  const runs = `${run.spent_runs} ${run.spent_runs === 1 ? "run" : "runs"}`;
  return `${runs} · ${formatTokens(run.spent_tokens)} tokens`;
}

/// The team as `team_update` takes it, with `change` applied: its members keep their roles,
/// and its account and workspace stay.
export function teamSpec(team: Team, change: Partial<TeamSpec> = {}): TeamSpec {
  return {
    name: team.name,
    description: team.description,
    members: team.members.map((m) => ({ dot_id: m.dot_id, role: m.role })),
    org_id: team.org_id,
    workspace_id: team.workspace_id,
    ...change,
  };
}

/// The limits a new workflow starts with (section 5.7), the core's defaults.
export const DEFAULT_BUDGET: Budget = {
  max_runs: 10,
  max_tokens: 1_500_000,
  max_minutes: 120,
  max_send_backs: 3,
};

/// A new workflow named `name`: run by hand, with one agent step that `member` does, and off
/// until the person has built it and turns it on. A save needs a step, and a step a member.
export function newWorkflowSpec(name: string, member: string): WorkflowSpec {
  return {
    name,
    enabled: false,
    definition: {
      trigger: { kind: "manual" },
      steps: [
        {
          id: "work",
          name: "Work",
          kind: "agent",
          member,
          job: "Work on the task.",
          can_send_back_to: [],
          outcomes: [],
          delegate: null,
        },
      ],
      budget: DEFAULT_BUDGET,
    },
  };
}

/// `base`, else `base 2`, `base 3`, …: the first name none of `taken` has, without case.
export function uniqueName(base: string, taken: string[]): string {
  const used = new Set(taken.map((name) => name.trim().toLowerCase()));
  if (!used.has(base.toLowerCase())) return base;
  let n = 2;
  while (used.has(`${base} ${n}`.toLowerCase())) n += 1;
  return `${base} ${n}`;
}

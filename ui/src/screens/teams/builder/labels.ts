import type {
  AgentStep,
  BranchOn,
  DeployMode,
  DeployStep,
  MergeMethod,
  Step,
  Trigger,
} from "@/api/types";

import { taskLine, TRIGGER_LABEL } from "../format";
import { findStep, type StepKindName } from "./draft";

/// A step's kind as the Add step menu and the cards name it.
export const KIND_LABEL: Record<StepKindName, string> = {
  agent: "Agent step",
  check: "Check",
  deploy: "Deploy",
  branch: "Branch",
};

/// What a deploy of each mode does with the approved branches (section 5.5).
export const DEPLOY_LABEL: Record<DeployMode, string> = {
  branch: "Keeps the branches: nothing is pushed",
  push: "Pushes the branches",
  pr: "Pushes the branches and opens a pull request",
  pr_merge: "Pushes, opens a pull request and merges it",
  member: "A member deploys",
};

/// The deploy modes as the settings sheet offers them, in its order.
export const MODE_OPTIONS: [DeployMode, string][] = [
  ["branch", "Keep the branches"],
  ["push", "Push"],
  ["pr", "Push and open a pull request"],
  ["pr_merge", "Push, open a pull request and merge it"],
  ["member", "A member deploys"],
];

/// How `gh pr merge` merges, as the settings sheet offers it.
export const MERGE_OPTIONS: [MergeMethod, string][] = [
  ["squash", "Squash"],
  ["merge", "Merge commit"],
  ["rebase", "Rebase"],
];

/// What a branch step goes by, as the settings sheet offers it.
export const ON_OPTIONS: [BranchOn, string][] = [
  ["outcome", "The outcome reported before it"],
  ["project", "The project"],
];

/// A team member's name by dot id; an id that is no member shows as itself.
export type NameOf = (dotId: string) => string;

/// The trigger card's lines: what starts the workflow, then its detail.
export function triggerSummary(trigger: Trigger, nameOf: NameOf): [string, string] {
  const title = TRIGGER_LABEL[trigger.kind];
  switch (trigger.kind) {
    case "manual":
      return [title, "Run now on the team page, or Test run here"];
    case "chat":
      return [
        title,
        trigger.starters.length > 0
          ? `When you chat with ${trigger.starters.map(nameOf).join(", ")}`
          : "No member starts it yet",
      ];
    case "schedule":
      return [
        title,
        `${taskLine(trigger.task) || "No task yet"} · on ${trigger.project || "no project yet"}`,
      ];
    case "webhook":
      return [
        title,
        trigger.task
          ? `${taskLine(trigger.task)}, unless the call says the task`
          : "The call says the task",
      ];
  }
}

/// "may start Dev agent · 3 at once · 6 total": what a delegate step may start (section 10.2).
export function delegateLine(step: AgentStep, nameOf: NameOf): string | undefined {
  const d = step.delegate;
  if (!d) return undefined;
  const who = d.may_start.length > 0 ? d.may_start.map(nameOf).join(", ") : "no member yet";
  return `may start ${who} · ${d.at_once} at once · ${d.total} total`;
}

/// What a deploy does, with its mode's own field: the merge method, or the member and job.
export function deployLine(step: DeployStep, nameOf: NameOf): string {
  switch (step.mode) {
    case "pr_merge":
      return `${DEPLOY_LABEL.pr_merge} (${step.merge_method})`;
    case "member":
      return `${nameOf(step.member)} deploys: ${taskLine(step.job)}`;
    default:
      return DEPLOY_LABEL[step.mode];
  }
}

/// The card's line under the step's name, by its kind. `steps` is the whole draft, for the
/// name of the step a check sends back to.
export function stepLine(step: Step, steps: Step[], nameOf: NameOf): string {
  switch (step.kind) {
    case "agent":
      return `${nameOf(step.member)} · ${taskLine(step.job)}`;
    case "check": {
      const to = step.send_back_to ? findStep(steps, step.send_back_to) : undefined;
      return to
        ? `Approve, or send back to ${to.name} with a note`
        : "Approve, or send back with a note";
    }
    case "deploy":
      return deployLine(step, nameOf);
    case "branch":
      return step.on === "outcome"
        ? "Goes on by the outcome reported before it"
        : "Goes on by the project";
  }
}

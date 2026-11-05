import type {
  AgentStep,
  BranchStep,
  DeployMode,
  DeployStep,
  Delegate,
  Schedule,
  Step,
  Team,
  Trigger,
} from "@/api/types";

/// The core's limits on what a step holds (`workflow.rs`), in characters: a step's name, an
/// agent's or a deploying member's job, an outcome label, and a trigger's task.
export const MAX_NAME = 80;
export const MAX_JOB = 8_000;
export const MAX_OUTCOME = 40;
export const MAX_TASK = 8_000;

/// What a step becomes a delegate with (section 5.3): no member yet, 3 at once, 6 in all.
export const NEW_DELEGATE: Delegate = { may_start: [], at_once: 3, total: 6 };

/// The kinds of step the builder adds (teams spec section 5): a delegate is an agent step whose
/// settings say what it may start.
export type StepKindName = Step["kind"];
export const STEP_KINDS: StepKindName[] = ["agent", "check", "deploy", "branch"];

/// One list of steps in a definition: the top list, or one of a branch step's lists (a path by
/// its place, or `otherwise`).
export type ListRef = { branch: null } | { branch: string; path: number | "otherwise" };

/// The top list of steps.
export const TOP: ListRef = { branch: null };

/// Every step of `steps`, depth first as the core walks them: a branch, then its paths in order,
/// then `otherwise`.
export function allSteps(steps: Step[]): Step[] {
  return steps.flatMap((step) => [step, ...childLists(step).flatMap(allSteps)]);
}

/// The lists inside `step`: a branch's paths, then `otherwise`; none for the other kinds.
export function childLists(step: Step): Step[][] {
  if (step.kind !== "branch") return [];
  return [...step.paths.map((p) => p.steps), step.otherwise];
}

/// The step with the id `id`, at any depth.
export function findStep(steps: Step[], id: string): Step | undefined {
  return allSteps(steps).find((step) => step.id === id);
}

/// A new step id for `kind`: `<kind>-<n>`, the smallest n that no step of `steps` has. The
/// builder makes it once, when the step is added, and never changes it, so a step keeps its
/// id through moves, renames and saves (the core's rule: 1 to 40 of a-z, 0-9, '-' and '_').
export function nextStepId(kind: StepKindName, steps: Step[]): string {
  const taken = new Set(allSteps(steps).map((step) => step.id));
  let n = 1;
  while (taken.has(`${kind}-${n}`)) n += 1;
  return `${kind}-${n}`;
}

/// A new step of `kind` for `team`'s workflow whose steps are `steps`, with defaults that a save
/// takes where it can: an agent step done by the first member; a check that sends back to the
/// step before it; a deploy that keeps the branches and pushes nothing; a branch on the project
/// with a path per project of the team.
export function newStep(
  kind: StepKindName,
  steps: Step[],
  team: Pick<Team, "members" | "projects">,
): Step {
  const id = nextStepId(kind, steps);
  switch (kind) {
    case "agent":
      return {
        id,
        name: "Work",
        kind,
        member: team.members[0]?.dot_id ?? "",
        job: "Work on the task.",
        can_send_back_to: [],
        outcomes: [],
        delegate: null,
      };
    case "check":
      return { id, name: "Your check", kind, send_back_to: null };
    case "deploy":
      return { id, name: "Deploy", kind, mode: "branch" };
    case "branch": {
      const labels = team.projects.map((p) => p.name);
      return {
        id,
        name: "Branch",
        kind,
        on: "project",
        paths: (labels.length > 0 ? labels : ["Path 1"]).map((label) => ({ label, steps: [] })),
        otherwise: [],
      };
    }
  }
}

/// `steps` with the list `ref` replaced by `change(list)`. A ref to a branch or path that is not
/// there changes nothing.
export function editList(steps: Step[], ref: ListRef, change: (list: Step[]) => Step[]): Step[] {
  if (ref.branch === null) return change(steps);
  const { branch, path } = ref;
  return updateStep(steps, branch, (step) => {
    if (step.kind !== "branch") return step;
    if (path === "otherwise") return { ...step, otherwise: change(step.otherwise) };
    if (!step.paths[path]) return step;
    const paths = step.paths.map((p, i) => (i === path ? { ...p, steps: change(p.steps) } : p));
    return { ...step, paths } satisfies BranchStep;
  });
}

/// `steps` with `step` put into the list `ref` at `index` (its end when past it).
export function insertStep(steps: Step[], ref: ListRef, index: number, step: Step): Step[] {
  return editList(steps, ref, (list) => [...list.slice(0, index), step, ...list.slice(index)]);
}

/// `steps` with the step `id`, at any depth, replaced by `change(step)`.
export function updateStep(steps: Step[], id: string, change: (step: Step) => Step): Step[] {
  return steps.map((step) => {
    if (step.id === id) return change(step);
    if (step.kind !== "branch") return step;
    return {
      ...step,
      paths: step.paths.map((p) => ({ ...p, steps: updateStep(p.steps, id, change) })),
      otherwise: updateStep(step.otherwise, id, change),
    };
  });
}

/// Where the step `id` is: its list and its place in it; undefined when no step has the id.
export function locateStep(
  steps: Step[],
  id: string,
  ref: ListRef = TOP,
): { ref: ListRef; index: number; length: number } | undefined {
  for (const [index, step] of steps.entries()) {
    if (step.id === id) return { ref, index, length: steps.length };
    if (step.kind !== "branch") continue;
    const inner = [
      ...step.paths.map((p, path) => locateStep(p.steps, id, { branch: step.id, path })),
      locateStep(step.otherwise, id, { branch: step.id, path: "otherwise" }),
    ].find((found) => found !== undefined);
    if (inner) return inner;
  }
  return undefined;
}

/// `steps` without the step `id` (and, for a branch, the steps inside it).
export function deleteStep(steps: Step[], id: string): Step[] {
  const at = locateStep(steps, id);
  if (!at) return steps;
  return editList(steps, at.ref, (list) => list.filter((step) => step.id !== id));
}

/// Whether the step `id` can move `by` places (-1 up, 1 down) within its own list.
export function canMove(steps: Step[], id: string, by: -1 | 1): boolean {
  const at = locateStep(steps, id);
  if (!at) return false;
  const to = at.index + by;
  return to >= 0 && to < at.length;
}

/// `steps` with the step `id` moved `by` places (-1 up, 1 down) within its own list: a step
/// stays on its path. At either end of its list it stays where it is.
export function moveStep(steps: Step[], id: string, by: -1 | 1): Step[] {
  const at = locateStep(steps, id);
  if (!at || !canMove(steps, id, by)) return steps;
  return editList(steps, at.ref, (list) => {
    const next = [...list];
    const [step] = next.splice(at.index, 1);
    next.splice(at.index + by, 0, step!);
    return next;
  });
}

/// The way down to the step `id`: for each level from the top, the list it is in and its place
/// there. The last entry is the step itself, the ones before it its enclosing branches.
function trail(steps: Step[], id: string): [Step[], number][] | undefined {
  for (const [i, step] of steps.entries()) {
    if (step.id === id) return [[steps, i]];
    for (const list of childLists(step)) {
      const inner = trail(list, id);
      if (inner) return [[steps, i], ...inner];
    }
  }
  return undefined;
}

/// The steps that run before the step `id` on its path, in run order, as the core's
/// `Definition::before` finds them: the earlier steps of each enclosing list and the enclosing
/// branches themselves. Empty when no step has the id.
export function stepsBefore(steps: Step[], id: string): Step[] {
  const way = trail(steps, id);
  if (!way) return [];
  const last = way.length - 1;
  return way.flatMap(([list, i], level) => list.slice(0, level < last ? i + 1 : i));
}

/// The agent steps before the step `id` on its path: what it may send work back to.
export function agentsBefore(steps: Step[], id: string): AgentStep[] {
  return stepsBefore(steps, id).filter((step): step is AgentStep => step.kind === "agent");
}

/// The agent step whose outcome a branch on outcome `id` goes by: the nearest one before it.
export function outcomeSource(steps: Step[], id: string): AgentStep | undefined {
  return agentsBefore(steps, id).at(-1);
}

/// `step` in deploy mode `mode`, keeping its id and name. A deploy holds only its mode's
/// fields, so a new mode starts from its defaults: squash (section 5.5's default), or the
/// team's first member with a plain job.
export function withMode(
  step: DeployStep,
  mode: DeployMode,
  team: Pick<Team, "members">,
): DeployStep {
  if (step.mode === mode) return step;
  const base = { id: step.id, name: step.name, kind: step.kind };
  switch (mode) {
    case "pr_merge":
      return { ...base, mode, merge_method: "squash" };
    case "member":
      return {
        ...base,
        mode,
        member: team.members[0]?.dot_id ?? "",
        job: "Deploy what was approved.",
      };
    default:
      return { ...base, mode };
  }
}

/// `trigger` as a trigger of `kind`. A task and project typed for a schedule carry over to a
/// webhook's fallbacks and back; a new chat trigger is started by the team's first member; a
/// new schedule is `schedule` (daily) on the team's first project.
export function withTriggerKind(
  trigger: Trigger,
  kind: Trigger["kind"],
  team: Pick<Team, "members" | "projects">,
  schedule: Schedule,
): Trigger {
  if (trigger.kind === kind) return trigger;
  const task =
    trigger.kind === "schedule" ? trigger.task : trigger.kind === "webhook" ? trigger.task : null;
  const project =
    trigger.kind === "schedule"
      ? trigger.project
      : trigger.kind === "webhook"
        ? trigger.project
        : null;
  switch (kind) {
    case "manual":
      return { kind };
    case "chat":
      return { kind, starters: team.members.slice(0, 1).map((m) => m.dot_id) };
    case "schedule":
      return {
        kind,
        schedule,
        task: task ?? "",
        project: project || (team.projects[0]?.name ?? ""),
      };
    case "webhook":
      return { kind, task: task?.trim() ? task : null, project: project || null };
  }
}

/// The label of a new path of `step`: the first of `suggestions` (the outcomes or projects it
/// can go by) that no path has, else "Path <n>" with the smallest free n. Labels are compared
/// without case, as the core compares them.
export function newPathLabel(step: BranchStep, suggestions: string[]): string {
  const taken = new Set(step.paths.map((p) => p.label.trim().toLowerCase()));
  const free = suggestions.find((s) => s.trim() !== "" && !taken.has(s.trim().toLowerCase()));
  if (free) return free.trim();
  let n = 1;
  while (taken.has(`path ${n}`)) n += 1;
  return `Path ${n}`;
}

/// `list` with `item` added at its end when `on` (once), or taken out when not.
export function toggled(list: string[], item: string, on: boolean): string[] {
  if (!on) return list.filter((x) => x !== item);
  return list.includes(item) ? list : [...list, item];
}

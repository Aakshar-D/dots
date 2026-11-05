import type { BranchStep, Step, Team } from "@/api/types";

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

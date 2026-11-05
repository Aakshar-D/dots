import { describe, expect, it } from "vitest";

import { Step, type Team, type Workflow } from "@/api/types";
import { sample } from "@/test/fixtures";

import {
  allSteps,
  canMove,
  deleteStep,
  findStep,
  insertStep,
  locateStep,
  moveStep,
  newStep,
  nextStepId,
  TOP,
  updateStep,
} from "./draft";

const [crew] = sample.teams as [Team, Team];
const [fix, , nightly] = sample.workflows as [Workflow, Workflow, Workflow, Workflow];
// Build; Fixed? (path fixed: Review, Ship; otherwise: Design).
const fixSteps = fix.definition.steps;
// Work; Review; Which project (path Dots: pr; path Site: keep).
const nightlySteps = nightly.definition.steps;
const ids = (steps: Step[]) => allSteps(steps).map((s) => s.id);

describe("the builder's draft", () => {
  it("walks the steps depth first, as the core does, and finds one at any depth", () => {
    expect(ids(fixSteps)).toEqual(["build", "route", "review", "ship", "design"]);
    expect(findStep(fixSteps, "design")?.name).toBe("Design");
    expect(findStep(fixSteps, "nope")).toBeUndefined();
    expect(locateStep(fixSteps, "ship")).toEqual({
      ref: { branch: "route", path: 0 },
      index: 1,
      length: 2,
    });
    expect(locateStep(fixSteps, "design")).toEqual({
      ref: { branch: "route", path: "otherwise" },
      index: 0,
      length: 1,
    });
    expect(locateStep(fixSteps, "route")).toEqual({ ref: TOP, index: 1, length: 2 });
  });

  it("makes a step's id from its kind, one no step of the whole tree has", () => {
    expect(nextStepId("agent", fixSteps)).toBe("agent-1");
    const taken = insertStep(fixSteps, { branch: "route", path: 0 }, 0, {
      ...newStep("check", fixSteps, crew),
    });
    expect(ids(taken)).toContain("check-1");
    expect(nextStepId("check", taken)).toBe("check-2");
    // The id fits the core's rule: 1 to 40 of a-z, 0-9, '-' and '_'.
    expect(nextStepId("deploy", taken)).toMatch(/^[a-z0-9_-]{1,40}$/);
  });

  it("gives each kind of new step defaults a save takes", () => {
    const agent = newStep("agent", nightlySteps, crew);
    expect(agent).toEqual({
      id: "agent-1",
      name: "Work",
      kind: "agent",
      member: "01J0DOT",
      job: "Work on the task.",
      can_send_back_to: [],
      outcomes: [],
      delegate: null,
    });
    expect(newStep("check", nightlySteps, crew)).toEqual({
      id: "check-1",
      name: "Your check",
      kind: "check",
      send_back_to: null,
    });
    // A new deploy pushes nothing until its mode is chosen.
    expect(newStep("deploy", nightlySteps, crew)).toEqual({
      id: "deploy-1",
      name: "Deploy",
      kind: "deploy",
      mode: "branch",
    });
    expect(newStep("branch", nightlySteps, crew)).toEqual({
      id: "branch-1",
      name: "Branch",
      kind: "branch",
      on: "project",
      paths: [
        { label: "Dots", steps: [] },
        { label: "Site", steps: [] },
      ],
      otherwise: [],
    });
    const bare = newStep("branch", [], { members: [], projects: [] });
    expect(bare.kind === "branch" && bare.paths).toEqual([{ label: "Path 1", steps: [] }]);
    for (const kind of ["agent", "check", "deploy", "branch"] as const) {
      expect(Step.safeParse(newStep(kind, nightlySteps, crew)).success, kind).toBe(true);
    }
  });

  it("puts a new step into any list at its place", () => {
    const step = newStep("check", fixSteps, crew);
    expect(ids(insertStep(fixSteps, TOP, 0, step))).toEqual([
      "check-1",
      "build",
      "route",
      "review",
      "ship",
      "design",
    ]);
    expect(ids(insertStep(fixSteps, TOP, 2, step)).at(-1)).toBe("check-1");
    expect(ids(insertStep(fixSteps, { branch: "route", path: 0 }, 1, step))).toEqual([
      "build",
      "route",
      "review",
      "check-1",
      "ship",
      "design",
    ]);
    expect(ids(insertStep(fixSteps, { branch: "route", path: "otherwise" }, 1, step))).toEqual([
      "build",
      "route",
      "review",
      "ship",
      "design",
      "check-1",
    ]);
    // A list that is not there changes nothing.
    expect(insertStep(fixSteps, { branch: "route", path: 5 }, 0, step)).toEqual(fixSteps);
    expect(insertStep(fixSteps, { branch: "build", path: 0 }, 0, step)).toEqual(fixSteps);
  });

  it("changes and deletes a step by id at any depth, leaving the rest as it was", () => {
    const renamed = updateStep(fixSteps, "ship", (s) => ({ ...s, name: "Merge" }));
    expect(findStep(renamed, "ship")?.name).toBe("Merge");
    expect(allSteps(renamed).filter((s) => s.id !== "ship" && s.kind !== "branch")).toEqual(
      allSteps(fixSteps).filter((s) => s.id !== "ship" && s.kind !== "branch"),
    );
    expect(fixSteps).toEqual(fix.definition.steps);

    expect(ids(deleteStep(fixSteps, "review"))).toEqual(["build", "route", "ship", "design"]);
    expect(ids(deleteStep(fixSteps, "design"))).toEqual(["build", "route", "review", "ship"]);
    // A branch goes with the steps inside it.
    expect(ids(deleteStep(fixSteps, "route"))).toEqual(["build"]);
    expect(deleteStep(fixSteps, "nope")).toEqual(fixSteps);
  });

  it("moves a step up or down within its own list, and not past either end", () => {
    expect(ids(moveStep(nightlySteps, "work", 1))).toEqual([
      "check",
      "work",
      "by-project",
      "pr",
      "keep",
    ]);
    expect(ids(moveStep(nightlySteps, "by-project", -1)).slice(0, 3)).toEqual([
      "work",
      "by-project",
      "pr",
    ]);
    expect(canMove(nightlySteps, "work", -1)).toBe(false);
    expect(moveStep(nightlySteps, "work", -1)).toEqual(nightlySteps);
    expect(canMove(nightlySteps, "by-project", 1)).toBe(false);

    // Inside a path: the step stays on it.
    expect(ids(moveStep(fixSteps, "ship", -1))).toEqual([
      "build",
      "route",
      "ship",
      "review",
      "design",
    ]);
    expect(canMove(fixSteps, "ship", 1)).toBe(false);
    expect(canMove(fixSteps, "design", -1)).toBe(false);
    expect(canMove(fixSteps, "nope", 1)).toBe(false);
  });
});

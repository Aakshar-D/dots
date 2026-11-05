import { describe, expect, it } from "vitest";

import type { Run, StepRun, WorkflowRun } from "@/api/types";
import { sample } from "@/test/fixtures";

import {
  budgetReached,
  cardSteps,
  joinAnd,
  latestStepRuns,
  limits,
  pathTaken,
  sendBackTarget,
  stepName,
  stepRunLinks,
} from "./steps";

const [waiting, done, held, , budget] = sample.workflowRuns as [
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
  WorkflowRun,
];
const names: Record<string, string> = { "01J0DOT": "nightly-tests", "01J0DOT2": "reviewer" };
const nameOf = (id: string) => names[id] ?? id;
const run = (id: string, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  root_run_id: id,
  ...over,
});

describe("the run page's steps", () => {
  it("shows each step's newest step run, a later attempt over an earlier one", () => {
    const latest = latestStepRuns(done);
    expect(latest.get("build")?.id).toBe("01J0STP9");
    expect(latest.get("build")?.attempt).toBe(2);
    expect(latest.get("review")?.id).toBe("01J0STP4");
    expect(latest.get("ship")?.id).toBe("01J0STP5");
    expect(latest.has("design")).toBe(false);
  });

  it("reads the path a branch took, Otherwise as null, and nothing before it decided", () => {
    const route = latestStepRuns(waiting).get("route");
    expect(pathTaken(route)).toBe("fixed");
    expect(pathTaken({ ...route!, decision: { path: null } })).toBeNull();
    expect(pathTaken(latestStepRuns(waiting).get("review"))).toBeUndefined();
    expect(pathTaken(undefined)).toBeUndefined();
  });

  it("sends a check back to its own target, else to the nearest agent step before it", () => {
    expect(sendBackTarget(waiting.definition.steps, "review")?.name).toBe("Build");
    expect(sendBackTarget(budget.definition.steps, "check")?.name).toBe("Work");
    expect(sendBackTarget(waiting.definition.steps, "nope")).toBeUndefined();
    expect(stepName(waiting, "design")).toBe("Design");
    expect(stepName(waiting, "gone")).toBe("gone");
  });

  it("says what each limit spent of its limit, which are reached, and what Add budget adds", () => {
    expect(limits(waiting).map((l) => [l.label, l.text, l.reached])).toEqual([
      ["Runs", "3 / 10", false],
      ["Tokens", "120k / 1.5M", false],
      ["Minutes", "5 / 120", false],
      ["Send-backs", "0 / 3", false],
    ]);
    expect(budgetReached(budget).map((l) => [l.text, l.more])).toEqual([["3 / 3", "10 runs"]]);
    const spent: WorkflowRun = {
      ...budget,
      spent_tokens: 1_500_000,
      active_ms: 120 * 60_000,
      send_backs: 3,
    };
    expect(budgetReached(spent).map((l) => l.more)).toEqual([
      "10 runs",
      "1.5M tokens",
      "120 minutes",
    ]);
    expect(joinAnd(["10 runs", "1.5M tokens", "120 minutes"])).toBe(
      "10 runs, 1.5M tokens and 120 minutes",
    );
    expect(joinAnd(["10 runs"])).toBe("10 runs");
  });

  it("links a step to its dot runs by thread, and each member to the run of its branch", () => {
    const build = latestStepRuns(waiting).get("build") as StepRun;
    const runs = [
      run("01J0RUN7", { dot_id: "01J0DOT", root_run_id: "01J0RUN5", status: "succeeded" }),
      run("01J0RUN5", { dot_id: "01J0DOT", workflow_step_run_id: build.id }),
      run("01J0RUN6", { dot_id: "01J0DOT2", workflow_step_run_id: build.id }),
      // Another step run's.
      run("01J0RUN8", { workflow_step_run_id: "01J0STP2" }),
    ];
    const links = stepRunLinks(build, runs, nameOf);
    // The lead's thread is its root, once; the member's run is linked from the member.
    expect(links.step).toEqual([{ id: "01J0RUN5", label: "nightly-tests", status: "succeeded" }]);
    expect([...links.members]).toEqual([["dots/reviewer/01J0RUN6", "01J0RUN6"]]);

    // Runs that fell out of the newest still link from the hand-off, named by their branch.
    const old = stepRunLinks(build, [], nameOf);
    expect(old.step).toEqual([
      { id: "01J0RUN5", label: "dots/nightly-tests/01J0RUN5", status: null },
    ]);
    expect(stepRunLinks(latestStepRuns(waiting).get("review")!, runs, nameOf).step).toEqual([]);
  });

  it("lines up the chat card's steps along the path taken, else the path whose step ran", () => {
    const line = (run: Pick<WorkflowRun, "definition" | "steps">) =>
      cardSteps(run).map(({ id, status, label }) => [id, status, label]);
    expect(line(waiting)).toEqual([
      ["build", "succeeded", "Done"],
      ["review", "waiting", "Waiting for you"],
      ["ship", "not_reached", "Not reached"],
    ]);
    // The held send-back waits in Otherwise, which no decision of the branch names yet.
    expect(line(held)).toEqual([
      ["build", "not_reached", "Not reached"],
      ["design", "waiting", "Waiting"],
    ]);
    // Before anything ran, the branch stands for its paths.
    expect(line({ ...waiting, steps: [] })).toEqual([
      ["build", "not_reached", "Not reached"],
      ["route", "not_reached", "Not reached"],
    ]);
    const otherwise = { ...waiting.steps[1]!, decision: { path: null } };
    expect(line({ ...waiting, steps: [waiting.steps[0]!, otherwise] }).map(([id]) => id)).toEqual(
      ["build", "design"],
    );
    expect(cardSteps(waiting).map((s) => s.name)).toEqual(["Build", "Review", "Ship"]);
  });
});
